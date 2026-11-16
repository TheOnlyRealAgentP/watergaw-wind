//! Decoding a real GFS 0.25° 10 m U-wind field (complex packing with
//! second-order spatial differencing), checked against pygrib (ecCodes).

use serde_json::Value;
use watergaw_wind::grib2;

#[test]
fn gfs_u10_matches_pygrib() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let data = std::fs::read(dir.join("gfs_ugrd10m.grib2")).unwrap();
    let r: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("gfs_ugrd10m.ref.json")).unwrap()).unwrap();
    let f = grib2::decode_all(&data).unwrap().remove(0);
    assert_eq!((f.nj, f.ni), (721, 1440));
    assert_eq!((f.discipline, f.category, f.number), (0, 2, 2), "UGRD is discipline 0, category 2, number 2");
    assert!((f.la1 - 90.0).abs() < 1e-6 && f.lo1.abs() < 1e-6 && (f.di - 0.25).abs() < 1e-9);
    let close = |a: f64, b: f64| (a - b).abs() < 1e-3;
    let min = f.values.iter().cloned().fold(f32::INFINITY, f32::min) as f64;
    let max = f.values.iter().cloned().fold(f32::NEG_INFINITY, f32::max) as f64;
    let mean = f.values.iter().map(|&v| v as f64).sum::<f64>() / f.values.len() as f64;
    assert!(close(min, r["min"].as_f64().unwrap()) && close(max, r["max"].as_f64().unwrap()), "{min} {max}");
    assert!((mean - r["mean"].as_f64().unwrap()).abs() < 1e-5, "{mean}");
    for s in r["spots"].as_array().unwrap() {
        let (i, j, want) = (s[0].as_u64().unwrap() as usize, s[1].as_u64().unwrap() as usize, s[2].as_f64().unwrap());
        let got = f.values[i * f.ni + j] as f64;
        assert!(close(got, want), "({i},{j}) {got} vs {want}");
    }
    for (k, rs) in r["row_sums"].as_array().unwrap().iter().enumerate() {
        let row = k * 40;
        let sum: f64 = f.values[row * f.ni..(row + 1) * f.ni].iter().map(|&v| v as f64).sum();
        assert!((sum - rs.as_f64().unwrap()).abs() < 0.05, "row {row}: {sum} vs {}", rs);
    }
}
