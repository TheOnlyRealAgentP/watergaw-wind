//! watergaw-wind: free wind fields from NOAA weather models.
//!
//! Like Open-Meteo, this doesn't run a weather model; it decodes the public
//! output of NOAA's operational models (GFS and others) straight from the NOAA
//! Open Data buckets on AWS. Only the bytes for the fields you need are
//! downloaded (using the published `.idx` files and HTTP range requests), then
//! decoded here and resampled onto the grid you ask for. NOAA data is public
//! domain, so the output can be used for anything, including commercial apps.

pub mod ffi;
pub mod grib2;
pub mod idx;
pub mod wind;

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Truncated(&'static str),
    Format(&'static str),
    Unsupported(u16),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Truncated(m) => write!(f, "truncated data: {m}"),
            Error::Format(m) => write!(f, "bad format: {m}"),
            Error::Unsupported(t) => write!(f, "unsupported GRIB2 data representation template 5.{t}"),
        }
    }
}

impl std::error::Error for Error {}
