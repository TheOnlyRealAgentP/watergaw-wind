// Decodes GRIB2 wind off the main thread so maps keep animating smoothly.
import { loadWasm, windGrid, windPoints } from './wind-core.js';

let wasm = null;

self.onmessage = async (e) => {
  const { id, op, u, v, box, nx, ny, points, wasmUrl } = e.data;
  try {
    wasm ||= await loadWasm(wasmUrl ? new URL(wasmUrl) : undefined);
    const out = op === 'points' ? windPoints(wasm, u, v, points) : windGrid(wasm, u, v, box, nx, ny);
    self.postMessage({ id, out }, [out.u.buffer, out.v.buffer]);
  } catch (err) {
    self.postMessage({ id, error: String(err?.message || err) });
  }
};
