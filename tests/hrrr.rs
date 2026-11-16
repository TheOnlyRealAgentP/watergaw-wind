//! HRRR 3 km (Lambert conformal, simple packing, grid-relative winds),
//! checked against pygrib (ecCodes): our projection must put each reference
//! point exactly on its grid cell, and the decoded values must match.

use serde_json::Value;
use watergaw_wind::{grib2, wind};

#[test]
fn hrrr_projection_and_values_match_pygrib() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let u = grib2::decode_all(&std::fs::read(dir.join("hrrr_ugrd10m.grib2")).unwrap()).unwrap().remove(0);
    let v = grib2::decode_all(&std::fs::read(dir.join("hrrr_vgrd10m.grib2")).unwrap()).unwrap().remove(0);
    let r: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("hrrr_10m.ref.json")).unwrap()).unwrap();
    assert_eq!((u.nj, u.ni), (1059, 1799));
    assert!(u.lambert.is_some() && u.uv_grid_relative && u.south_first);
    for s in r["spots"].as_array().unwrap() {
        let (j, i) = (s[0].as_u64().unwrap() as usize, s[1].as_u64().unwrap() as usize);
        let (lat, lon) = (s[2].as_f64().unwrap(), s[3].as_f64().unwrap());
        let (fi, fj) = u.index_of(lat, lon).expect("inside grid");
        assert!((fi - i as f64).abs() < 0.02 && (fj - j as f64).abs() < 0.02, "({lat},{lon}) -> ({fi},{fj}) want ({i},{j})");
        assert!((u.values[j * u.ni + i] as f64 - s[4].as_f64().unwrap()).abs() < 1e-3);
        assert!((v.values[j * v.ni + i] as f64 - s[5].as_f64().unwrap()).abs() < 1e-3);
    }
    // Rotation preserves speed, is zero on the central meridian (97.5 W), and turns
    // grid-north toward true north by n * dlon (n = sin 38.5) elsewhere.
    let (gu, gv) = wind::regrid_uv(&u, &v, -122.0, -72.0, 30.0, 45.0, 50, 15);
    let (ru, rv) = (wind::regrid(&u, -122.0, -72.0, 30.0, 45.0, 50, 15), wind::regrid(&v, -122.0, -72.0, 30.0, 45.0, 50, 15));
    for k in 0..gu.values.len() {
        let a = (gu.values[k].powi(2) + gv.values[k].powi(2)).sqrt();
        let b = (ru.values[k].powi(2) + rv.values[k].powi(2)).sqrt();
        assert!((a - b).abs() < 1e-3);
    }
    assert!(u.uv_rotation(-97.5).abs() < 1e-12);
    let a = u.uv_rotation(-122.5).to_degrees();
    assert!((a - (-25.0 * 38.5f64.to_radians().sin())).abs() < 1e-6, "{a}");
}
