//! O1280 reduced Gaussian row table.
//!
//! The coordinates follow the Open-Meteo Float32 producer rule
//! (`GaussianGrid.swift`, cited by research R-01). This is an independent
//! reimplementation: no code or tables are copied from duckomo (Constitution VII).
//!
//! - N = 1280, rows `y = 0…2559` run north to south.
//! - `nx(y) = 20 + 4·min(y, 2N−y−1)`.
//! - `dy = f32(180 / f32(2N+0.5))`.
//! - `lat(y) = f32((N−y−1)·dy) + dy/2`.
//! - `lon(x) = wrap(x·360/nx(y))` into [−180, 180).
//! - Flat index = Σ nx(<y) + x. Total 6,599,680 points.

use std::sync::OnceLock;

use super::{MEAN_EARTH_RADIUS_M, normalize_longitude, sphere_haversine_f64_v1};

const N: usize = 1280;
const ROWS: usize = 2 * N;
const POINTS: u32 = 6_599_680;

const fn nx_const(y: usize) -> u32 {
    let mirror = (2 * N - 1) - y;
    let m = if y < mirror { y } else { mirror };
    20 + 4 * (m as u32)
}

const fn build_offsets() -> [u32; ROWS + 1] {
    let mut offsets = [0u32; ROWS + 1];
    let mut y = 0usize;
    while y < ROWS {
        offsets[y + 1] = offsets[y] + nx_const(y);
        y += 1;
    }
    offsets
}

const ROW_OFFSETS: [u32; ROWS + 1] = build_offsets();

/// Nearest O1280 grid point for one station.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nearest {
    pub flat_index: u32,
    pub lat: f32,
    pub lon: f32,
    pub distance_m: f64,
}

/// Reduced Gaussian O1280 grid (ECMWF IFS HRES 9 km).
pub struct O1280;

impl O1280 {
    pub fn points() -> u32 {
        POINTS
    }

    pub fn nx(y: usize) -> u32 {
        assert!(y < ROWS, "O1280 row {y} is outside 0..{ROWS}");
        nx_const(y)
    }

    pub fn row_offset(y: usize) -> u32 {
        assert!(y < ROWS, "O1280 row {y} is outside 0..{ROWS}");
        ROW_OFFSETS[y]
    }

    pub fn flat_index(y: usize, x: usize) -> u32 {
        let nx = Self::nx(y);
        assert!(x < nx as usize, "O1280 column {x} is outside row {y}");
        Self::row_offset(y) + x as u32
    }

    /// `(lat, lon)` in degrees, Float32 producer rule.
    pub fn point(flat: u32) -> (f32, f32) {
        assert!(
            flat < POINTS,
            "O1280 flat index {flat} is outside 0..{POINTS}"
        );
        let y = row_of(flat);
        let x = (flat - ROW_OFFSETS[y]) as usize;
        (lat_f32(y), lon_f32(x, nx_const(y)))
    }

    /// Row whose Float32 latitude is closest to `lat`.
    pub fn row_near_lat(lat: f64) -> usize {
        if !lat.is_finite() {
            return 0;
        }
        if lat >= f64::from(lat_f32(0)) {
            return 0;
        }
        if lat <= f64::from(lat_f32(ROWS - 1)) {
            return ROWS - 1;
        }
        let mut lo = 0usize;
        let mut hi = ROWS - 1;
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if f64::from(lat_f32(mid)) >= lat {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let d_lo = (f64::from(lat_f32(lo)) - lat).abs();
        let d_hi = (f64::from(lat_f32(hi)) - lat).abs();
        if d_lo < d_hi || (d_lo == d_hi && lo < hi) {
            lo
        } else {
            hi
        }
    }

    /// Nearest grid point. Candidate rows are ±2 around [`row_near_lat`]; each row
    /// contributes its two longitude neighbours. Ties keep the smaller flat index.
    pub fn nearest(lat: f64, lon: f64) -> Nearest {
        let center = if lat > 89.9 {
            0
        } else if lat < -89.9 {
            ROWS - 1
        } else {
            Self::row_near_lat(lat)
        };
        let y0 = center.saturating_sub(2);
        let y1 = (center + 2).min(ROWS - 1);
        let mut best: Option<Nearest> = None;
        for y in y0..=y1 {
            let nx = nx_const(y) as usize;
            let (x_left, x_right) = longitude_neighbours(lon, nx);
            for x in [x_left, x_right] {
                let flat = ROW_OFFSETS[y] + x as u32;
                let (grid_lat, grid_lon) = (lat_f32(y), lon_f32(x, nx as u32));
                let distance_m = sphere_haversine_f64_v1(
                    lon,
                    lat,
                    f64::from(grid_lon),
                    f64::from(grid_lat),
                    MEAN_EARTH_RADIUS_M,
                );
                let candidate = Nearest {
                    flat_index: flat,
                    lat: grid_lat,
                    lon: grid_lon,
                    distance_m,
                };
                best = Some(match best {
                    None => candidate,
                    Some(current) => prefer(current, candidate),
                });
            }
        }
        best.expect("O1280 nearest search always inspects at least one row")
    }

    /// Half-cell diagonal bound (metres). Research R-01 rounds the observed maximum up to 7,200 m.
    pub fn max_nearest_distance_bound_m() -> f64 {
        *HALF_CELL_DIAGONAL_M.get_or_init(compute_half_cell_diagonal_m)
    }
}

fn prefer(current: Nearest, candidate: Nearest) -> Nearest {
    if candidate.distance_m < current.distance_m
        || (candidate.distance_m == current.distance_m && candidate.flat_index < current.flat_index)
    {
        candidate
    } else {
        current
    }
}

fn row_of(flat: u32) -> usize {
    let mut lo = 0usize;
    let mut hi = ROWS;
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if ROW_OFFSETS[mid] <= flat {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

fn dy_f32() -> f32 {
    // f32(180 / f32(2N + 0.5)). 2560.5 is exact in f32.
    let denom = (2.0f32 * N as f32) + 0.5;
    180.0f32 / denom
}

fn lat_f32(y: usize) -> f32 {
    let dy = dy_f32();
    let scaled = (N as f32 - y as f32 - 1.0) * dy;
    scaled + (dy / 2.0)
}

fn lon_f32(x: usize, nx: u32) -> f32 {
    let dlon = 360.0f32 / (nx as f32);
    let raw = (x as f32) * dlon;
    wrap_lon_f32(raw)
}

fn wrap_lon_f32(mut lon: f32) -> f32 {
    if lon >= 180.0 {
        lon -= 360.0;
    } else if lon < -180.0 {
        lon += 360.0;
    }
    if lon == 0.0 { 0.0 } else { lon }
}

fn longitude_neighbours(lon: f64, nx: usize) -> (usize, usize) {
    let unwrapped = {
        let normalized = normalize_longitude(lon);
        if normalized < 0.0 {
            normalized + 360.0
        } else {
            normalized
        }
    };
    let fraction = unwrapped / 360.0 * nx as f64;
    let left = fraction.floor().rem_euclid(nx as f64) as usize % nx;
    let right = fraction.ceil().rem_euclid(nx as f64) as usize % nx;
    (left, right)
}

fn compute_half_cell_diagonal_m() -> f64 {
    let dy = f64::from(dy_f32());
    let mut max_distance = 0.0f64;
    for y in 0..ROWS {
        let lat = f64::from(lat_f32(y));
        let dlon = 360.0 / f64::from(nx_const(y));
        let corner_lat = lat + dy / 2.0;
        let distance =
            sphere_haversine_f64_v1(0.0, lat, dlon / 2.0, corner_lat, MEAN_EARTH_RADIUS_M);
        if distance > max_distance {
            max_distance = distance;
        }
    }
    max_distance
}

static HALF_CELL_DIAGONAL_M: OnceLock<f64> = OnceLock::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_cover_every_point_once() {
        assert_eq!(ROW_OFFSETS[ROWS], POINTS);
        assert_eq!(O1280::nx(0), 20);
        assert_eq!(O1280::nx(1279), 5136);
        assert_eq!(O1280::nx(1280), 5136);
        assert_eq!(O1280::nx(2559), 20);
    }
}
