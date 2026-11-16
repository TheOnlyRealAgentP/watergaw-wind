# Watergaw Wind

Free wind data from NOAA's weather models, decoded in the browser or in Rust.
No API key, no server, no usage limits, and because NOAA data is public domain,
you can use the output in commercial apps too.

**Live demo:** https://theonlyrealagentp.github.io/watergaw-wind/

Watergaw Wind works the way services like Open-Meteo do: it doesn't run a
weather model, it repackages the output of the national ones, straight from
[NOAA Open Data](https://registry.opendata.aws/collab/noaa/) on AWS:

| Model | Resolution | Coverage | Updates | Used when |
|---|---|---|---|---|
| **HRRR** | 3 km | Contiguous US | Hourly | The view is over the US and smaller than ~30° × 20° |
| **GFS** | 0.25° (~25 km) | Global | Every 6 hours | Everywhere else, and as the fallback |

1. The file's `.idx` inventory gives the byte range of the 10 m U and V wind fields.
2. Two HTTP range requests download just those fields (1 to 2 MB each, not the 100 to 500 MB file).
3. A Rust GRIB2 decoder (compiled to WebAssembly, run in a Web Worker) unpacks them, projects
   HRRR's Lambert conformal grid, rotates its grid-relative winds to true north, and resamples
   U and V onto whatever box or points you ask for.

It powers the animated wind layer in [Watergaw](https://github.com/TheOnlyRealAgentP/watergaw).

## Use it in a web page

```js
import { WindEngine, speedDir } from './js/wind-engine.js';
import { WindParticles } from './js/wind-particles.js';

const engine = new WindEngine();                     // loads wasm/watergaw_wind.wasm
const wind = await engine.wind(
  { west: -125, east: -66, south: 24, north: 50 },   // any box
  { nx: 236, ny: 104 });                             // output grid size
const [u, v] = wind.at(40.76, -111.89);              // m/s east, north (bilinear)
const [speed, from] = speedDir(u, v);                // m/s, degrees the wind blows from

new WindParticles(map, (lat, lon) => wind.at(lat, lon)).start();  // MapLibre animation
```

Options for `wind()` and `points()`:

| Option | Default | Meaning |
|---|---|---|
| `time` | now | Unix seconds; the newest run with a forecast for that hour is used, so you can animate future wind |
| `model` | `'auto'` | `'hrrr'`, `'gfs'` or `'auto'` (HRRR for US views, GFS otherwise; falls back to GFS) |
| `interpolate` | `false` | Blend the two nearest forecast hours for times between hours (downloads twice as much) |
| `nx`, `ny` | 96, 64 | Output grid size for `wind()` |

`engine.points([[lat, lon], ...])` returns exact-point wind (`u`, `v`, `speed`, `from`)
without a grid. Requests time out and retry, a missing run falls back to the previous one,
every download is checked to be a complete GRIB2 message, and boxes may cross the date line
(`east` greater than 180). Copy `js/` and `wasm/` into your site; there are no other dependencies.

## Use it from Rust

```rust
let fields = watergaw_wind::grib2::decode_all(&bytes)?;          // any GRIB2 message(s)
let grid = watergaw_wind::wind::regrid(&fields[0], -125.0, -66.0, 24.0, 50.0, 236, 104);
let entries = watergaw_wind::idx::parse(&idx_text);              // .idx inventories
```

The GRIB2 decoder handles regular lat/lon (3.0) and Lambert conformal (3.30) grids with
simple (5.0), complex (5.2), complex with spatial differencing (5.3) and PNG (5.41) packing,
with or without bitmaps, and reports grid-relative winds (`wind::regrid_uv` and
`wind::sample_points` rotate them). That covers most NOAA model output, not only wind.

## Accuracy

**Decoding.** `cargo test` decodes real GFS and HRRR files and checks them against
pygrib/ecCodes: identical values (GFS complex packing with second-order spatial
differencing, 1,038,240 points; HRRR simple packing), and the HRRR Lambert projection
puts reference points on their exact grid cells (to 10⁻¹² of a cell).

**Against the weather.** `node tools/validate.mjs` compares both models with every
reporting US airport (METARs) at the latest top of the hour. On 16 Nov 2026 at 22:00 UTC:

| Model | Stations | Vector RMSE | Speed MAE | Speed bias | Direction MAE (winds ≥ 2.5 m/s) |
|---|---|---|---|---|---|
| GFS | 2,505 | 2.33 m/s | 1.28 m/s | +0.33 m/s | 20° |
| HRRR | 2,505 | 1.74 m/s | 0.93 m/s | +0.00 m/s | 14° |

That's why US views use HRRR: about 25% smaller errors, and much better in mountain valleys.

## Building

```
cargo test
cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/watergaw_wind.wasm wasm/
```

## Roadmap

Gusts, and more fields (temperature, pressure, precipitation) through the same pipeline.

## License

Copyright © 2026 Harrison Stewart. Free software under the
[GNU Affero General Public License v3.0 or later](LICENSE). Model data: NOAA (public domain).
