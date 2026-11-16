# Watergaw Wind

Free wind data from NOAA's weather models, decoded in the browser or in Rust.
No API key, no server, no usage limits, and because NOAA data is public domain,
you can use the output in commercial apps too.

**Live demo:** https://theonlyrealagentp.github.io/watergaw-wind/

Watergaw Wind works the way services like Open-Meteo do: it doesn't run a
weather model, it repackages the output of the national ones. It reads NOAA's
**GFS** (0.25°, global, hourly to 5 days) straight from the
[NOAA Open Data](https://registry.opendata.aws/noaa-gfs-bdp-pds/) bucket on AWS:

1. The file's `.idx` inventory gives the byte range of the 10 m U and V wind fields.
2. Two HTTP range requests download just those fields (about 1 MB each, not the 500 MB file).
3. A Rust GRIB2 decoder (compiled to WebAssembly) unpacks them and resamples U and V
   onto whatever lat/lon box and resolution you ask for.

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

`engine.wind(box, { time })` picks the newest published GFS run and the forecast
hour closest to `time` (Unix seconds; default now), so you can animate future
wind too. Copy `js/` and `wasm/` into your site; there are no other dependencies.

## Use it from Rust

```rust
let fields = watergaw_wind::grib2::decode_all(&bytes)?;          // any GRIB2 message(s)
let grid = watergaw_wind::wind::regrid(&fields[0], -125.0, -66.0, 24.0, 50.0, 236, 104);
let entries = watergaw_wind::idx::parse(&idx_text);              // .idx inventories
```

The GRIB2 decoder handles regular lat/lon grids (template 3.0) with simple (5.0),
complex (5.2), complex with spatial differencing (5.3) and PNG (5.41) packing,
with or without bitmaps. That covers most NOAA model output, not only wind.

## Accuracy

`cargo test` decodes a real GFS 10 m U-wind field (complex packing with
second-order spatial differencing, 1,038,240 points) and checks it against
pygrib/ecCodes: identical minimum, maximum and mean, spot values and row sums.

## Building

```
cargo test
cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/watergaw_wind.wasm wasm/
```

## Roadmap

HRRR (3 km, Lambert conformal) for higher-resolution US wind, gusts, and more
fields (temperature, pressure) through the same pipeline.

## License

Copyright © 2026 Harrison Stewart. Free software under the
[GNU Affero General Public License v3.0 or later](LICENSE). Model data: NOAA (public domain).
