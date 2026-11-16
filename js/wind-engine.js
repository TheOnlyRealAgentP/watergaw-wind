// watergaw-wind: free wind fields from NOAA's GFS model, in the browser.
//
//   import { WindEngine } from './wind-engine.js';
//   const engine = new WindEngine();
//   const wind = await engine.wind({ west: -125, east: -66, south: 24, north: 50 }, { nx: 120, ny: 60 });
//   wind.at(40.76, -111.89)   // -> [u, v] in m/s (u east, v north), or null
//
// Only the ~1 MB of each wind component is downloaded: the .idx inventory says
// where U and V sit in the GFS file, and HTTP range requests fetch just those
// bytes from the NOAA Open Data bucket on AWS (which allows browser access).
// Decoding and regridding run in Rust compiled to WebAssembly.

const GFS = 'https://noaa-gfs-bdp-pds.s3.amazonaws.com';
const WASM_URL = new URL('../wasm/watergaw_wind.wasm', import.meta.url);

async function instantiate(url) {
  try {
    return (await WebAssembly.instantiateStreaming(fetch(url), {})).instance.exports;
  } catch {
    const bytes = await (await fetch(url)).arrayBuffer();
    return (await WebAssembly.instantiate(bytes, {})).instance.exports;
  }
}

/** Parse a NOAA .idx inventory into { var, level, forecast, start, end } entries. */
export function parseIdx(text) {
  const rows = text.split('\n').map((l) => l.split(':')).filter((p) => p.length >= 6 && /^\d+$/.test(p[1]));
  return rows.map((p, i) => ({ var: p[3], level: p[4], forecast: p[5], start: +p[1], end: rows[i + 1] ? +rows[i + 1][1] - 1 : null }));
}

const pad = (n, w = 2) => String(n).padStart(w, '0');

export class WindEngine {
  constructor({ wasmUrl = WASM_URL } = {}) {
    this.wasmUrl = wasmUrl;
    this.raw = new Map(); // file URL -> { u, v } GRIB2 bytes
  }

  async exports() {
    this.w ||= await instantiate(this.wasmUrl);
    return this.w;
  }

  /** GFS file URL for a cycle (Unix s) and lead (hours). */
  static gfsUrl(cycle, lead) {
    const d = new Date(cycle * 1000);
    const ymd = `${d.getUTCFullYear()}${pad(d.getUTCMonth() + 1)}${pad(d.getUTCDate())}`;
    const hh = pad(d.getUTCHours());
    return `${GFS}/gfs.${ymd}/${hh}/atmos/gfs.t${hh}z.pgrb2.0p25.f${pad(lead, 3)}`;
  }

  /** Newest published GFS file valid closest to `time` (Unix s, default now). */
  async gfsFile(time = Date.now() / 1000) {
    const latest = Math.floor(time / 21600) * 21600;
    for (let back = 0; back < 5; back++) {
      const cycle = latest - back * 21600;
      let lead = Math.max(0, Math.round((time - cycle) / 3600));
      if (lead > 120) lead = Math.min(384, Math.round(lead / 3) * 3);
      const url = WindEngine.gfsUrl(cycle, lead);
      const res = await fetch(`${url}.idx`);
      if (res.ok) return { url, cycle, lead, valid: cycle + lead * 3600, idx: parseIdx(await res.text()) };
    }
    throw new Error('no recent GFS run is available');
  }

  async range(url, e) {
    const res = await fetch(url, { headers: { Range: `bytes=${e.start}-${e.end ?? ''}` } });
    if (!res.ok) throw new Error(`GFS download failed (${res.status})`);
    return new Uint8Array(await res.arrayBuffer());
  }

  /** U/V GRIB2 bytes for one file (cached). */
  async rawWind(file) {
    let r = this.raw.get(file.url);
    if (r) return r;
    const find = (v) => file.idx.find((e) => e.var === v && e.level === '10 m above ground');
    const eu = find('UGRD'), ev = find('VGRD');
    if (!eu || !ev) throw new Error('10 m wind not found in the GFS inventory');
    const [u, v] = await Promise.all([this.range(file.url, eu), this.range(file.url, ev)]);
    r = { u, v };
    this.raw.set(file.url, r);
    if (this.raw.size > 4) this.raw.delete(this.raw.keys().next().value);
    return r;
  }

  /**
   * Wind on an nx x ny grid covering a box (cell centres, north row first).
   * Returns { u, v, nx, ny, box, refTime, validTime, at(lat, lon) }.
   */
  async wind(box, { nx = 96, ny = 64, time } = {}) {
    const [w, file] = await Promise.all([this.exports(), this.gfsFile(time)]);
    const raw = await this.rawWind(file);
    const mem = () => w.memory.buffer;
    const put = (bytes) => {
      const p = w.ww_alloc(bytes.length);
      new Uint8Array(mem(), p, bytes.length).set(bytes);
      return p;
    };
    const pu = put(raw.u), pv = put(raw.v);
    const blob = w.ww_wind_grid(pu, raw.u.length, pv, raw.v.length, box.west, box.east, box.south, box.north, nx, ny);
    w.ww_free(pu, raw.u.length);
    w.ww_free(pv, raw.v.length);
    const parts = new Uint32Array(mem(), blob, 8);
    const read = (i) => mem().slice(parts[i * 2], parts[i * 2] + parts[i * 2 + 1]);
    const meta = JSON.parse(new TextDecoder().decode(read(0)));
    const u = new Float32Array(read(1)), v = new Float32Array(read(2));
    w.ww_blob_free(blob);
    if (meta.error) throw new Error(meta.error);
    return makeGrid({ ...box }, nx, ny, u, v, { refTime: meta.ref_time, validTime: meta.valid_time, source: `NOAA GFS 0.25° ${new Date(file.cycle * 1000).toISOString().slice(0, 13)}Z +${file.lead}h` });
  }
}

/** Wrap U/V arrays with bilinear sampling. */
export function makeGrid(box, nx, ny, u, v, info = {}) {
  const at = (lat, lon) => {
    const fx = ((lon - box.west) / (box.east - box.west)) * nx - 0.5;
    const fy = ((box.north - lat) / (box.north - box.south)) * ny - 0.5;
    if (fx < -0.5 || fy < -0.5 || fx > nx - 0.5 || fy > ny - 0.5) return null;
    const x = Math.min(nx - 1.001, Math.max(0, fx)), y = Math.min(ny - 1.001, Math.max(0, fy));
    const i = Math.floor(x), j = Math.floor(y), tx = x - i, ty = y - j;
    const g = (a, ii, jj) => a[jj * nx + ii];
    const lerp = (a) => (g(a, i, j) * (1 - tx) + g(a, i + 1, j) * tx) * (1 - ty) + (g(a, i, j + 1) * (1 - tx) + g(a, i + 1, j + 1) * tx) * ty;
    const uu = lerp(u), vv = lerp(v);
    return Number.isFinite(uu) && Number.isFinite(vv) ? [uu, vv] : null;
  };
  return { box, nx, ny, u, v, at, ...info };
}

/** Speed (m/s) and direction the wind blows FROM (degrees). */
export function speedDir(u, v) {
  return [Math.hypot(u, v), ((Math.atan2(u, v) * 180) / Math.PI + 180 + 360) % 360];
}
