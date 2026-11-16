//! Wind grids for a map view: U and V resampled onto a regular lat/lon box.

use crate::grib2::Field;

#[derive(Debug, Clone)]
pub struct Grid {
    pub west: f64,
    pub east: f64,
    pub south: f64,
    pub north: f64,
    pub nx: usize,
    pub ny: usize,
    /// Row-major, north row first, values at cell centres. NaN outside the source grid.
    pub values: Vec<f32>,
}

/// Bilinear resample of a field onto `nx` x `ny` cell centres covering the box.
pub fn regrid(f: &Field, west: f64, east: f64, south: f64, north: f64, nx: usize, ny: usize) -> Grid {
    let mut values = Vec::with_capacity(nx * ny);
    for j in 0..ny {
        let lat = north - (j as f64 + 0.5) / ny as f64 * (north - south);
        for i in 0..nx {
            let lon = west + (i as f64 + 0.5) / nx as f64 * (east - west);
            values.push(f.sample(lat, lon));
        }
    }
    Grid { west, east, south, north, nx, ny, values }
}

/// U and V resampled together, rotated to true east/north when the source
/// grid reports grid-relative components (e.g. HRRR's Lambert grid).
pub fn regrid_uv(u: &Field, v: &Field, west: f64, east: f64, south: f64, north: f64, nx: usize, ny: usize) -> (Grid, Grid) {
    let mut gu = Vec::with_capacity(nx * ny);
    let mut gv = Vec::with_capacity(nx * ny);
    for j in 0..ny {
        let lat = north - (j as f64 + 0.5) / ny as f64 * (north - south);
        for i in 0..nx {
            let lon = west + (i as f64 + 0.5) / nx as f64 * (east - west);
            let (ug, vg) = (u.sample(lat, lon), v.sample(lat, lon));
            let a = u.uv_rotation(lon) as f32;
            if a == 0.0 {
                gu.push(ug);
                gv.push(vg);
            } else {
                // NCEP/NCL rotation from grid-relative to earth-relative components.
                let (s, c) = a.sin_cos();
                gu.push(ug * c + vg * s);
                gv.push(vg * c - ug * s);
            }
        }
    }
    let mk = |values| Grid { west, east, south, north, nx, ny, values };
    (mk(gu), mk(gv))
}

/// Earth-relative U/V at exact points (lat, lon), rotating grid-relative winds.
pub fn sample_points(u: &Field, v: &Field, points: &[(f64, f64)]) -> (Vec<f32>, Vec<f32>) {
    points
        .iter()
        .map(|&(lat, lon)| {
            let (ug, vg) = (u.sample(lat, lon), v.sample(lat, lon));
            let a = u.uv_rotation(lon) as f32;
            if a == 0.0 { (ug, vg) } else {
                let (s, c) = a.sin_cos();
                (ug * c + vg * s, vg * c - ug * s)
            }
        })
        .unzip()
}

/// Speed (m/s) and the direction the wind blows FROM (degrees), meteorological convention.
pub fn speed_dir(u: f32, v: f32) -> (f32, f32) {
    let s = (u * u + v * v).sqrt();
    let d = (u.atan2(v).to_degrees() + 180.0).rem_euclid(360.0);
    (s, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_convention() {
        // A west wind blows from 270 toward the east: u > 0, v = 0.
        let (s, d) = speed_dir(10.0, 0.0);
        assert!((s - 10.0).abs() < 1e-6 && (d - 270.0).abs() < 1e-4);
        let (_, d) = speed_dir(0.0, -5.0); // from the north
        assert!(d.abs() < 1e-4 || (d - 360.0).abs() < 1e-4);
    }
}
