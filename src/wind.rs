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
