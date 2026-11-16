//! C ABI for the WebAssembly build. Results come back as a blob of up to four
//! byte buffers (JSON metadata plus raw arrays) that JavaScript copies out and
//! frees with `ww_blob_free`.

use crate::{grib2, wind};

#[repr(C)]
pub struct Blob {
    pub parts: [usize; 8],
}

#[repr(C)]
struct Owned {
    header: Blob,
    bufs: [Vec<u8>; 4],
}

fn blob(bufs: [Vec<u8>; 4]) -> *mut Blob {
    let mut o = Box::new(Owned { header: Blob { parts: [0; 8] }, bufs });
    for i in 0..4 {
        o.header.parts[i * 2] = o.bufs[i].as_ptr() as usize;
        o.header.parts[i * 2 + 1] = o.bufs[i].len();
    }
    Box::into_raw(o) as *mut Blob
}

fn error(msg: String) -> *mut Blob {
    blob([format!("{{\"error\":\"{}\"}}", msg.replace('"', "'")).into_bytes(), vec![], vec![], vec![]])
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn meta(f: &grib2::Field, g: &wind::Grid) -> String {
    format!(
        "{{\"ref_time\":{},\"lead_hours\":{},\"valid_time\":{},\"discipline\":{},\"category\":{},\"number\":{},\"source\":{{\"ni\":{},\"nj\":{},\"di\":{},\"dj\":{}}},\"west\":{},\"east\":{},\"south\":{},\"north\":{},\"nx\":{},\"ny\":{}}}",
        f.ref_time, f.lead_hours, f.ref_time + f.lead_hours * 3600.0, f.discipline, f.category, f.number,
        f.ni, f.nj, f.di, f.dj, g.west, g.east, g.south, g.north, g.nx, g.ny
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn ww_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` must come from `ww_alloc(len)` with the same `len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ww_free(ptr: *mut u8, len: usize) {
    unsafe { drop(Vec::from_raw_parts(ptr, 0, len.max(1))) }
}

/// # Safety
/// `ptr` must come from a function returning `*mut Blob`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ww_blob_free(ptr: *mut Blob) {
    if !ptr.is_null() {
        unsafe { drop(Box::from_raw(ptr as *mut Owned)) }
    }
}

/// Decode U and V GRIB2 messages and resample both onto an nx x ny box.
/// Parts: JSON metadata, U (f32), V (f32).
///
/// # Safety
/// The pointer/length pairs must be readable memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ww_wind_grid(
    u_ptr: *const u8, u_len: usize, v_ptr: *const u8, v_len: usize,
    west: f64, east: f64, south: f64, north: f64, nx: u32, ny: u32,
) -> *mut Blob {
    let (ub, vb) = unsafe { (std::slice::from_raw_parts(u_ptr, u_len), std::slice::from_raw_parts(v_ptr, v_len)) };
    let (u, v) = match (grib2::decode_all(ub), grib2::decode_all(vb)) {
        (Ok(mut a), Ok(mut b)) => (a.remove(0), b.remove(0)),
        (Err(e), _) | (_, Err(e)) => return error(e.to_string()),
    };
    let gu = wind::regrid(&u, west, east, south, north, nx as usize, ny as usize);
    let gv = wind::regrid(&v, west, east, south, north, nx as usize, ny as usize);
    blob([meta(&u, &gu).into_bytes(), f32_bytes(&gu.values), f32_bytes(&gv.values), vec![]])
}

/// Decode any single-field GRIB2 message and resample it onto an nx x ny box.
/// Parts: JSON metadata, values (f32).
///
/// # Safety
/// `ptr..ptr+len` must be readable memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ww_field_grid(ptr: *const u8, len: usize, west: f64, east: f64, south: f64, north: f64, nx: u32, ny: u32) -> *mut Blob {
    let data = unsafe { std::slice::from_raw_parts(ptr, len) };
    match grib2::decode_all(data) {
        Ok(mut fs) => {
            let f = fs.remove(0);
            let g = wind::regrid(&f, west, east, south, north, nx as usize, ny as usize);
            blob([meta(&f, &g).into_bytes(), f32_bytes(&g.values), vec![], vec![]])
        }
        Err(e) => error(e.to_string()),
    }
}
