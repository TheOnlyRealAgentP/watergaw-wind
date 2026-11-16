//! GRIB2 decoding for NOAA model output.
//!
//! Supports what NOAA's operational models publish for surface wind and most
//! other fields: regular latitude/longitude grids (template 3.0), and simple
//! (5.0), complex (5.2), complex with spatial differencing (5.3) and PNG (5.41)
//! packing, with or without a bitmap.

use crate::Error;

/// A decoded GRIB2 field on a regular latitude/longitude grid.
#[derive(Debug, Clone)]
pub struct Field {
    pub discipline: u8,
    pub category: u8,
    pub number: u8,
    /// Model reference (cycle) time, Unix seconds.
    pub ref_time: f64,
    /// Forecast lead in hours (0 for analyses).
    pub lead_hours: f64,
    pub ni: usize,
    pub nj: usize,
    /// First grid point latitude/longitude (longitude in -180..360 as stored).
    pub la1: f64,
    pub lo1: f64,
    pub di: f64,
    pub dj: f64,
    /// Rows run south to north when true (scanning-mode bit 0x40).
    pub south_first: bool,
    /// Row-major values as stored (row 0 is the first scanned row). NaN = missing.
    pub values: Vec<f32>,
}

impl Field {
    pub fn lat(&self, j: usize) -> f64 {
        if self.south_first { self.la1 + j as f64 * self.dj } else { self.la1 - j as f64 * self.dj }
    }

    /// Bilinear value at a point; handles global grids that wrap in longitude.
    pub fn sample(&self, lat: f64, lon: f64) -> f32 {
        let fj = if self.south_first { (lat - self.la1) / self.dj } else { (self.la1 - lat) / self.dj };
        if fj < 0.0 || fj > (self.nj - 1) as f64 {
            return f32::NAN;
        }
        let global = (self.ni as f64 * self.di - 360.0).abs() < self.di * 1.5;
        let mut fi = (lon - self.lo1) / self.di;
        if global {
            fi = fi.rem_euclid(self.ni as f64);
        } else if fi < 0.0 || fi > (self.ni - 1) as f64 {
            return f32::NAN;
        }
        let (i0, j0) = (fi.floor() as usize, fj.floor() as usize);
        let i1 = if global { (i0 + 1) % self.ni } else { (i0 + 1).min(self.ni - 1) };
        let j1 = (j0 + 1).min(self.nj - 1);
        let (tx, ty) = ((fi - fi.floor()) as f32, (fj - fj.floor()) as f32);
        let g = |i: usize, j: usize| self.values[j * self.ni + i];
        let a = g(i0, j0) * (1.0 - tx) + g(i1, j0) * tx;
        let b = g(i0, j1) * (1.0 - tx) + g(i1, j1) * tx;
        a * (1.0 - ty) + b * ty
    }
}

/// MSB-first bit reader.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize, // in bits
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        if self.pos + n as usize > self.data.len() * 8 {
            return Err(Error::Truncated("packed data"));
        }
        let mut v: u64 = 0;
        let mut left = n;
        while left > 0 {
            let byte = self.data[self.pos / 8];
            let bit_off = (self.pos % 8) as u32;
            let take = (8 - bit_off).min(left);
            let chunk = (byte >> (8 - bit_off - take)) & ((1u16 << take) - 1) as u8;
            v = (v << take) | chunk as u64;
            self.pos += take as usize;
            left -= take;
        }
        Ok(v as u32)
    }

    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
/// GRIB's sign-and-magnitude integers.
fn sm16(v: u16) -> i32 {
    if v & 0x8000 != 0 { -((v & 0x7fff) as i32) } else { v as i32 }
}
fn sm32(v: u32) -> i64 {
    if v & 0x8000_0000 != 0 { -((v & 0x7fff_ffff) as i64) } else { v as i64 }
}
fn sm_n(b: &[u8]) -> i64 {
    let mut v: u64 = 0;
    for &x in b {
        v = (v << 8) | x as u64;
    }
    let bits = b.len() * 8;
    let sign = 1u64 << (bits - 1);
    if v & sign != 0 { -((v & (sign - 1)) as i64) } else { v as i64 }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

struct Rep<'a> {
    template: u16,
    r: f64,
    e: i32,
    d: i32,
    bits: u8,
    sec5: &'a [u8],
}

/// Decode every message in a buffer (a byte-range download can hold several).
pub fn decode_all(data: &[u8]) -> Result<Vec<Field>, Error> {
    let mut out = Vec::new();
    let mut off = 0;
    while off + 16 <= data.len() {
        if &data[off..off + 4] != b"GRIB" {
            off += 1;
            continue;
        }
        if data[off + 7] != 2 {
            return Err(Error::Format("only GRIB edition 2 is supported"));
        }
        let len = u64::from_be_bytes(data[off + 8..off + 16].try_into().unwrap()) as usize;
        if len < 16 || off + len > data.len() {
            return Err(Error::Truncated("GRIB2 message"));
        }
        out.push(decode_message(&data[off..off + len])?);
        off += len;
    }
    if out.is_empty() {
        return Err(Error::Format("no GRIB2 message found"));
    }
    Ok(out)
}

fn decode_message(m: &[u8]) -> Result<Field, Error> {
    let mut f = Field {
        discipline: m[6], category: 0, number: 0, ref_time: 0.0, lead_hours: 0.0,
        ni: 0, nj: 0, la1: 0.0, lo1: 0.0, di: 0.0, dj: 0.0, south_first: false, values: Vec::new(),
    };
    let mut rep: Option<Rep> = None;
    let mut bitmap: Option<&[u8]> = None;
    let mut data7: Option<&[u8]> = None;
    let mut npoints = 0usize;
    let mut off = 16;
    while off + 5 <= m.len() {
        if &m[off..off + 4] == b"7777" {
            break;
        }
        let len = be32(m, off) as usize;
        let sec = m[off + 4];
        if len < 5 || off + len > m.len() {
            return Err(Error::Truncated("GRIB2 section"));
        }
        let s = &m[off..off + len];
        match sec {
            1 => {
                let y = be16(s, 12) as i64;
                f.ref_time = (days_from_civil(y, s[14] as i64, s[15] as i64) * 86400 + s[16] as i64 * 3600 + s[17] as i64 * 60 + s[18] as i64) as f64;
            }
            3 => {
                if be16(s, 12) != 0 {
                    return Err(Error::Format("only regular lat/lon grids (template 3.0) are supported"));
                }
                f.ni = be32(s, 30) as usize;
                f.nj = be32(s, 34) as usize;
                f.la1 = sm32(be32(s, 46)) as f64 * 1e-6;
                f.lo1 = sm32(be32(s, 50)) as f64 * 1e-6;
                f.di = be32(s, 63) as f64 * 1e-6;
                f.dj = be32(s, 67) as f64 * 1e-6;
                f.south_first = s[71] & 0x40 != 0;
            }
            4 => {
                f.category = s[9];
                f.number = s[10];
                // Template 4.0/4.8: time unit (octet 18) and forecast time (19-22).
                if s.len() >= 22 {
                    let unit = s[17];
                    let t = be32(s, 18) as f64;
                    f.lead_hours = match unit {
                        0 => t / 60.0,
                        1 => t,
                        2 => t * 24.0,
                        10 => t * 3.0,
                        11 => t * 6.0,
                        12 => t * 12.0,
                        13 => t / 3600.0,
                        _ => t,
                    };
                }
            }
            5 => {
                npoints = be32(s, 5) as usize;
                rep = Some(Rep {
                    template: be16(s, 9),
                    r: f32::from_bits(be32(s, 11)) as f64,
                    e: sm16(be16(s, 15)),
                    d: sm16(be16(s, 17)),
                    bits: s[19],
                    sec5: s,
                });
            }
            6 => {
                if s[5] == 0 {
                    bitmap = Some(&s[6..]);
                } else if s[5] != 255 {
                    return Err(Error::Format("predefined bitmaps aren't supported"));
                }
            }
            7 => data7 = Some(&s[5..]),
            _ => {}
        }
        off += len;
    }
    let rep = rep.ok_or(Error::Format("missing data representation section"))?;
    let data = data7.ok_or(Error::Format("missing data section"))?;
    if f.ni == 0 || f.nj == 0 {
        return Err(Error::Format("missing grid definition"));
    }
    let packed = match rep.template {
        0 => unpack_simple(&rep, data, npoints)?,
        2 | 3 => unpack_complex(&rep, data, npoints)?,
        41 => unpack_png(&rep, data, npoints)?,
        t => return Err(Error::Unsupported(t)),
    };
    let total = f.ni * f.nj;
    f.values = match bitmap {
        Some(bm) => {
            let mut v = vec![f32::NAN; total];
            let mut k = 0;
            for (i, slot) in v.iter_mut().enumerate() {
                if bm[i / 8] & (0x80 >> (i % 8)) != 0 {
                    *slot = *packed.get(k).unwrap_or(&f32::NAN);
                    k += 1;
                }
            }
            v
        }
        None => {
            if packed.len() < total {
                return Err(Error::Truncated("fewer values than grid points"));
            }
            packed[..total].to_vec()
        }
    };
    Ok(f)
}

fn scale(rep: &Rep) -> (f64, f64, f64) {
    (rep.r, 2f64.powi(rep.e), 10f64.powi(-rep.d))
}

fn unpack_simple(rep: &Rep, data: &[u8], n: usize) -> Result<Vec<f32>, Error> {
    let (r, bs, ds) = scale(rep);
    if rep.bits == 0 {
        return Ok(vec![(r * ds) as f32; n]);
    }
    let mut b = Bits::new(data);
    (0..n).map(|_| Ok(((r + b.read(rep.bits as u32)? as f64 * bs) * ds) as f32)).collect()
}

/// Complex packing (5.2) and complex packing with spatial differencing (5.3).
fn unpack_complex(rep: &Rep, data: &[u8], n: usize) -> Result<Vec<f32>, Error> {
    let s = rep.sec5;
    let (r, bs, ds) = scale(rep);
    let missing_mode = s[22];
    let ng = be32(s, 31) as usize;
    let width_ref = s[35] as u32;
    let width_bits = s[36] as u32;
    let len_ref = be32(s, 37) as usize;
    let len_inc = s[41] as usize;
    let last_len = be32(s, 42) as usize;
    let len_bits = s[46] as u32;
    let (order, noct) = if rep.template == 3 { (s[47] as usize, s[48] as usize) } else { (0, 0) };

    // Spatial differencing descriptors: first value(s), then the overall minimum.
    let mut pos = 0;
    let mut firsts = Vec::new();
    let mut gmin = 0i64;
    if order > 0 {
        if noct == 0 || data.len() < noct * (order + 1) {
            return Err(Error::Truncated("spatial differencing descriptors"));
        }
        // First values are unsigned; only the overall minimum carries a sign bit (as in NCEP g2c).
        for _ in 0..order {
            firsts.push(data[pos..pos + noct].iter().fold(0i64, |a, &x| (a << 8) | x as i64));
            pos += noct;
        }
        gmin = sm_n(&data[pos..pos + noct]);
        pos += noct;
    }
    let mut b = Bits::new(&data[pos..]);
    let mut refs = Vec::with_capacity(ng);
    for _ in 0..ng {
        refs.push(b.read(rep.bits as u32)?);
    }
    b.align();
    let mut widths = Vec::with_capacity(ng);
    for _ in 0..ng {
        widths.push(b.read(width_bits)? + width_ref);
    }
    b.align();
    let mut lens = Vec::with_capacity(ng);
    for _ in 0..ng {
        lens.push(len_ref + b.read(len_bits)? as usize * len_inc);
    }
    if let Some(l) = lens.last_mut() {
        *l = last_len;
    }
    b.align();

    // Raw integers, None = missing.
    let mut raw: Vec<Option<i64>> = Vec::with_capacity(n);
    for g in 0..ng {
        let w = widths[g];
        let gref = refs[g] as i64;
        let all_ones_ref = if rep.bits > 0 { (1i64 << rep.bits) - 1 } else { 0 };
        for _ in 0..lens[g] {
            if w == 0 {
                raw.push(if missing_mode >= 1 && gref == all_ones_ref { None } else { Some(gref) });
            } else {
                let x = b.read(w)? as i64;
                let all_ones = (1i64 << w) - 1;
                if missing_mode >= 1 && x == all_ones {
                    raw.push(None);
                } else if missing_mode == 2 && x == all_ones - 1 {
                    raw.push(None);
                } else {
                    raw.push(Some(gref + x));
                }
            }
        }
    }
    raw.truncate(n);

    // Undo spatial differencing over the non-missing values.
    if order > 0 {
        let mut k = 0usize;
        let (mut p1, mut p2) = (0i64, 0i64);
        for v in raw.iter_mut() {
            let Some(x) = v.as_mut() else { continue };
            let val = if k < order {
                firsts[k]
            } else if order == 1 {
                *x + gmin + p1
            } else {
                *x + gmin + 2 * p1 - p2
            };
            *x = val;
            p2 = p1;
            p1 = val;
            k += 1;
        }
    }
    Ok(raw.into_iter().map(|v| v.map_or(f32::NAN, |x| ((r + x as f64 * bs) * ds) as f32)).collect())
}

fn unpack_png(rep: &Rep, data: &[u8], n: usize) -> Result<Vec<f32>, Error> {
    let (r, bs, ds) = scale(rep);
    let mut dec = png::Decoder::new(std::io::Cursor::new(data));
    dec.set_transformations(png::Transformations::IDENTITY);
    let mut reader = dec.read_info().map_err(|_| Error::Format("bad PNG in GRIB2"))?;
    let bytes = match reader.info().bit_depth {
        png::BitDepth::Eight => 1,
        png::BitDepth::Sixteen => 2,
        _ => return Err(Error::Format("unsupported PNG bit depth")),
    };
    let mut out = Vec::with_capacity(n);
    while let Some(row) = reader.next_row().map_err(|_| Error::Format("bad PNG row"))? {
        let d = row.data();
        for i in 0..d.len() / bytes {
            let x = if bytes == 2 { u16::from_be_bytes([d[2 * i], d[2 * i + 1]]) as f64 } else { d[i] as f64 };
            out.push(((r + x * bs) * ds) as f32);
        }
    }
    out.truncate(n);
    Ok(out)
}
