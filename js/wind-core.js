// Thin wrapper over the Rust/WebAssembly decoder, shared by the engine
// (main thread) and the decoding worker.

const WASM_URL = new URL('../wasm/watergaw_wind.wasm', import.meta.url);

export async function loadWasm(url = WASM_URL) {
  if (typeof url === 'string' || url instanceof URL) {
    try {
      return (await WebAssembly.instantiateStreaming(fetch(url), {})).instance.exports;
    } catch {
      const bytes = await (await fetch(url)).arrayBuffer();
      return (await WebAssembly.instantiate(bytes, {})).instance.exports;
    }
  }
  // Raw bytes (e.g. read from disk in Node).
  return (await WebAssembly.instantiate(url, {})).instance.exports;
}

function call(w, inputs, fn) {
  const mem = () => w.memory.buffer;
  const ptrs = inputs.map((bytes) => {
    const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const p = w.ww_alloc(view.length);
    new Uint8Array(mem(), p, view.length).set(view);
    return [p, view.length];
  });
  const blob = fn(ptrs);
  for (const [p, n] of ptrs) w.ww_free(p, n);
  const parts = new Uint32Array(mem(), blob, 8);
  const read = (i) => mem().slice(parts[i * 2], parts[i * 2] + parts[i * 2 + 1]);
  const meta = JSON.parse(new TextDecoder().decode(read(0)));
  const a = read(1), b = read(2);
  w.ww_blob_free(blob);
  if (meta.error) throw new Error(meta.error);
  return { meta, a: new Float32Array(a), b: new Float32Array(b) };
}

/** U/V regridded onto nx x ny cell centres of a box (north row first), earth-relative. */
export function windGrid(w, u, v, box, nx, ny) {
  const r = call(w, [u, v], ([[pu, lu], [pv, lv]]) => w.ww_wind_grid(pu, lu, pv, lv, box.west, box.east, box.south, box.north, nx, ny));
  return { meta: r.meta, u: r.a, v: r.b };
}

/** U/V at exact points [[lat, lon], ...], earth-relative. */
export function windPoints(w, u, v, points) {
  const pts = new Float64Array(points.flat());
  const r = call(w, [u, v, new Uint8Array(pts.buffer)], ([[pu, lu], [pv, lv], [pp]]) => w.ww_wind_points(pu, lu, pv, lv, pp, points.length));
  return { meta: r.meta, u: r.a, v: r.b };
}
