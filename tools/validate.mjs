// Compare model 10 m wind with airport observations (METARs) across the US.
//
//   node tools/validate.mjs
//
// Observations come from NOAA's Aviation Weather Center (via the NWS map
// service). For the most recent top of the hour, every station that reported
// within 15 minutes of it is compared with GFS and HRRR at the same moment.
// Prints vector error, speed error and direction error (for winds >= 2.5 m/s).

import fs from 'node:fs';
import { WindEngine, speedDir } from '../js/wind-engine.js';

const AWC = 'https://mapservices.weather.noaa.gov/vector/rest/services/aviation/awc_aviation_weather/MapServer/12/query'; // every reporting station (layer 34 is major airports only)
const KT = 0.514444;

async function metars() {
  const out = [];
  for (let offset = 0; offset < 8000; offset += 2000) {
    const q = new URLSearchParams({ where: '1=1', outFields: 'station_id,latitude,longitude,wind_speed_kt,winddir,observation_time', f: 'json',
      geometry: '-125,24,-66,50', geometryType: 'esriGeometryEnvelope', inSR: '4326', resultOffset: String(offset), resultRecordCount: '2000' });
    const d = await (await fetch(`${AWC}?${q}`)).json();
    const feats = d.features || [];
    out.push(...feats.map((f) => f.attributes));
    if (feats.length < 2000) break;
  }
  return out;
}

const obsAll = await metars();
// The top of the hour closest to most reports (routine METARs come in just before it).
const times = obsAll.map((o) => o.observation_time / 1000).filter(Boolean).sort((a, b) => a - b);
const valid = Math.round(times[Math.floor(times.length / 2)] / 3600) * 3600;
const obs = obsAll.filter((o) => o.wind_speed_kt != null && o.winddir != null && o.winddir !== 'VRB'
  && Math.abs(o.observation_time / 1000 - valid) <= 15 * 60);
const engine = new WindEngine({ wasmUrl: fs.readFileSync(new URL('../wasm/watergaw_wind.wasm', import.meta.url)), worker: false });
const points = obs.map((o) => [o.latitude, o.longitude]);
const rows = [];
for (const model of ['gfs', 'hrrr']) {
  const r = await engine.points(points, { model, time: valid });
  let vec = 0, spdAbs = 0, spdBias = 0, dirAbs = 0, nDir = 0;
  r.forEach((p, i) => {
    const o = obs[i];
    const s = o.wind_speed_kt * KT;
    const from = (+o.winddir * Math.PI) / 180;
    const ou = -s * Math.sin(from), ov = -s * Math.cos(from);
    vec += (p.u - ou) ** 2 + (p.v - ov) ** 2;
    spdAbs += Math.abs(p.speed - s);
    spdBias += p.speed - s;
    if (s >= 2.5) {
      nDir++;
      dirAbs += Math.abs(((p.from - +o.winddir + 540) % 360) - 180);
    }
  });
  const n = r.length;
  rows.push({ model: model.toUpperCase(), n, vectorRmse: Math.sqrt(vec / n), speedMae: spdAbs / n, speedBias: spdBias / n, dirMae: dirAbs / nDir, nDir });
}
console.log(`Valid ${new Date(valid * 1000).toISOString().slice(0, 16)}Z, ${obs.length} METAR stations (of ${obsAll.length})\n`);
console.log('| Model | Stations | Vector RMSE (m/s) | Speed MAE (m/s) | Speed bias (m/s) | Direction MAE (°, winds ≥ 2.5 m/s) |');
console.log('|---|---|---|---|---|---|');
for (const r of rows) console.log(`| ${r.model} | ${r.n} | ${r.vectorRmse.toFixed(2)} | ${r.speedMae.toFixed(2)} | ${r.speedBias >= 0 ? '+' : ''}${r.speedBias.toFixed(2)} | ${r.dirMae.toFixed(0)} (n=${r.nDir}) |`);
void speedDir;
