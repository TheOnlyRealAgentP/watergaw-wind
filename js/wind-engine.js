// watergaw-wind: free wind fields from NOAA weather models, in the browser.
//
//   import { WindEngine } from './wind-engine.js';
//   const engine = new WindEngine();
//   const wind = await engine.wind({ west: -125, east: -66, south: 24, north: 50 }, { nx: 236, ny: 104 });
//   wind.at(40.76, -111.89)                // -> [u, v] in m/s (u east, v north), or null
//   await engine.points([[40.76, -111.89]]) // -> [{ lat, lon, u, v, speed, from }]
//
// Models: HRRR (3 km, contiguous US, hourly) when the view is over the US and
// zoomed in, GFS (0.25°, global) otherwise; if HRRR fails we fall back to GFS.
// Only the U and V fields are downloaded (HTTP range requests guided by the
// published .idx inventories), from NOAA's open data on AWS. Decoding runs in
// Rust/WebAssembly inside a Web Worker when one is available.

import { loadWasm, windGrid, windPoints } from './wind-core.js';

const pad = (n, w = 2) => String(n).padStart(w, '0');
const ymd = (d) => `${d.getUTCFullYear()}${pad(d.getUTCMonth() + 1)}${pad(d.getUTCDate())}`;

export const MODELS = {
  gfs: {
    label: 'NOAA GFS 0.25°',
    cycleHours: 6,
    tries: 5,
    lead(h) { return h <= 120 ? h : Math.min(384, Math.round(h / 3) * 3); },
    maxLead() { return 384; },
    url(cycle, lead) {
      const d = new Date(cycle * 1000), hh = pad(d.getUTCHours());
      return `https://noaa-gfs-bdp-pds.s3.amazonaws.com/gfs.${ymd(d)}/${hh}/atmos/gfs.t${hh}z.pgrb2.0p25.f${pad(lead, 3)}`;
    },
    covers() { return true; },
  },
  hrrr: {
    label: 'NOAA HRRR 3 km',
    cycleHours: 1,
    tries: 4,
    lead(h) { return h; },
    maxLead(cycle) { return new Date(cycle * 1000).getUTCHours() % 6 === 0 ? 48 : 18; },
    url(cycle, lead) {
      const d = new Date(cycle * 1000);
      return `https://noaa-hrrr-bdp-pds.s3.amazonaws.com/hrrr.${ymd(d)}/conus/hrrr.t${pad(d.getUTCHours())}z.wrfsfcf${pad(lead)}.grib2`;
    },
    // Conservative rectangle inside HRRR's Lambert domain.
    covers(b) { return b.south >= 21.5 && b.north <= 52 && b.west >= -133 && b.east <= -61; },
  },
};

const U10 = ['UGRD', '10 m above ground'];
const V10 = ['VGRD', '10 m above ground'];

/** Parse a NOAA .idx inventory into { var, level, forecast, start, end } entries. */
export function parseIdx(text) {
  const rows = text.split('\n').map((l) => l.split(':')).filter((p) => p.length >= 6 && /^\d+$/.test(p[1]));
  return rows.map((p, i) => ({ var: p[3], level: p[4], forecast: p[5], start: +p[1], end: rows[i + 1] ? +rows[i + 1][1] - 1 : null }));
}

/** fetch with a timeout and retries (network errors, 429 and 5xx; never 4xx "not there"). */
export async function fetchRetry(url, init = {}, { timeout = 25000, retries = 2 } = {}) {
  let last;
  for (let a = 0; a <= retries; a++) {
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), timeout);
    try {
      const res = await fetch(url, { ...init, signal: ctl.signal });
      clearTimeout(timer);
      if (res.ok || (res.status >= 400 && res.status < 500 && res.status !== 429)) return res;
      last = new Error(`HTTP ${res.status}`);
    } catch (e) {
      clearTimeout(timer);
      last = e.name === 'AbortError' ? new Error('timed out') : e;
    }
    if (a < retries) await new Promise((r) => setTimeout(r, 500 * 3 ** a));
  }
  throw last;
}

/** Wrap U/V arrays (north row first) with bilinear sampling. */
export function makeGrid(box, nx, ny, u, v, info = {}) {
  const at = (lat, lonIn) => {
    let lon = lonIn;
    if (box.east > 180 && lon < box.west) lon += 360;
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

export class WindEngine {
  /**
   * @param opts.wasmUrl  where watergaw_wind.wasm lives (default ../wasm/ next to this file)
   * @param opts.worker   decode in a Web Worker when possible (default true)
   */
  constructor({ wasmUrl, worker = true } = {}) {
    this.wasmUrl = wasmUrl;
    this.useWorker = worker && typeof Worker !== 'undefined';
    this.files = new Map(); // model:validHour -> file
    this.raw = new Map(); // url -> Promise<{ u, v }>
    this.pending = new Map();
    this.nextId = 1;
  }

  // ------------------------------------------------------------ decoding

  async decode(op, payload) {
    if (this.useWorker) {
      if (!this.worker) {
        this.worker = new Worker(new URL('./wind-worker.js', import.meta.url), { type: 'module' });
        this.worker.onmessage = (e) => {
          const p = this.pending.get(e.data.id);
          if (!p) return;
          this.pending.delete(e.data.id);
          e.data.error ? p.reject(new Error(e.data.error)) : p.resolve(e.data.out);
        };
        this.worker.onerror = (e) => {
          // A broken worker (e.g. blocked module workers): fall back to the main thread.
          this.useWorker = false;
          for (const p of this.pending.values()) p.reject(new Error(e.message || 'wind worker failed'));
          this.pending.clear();
        };
      }
      const id = this.nextId++;
      return new Promise((resolve, reject) => {
        this.pending.set(id, { resolve, reject });
        // Copies, so cached GRIB bytes stay usable here.
        this.worker.postMessage({ id, op, ...payload, u: payload.u.slice(), v: payload.v.slice(), wasmUrl: this.wasmUrl ? String(this.wasmUrl) : undefined });
      }).catch(async (e) => {
        if (this.useWorker) throw e;
        return this.decode(op, payload);
      });
    }
    this.wasm ||= await loadWasm(this.wasmUrl);
    return op === 'points' ? windPoints(this.wasm, payload.u, payload.v, payload.points) : windGrid(this.wasm, payload.u, payload.v, payload.box, payload.nx, payload.ny);
  }

  // ------------------------------------------------------------ finding files

  /** Newest published file of a model valid at (or nearest) `time`. */
  async file(model, time = Date.now() / 1000) {
    const m = MODELS[model];
    const key = `${model}:${Math.round(time / 3600)}`;
    const hit = this.files.get(key);
    if (hit && Date.now() - hit.found < 10 * 60e3) return hit;
    const step = m.cycleHours * 3600;
    const latest = Math.floor(time / step) * step;
    for (let k = 0; k < m.tries; k++) {
      const cycle = latest - k * step;
      const lead = m.lead(Math.max(0, Math.round((time - cycle) / 3600)));
      if (lead > m.maxLead(cycle)) continue;
      const url = m.url(cycle, lead);
      let res;
      try {
        res = await fetchRetry(`${url}.idx`, {}, { timeout: 15000, retries: 1 });
      } catch {
        continue;
      }
      if (!res.ok) continue;
      const idx = parseIdx(await res.text());
      const find = ([v, l]) => idx.find((e) => e.var === v && e.level === l);
      if (!find(U10) || !find(V10)) continue;
      const f = { model, url, cycle, lead, valid: cycle + lead * 3600, u: find(U10), v: find(V10), found: Date.now() };
      this.files.set(key, f);
      if (this.files.size > 24) this.files.delete(this.files.keys().next().value);
      return f;
    }
    throw new Error(`no recent ${m.label} run is available`);
  }

  async range(url, e) {
    const res = await fetchRetry(url, { headers: { Range: `bytes=${e.start}-${e.end ?? ''}` } });
    if (!res.ok) throw new Error(`download failed (HTTP ${res.status})`);
    const bytes = new Uint8Array(await res.arrayBuffer());
    // Make sure we got exactly one complete GRIB2 message.
    const ok = bytes.length > 16 && bytes[0] === 0x47 && bytes[1] === 0x52 && bytes[2] === 0x49 && bytes[3] === 0x42
      && (e.end == null || bytes.length === e.end - e.start + 1)
      && bytes[bytes.length - 4] === 0x37 && bytes[bytes.length - 1] === 0x37;
    if (!ok) throw new Error('incomplete download');
    return bytes;
  }

  rawWind(f) {
    let p = this.raw.get(f.url);
    if (!p) {
      p = Promise.all([this.range(f.url, f.u), this.range(f.url, f.v)]).then(([u, v]) => ({ u, v }));
      p.catch(() => this.raw.delete(f.url));
      this.raw.set(f.url, p);
      if (this.raw.size > 6) this.raw.delete(this.raw.keys().next().value);
    }
    return p;
  }

  /** Models to try for a box, best first. */
  pick(box, model) {
    if (model && model !== 'auto') return model === 'hrrr' ? ['hrrr', 'gfs'] : [model];
    const wide = box.east - box.west > 30 || box.north - box.south > 20;
    return !wide && MODELS.hrrr.covers(box) ? ['hrrr', 'gfs'] : ['gfs'];
  }

  /** Run `job(file, raw)` with the first model that works; optionally blend two hours. */
  async withModel(box, { time, model, interpolate }, job) {
    const t = time ?? Date.now() / 1000;
    let lastErr;
    for (const name of this.pick(box, model)) {
      try {
        if (interpolate && Math.abs(t / 3600 - Math.round(t / 3600)) > 0.05) {
          const t0 = Math.floor(t / 3600) * 3600, t1 = t0 + 3600;
          const [f0, f1] = await Promise.all([this.file(name, t0), this.file(name, t1)]);
          if (f0.valid !== f1.valid) {
            const [r0, r1] = await Promise.all([this.rawWind(f0), this.rawWind(f1)]);
            const [a, b] = await Promise.all([job(f0, r0), job(f1, r1)]);
            const w = (t - f0.valid) / (f1.valid - f0.valid);
            const mix = (x, y) => x.map((val, i) => val * (1 - w) + y[i] * w);
            return { ...a, u: mix(a.u, b.u), v: mix(a.v, b.v), file: { ...f0, valid: t, blended: [f0, f1] } };
          }
        }
        const f = await this.file(name, t);
        const out = await job(f, await this.rawWind(f));
        return { ...out, file: f };
      } catch (e) {
        lastErr = e;
      }
    }
    throw lastErr || new Error('wind data unavailable');
  }

  // ------------------------------------------------------------ public API

  /**
   * Wind on an nx x ny grid over a box (cell centres, north row first).
   * @param box  { west, east, south, north } in degrees (east may exceed 180 to cross the date line)
   * @param opts { nx, ny, time (Unix s), model: 'auto' | 'hrrr' | 'gfs', interpolate: blend the two nearest hours }
   */
  async wind(boxIn, { nx = 96, ny = 64, time, model = 'auto', interpolate = false } = {}) {
    const box = { ...boxIn };
    if (box.east < box.west) box.east += 360;
    const r = await this.withModel(box, { time, model, interpolate }, (f, raw) => this.decode('grid', { u: raw.u, v: raw.v, box, nx, ny }));
    const f = r.file;
    return makeGrid(box, nx, ny, r.u, r.v, {
      model: f.model, refTime: f.cycle, validTime: f.valid,
      source: `${MODELS[f.model].label}, ${new Date(f.cycle * 1000).toISOString().slice(0, 13)}Z run${f.blended ? ' (interpolated)' : ` +${f.lead}h`}`,
    });
  }

  /** Wind at exact points [[lat, lon], ...]: [{ lat, lon, u, v, speed, from }]. */
  async points(points, { time, model = 'auto', interpolate = false } = {}) {
    const lats = points.map((p) => p[0]), lons = points.map((p) => p[1]);
    const box = { west: Math.min(...lons), east: Math.max(...lons), south: Math.min(...lats), north: Math.max(...lats) };
    const r = await this.withModel(box, { time, model, interpolate }, (f, raw) => this.decode('points', { u: raw.u, v: raw.v, points }));
    return points.map(([lat, lon], i) => {
      const [speed, from] = speedDir(r.u[i], r.v[i]);
      return { lat, lon, u: r.u[i], v: r.v[i], speed, from, model: r.file.model, validTime: r.file.valid };
    });
  }
}
