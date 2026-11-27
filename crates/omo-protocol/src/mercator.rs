//! Spherical Web Mercator (`EPSG:3857`) slippy-tile math and station framing.
//!
//! Contract: `specs/003-m2-regional-tile-map/contracts/layer-stack-and-pmtiles.md` §1.
//! Pixel addressing uses exact `u64` integer arithmetic for the longitude axis so adjacent
//! tiles (including across `x = 2^z - 1 -> 0`) receive bit-identical inputs.

use std::f64::consts::PI;

/// Latitude where the Web Mercator square ends: `atan(sinh(pi))`.
pub const MAX_LATITUDE: f64 = 85.05112878;
pub const TILE_SIZE: u32 = 256;
pub const MIN_ZOOM: u8 = 1;
pub const MAX_ZOOM: u8 = 12;
/// Minimum framed half-span in degrees for a single or degenerate station set.
pub const MIN_HALF_SPAN_DEG: f64 = 2.0;
/// Fractional padding added on each side when choosing the framing zoom.
pub const FRAME_PADDING: f64 = 0.1;

/// World size in pixels at zoom `z` (`256 << z`).
pub fn world_px(z: u8) -> u64 {
    u64::from(TILE_SIZE) << z
}

/// True when `lat` is finite and inside the projection square.
pub fn is_in_projection(lat: f64) -> bool {
    lat.is_finite() && lat.abs() <= MAX_LATITUDE
}

/// Normalize a longitude into `[-180, 180)`.
pub fn normalize_lon(lon: f64) -> f64 {
    let wrapped = (lon + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped == 0.0 { 0.0 } else { wrapped }
}

/// `(lon, lat)` degrees to fractional world pixels at zoom `z`. Latitudes outside the
/// projection square clamp to it.
pub fn lonlat_to_world_px(z: u8, lon: f64, lat: f64) -> (f64, f64) {
    let size = world_px(z) as f64;
    let lat = lat.clamp(-MAX_LATITUDE, MAX_LATITUDE);
    let x = (lon + 180.0) / 360.0 * size;
    let sin = lat.to_radians().sin();
    let y = (0.5 - ((1.0 + sin) / (1.0 - sin)).ln() / (4.0 * PI)) * size;
    (x, y)
}

/// Inverse of [`lonlat_to_world_px`]; longitude is normalized into `[-180, 180)`.
pub fn world_px_to_lonlat(z: u8, px: f64, py: f64) -> (f64, f64) {
    let size = world_px(z) as f64;
    let lon = normalize_lon(px / size * 360.0 - 180.0);
    let n = PI * (1.0 - 2.0 * py / size);
    let lat = n.sinh().atan().to_degrees();
    (lon, lat)
}

/// Global pixel of pixel `(u, v)` in tile `(z, x, y)`. `x` wraps modulo `2^z`; `y` does not.
pub fn tile_pixel_global(z: u8, x: u32, y: u32, u: u32, v: u32) -> (u64, u64) {
    let size = world_px(z);
    let gx = ((u64::from(x) * u64::from(TILE_SIZE)) + u64::from(u)) & (size - 1);
    let gy = u64::from(y) * u64::from(TILE_SIZE) + u64::from(v);
    (gx, gy)
}

/// East-longitude pixel index (`[0 deg, 360 deg)` in world pixels) for a global x pixel.
pub fn east_px(z: u8, gx: u64) -> u64 {
    let size = world_px(z);
    (gx + (size >> 1)) & (size - 1)
}

/// Latitude (degrees) at the vertical center of global pixel row `gy`.
pub fn lat_of_global_y(z: u8, gy: u64) -> f64 {
    let size = world_px(z) as f64;
    let n = PI * (1.0 - 2.0 * (gy as f64 + 0.5) / size);
    n.sinh().atan().to_degrees()
}

/// Automatic initial framing of the in-projection stations (`FR-002`).
#[derive(Clone, Debug, PartialEq)]
pub struct StationFrame {
    pub lat_min: f64,
    pub lat_max: f64,
    /// Western edge of the shortest longitude arc, in `[-180, 180)`.
    pub lon_west: f64,
    /// Length of the shortest longitude arc in degrees, at least `2 * MIN_HALF_SPAN_DEG`.
    pub lon_span_deg: f64,
    pub center_lat: f64,
    pub center_lon: f64,
    pub zoom: u8,
    /// Stations with `|lat| > MAX_LATITUDE` (or non-finite), reported but not framed.
    pub out_of_projection: usize,
}

impl StationFrame {
    /// Longitude segments covering the arc: one, or two when it crosses the antimeridian.
    pub fn lon_spans(&self) -> Vec<[f64; 2]> {
        let east = self.lon_west + self.lon_span_deg;
        if east <= 180.0 {
            vec![[self.lon_west, east]]
        } else {
            vec![[self.lon_west, 180.0], [-180.0, east - 360.0]]
        }
    }
}

/// Frame `(lon, lat)` stations for a viewport of `width x height` pixels.
///
/// Returns `None` when there is no in-projection station.
pub fn frame_stations(stations: &[(f64, f64)], width: f64, height: f64) -> Option<StationFrame> {
    let mut out_of_projection = 0usize;
    let mut lons: Vec<f64> = Vec::new();
    let mut lat_min = f64::INFINITY;
    let mut lat_max = f64::NEG_INFINITY;
    for &(lon, lat) in stations {
        if !lon.is_finite() || !is_in_projection(lat) {
            out_of_projection += 1;
            continue;
        }
        lons.push(normalize_lon(lon));
        lat_min = lat_min.min(lat);
        lat_max = lat_max.max(lat);
    }
    if lons.is_empty() {
        return None;
    }
    lons.sort_by(|a, b| a.total_cmp(b));

    // Largest gap between neighbouring longitudes (circular); the arc is its complement.
    let n = lons.len();
    let (mut west, mut span) = (lons[0], lons[n - 1] - lons[0]);
    if n > 1 {
        let mut best_gap = lons[0] + 360.0 - lons[n - 1];
        let mut best_west = lons[0];
        for i in 0..n - 1 {
            let gap = lons[i + 1] - lons[i];
            if gap > best_gap {
                best_gap = gap;
                best_west = lons[i + 1];
            }
        }
        west = best_west;
        span = 360.0 - best_gap;
        if best_west == lons[0] {
            span = lons[n - 1] - lons[0];
        }
    }

    if span < 2.0 * MIN_HALF_SPAN_DEG {
        let center = west + span / 2.0;
        west = center - MIN_HALF_SPAN_DEG;
        span = 2.0 * MIN_HALF_SPAN_DEG;
    }
    if lat_max - lat_min < 2.0 * MIN_HALF_SPAN_DEG {
        let center = (lat_min + lat_max) / 2.0;
        lat_min = center - MIN_HALF_SPAN_DEG;
        lat_max = center + MIN_HALF_SPAN_DEG;
    }
    lat_min = lat_min.max(-MAX_LATITUDE);
    lat_max = lat_max.min(MAX_LATITUDE);
    let west = normalize_lon(west);
    let center_lon = normalize_lon(west + span / 2.0);
    let center_lat = (lat_min + lat_max) / 2.0;

    let pad = 1.0 + 2.0 * FRAME_PADDING;
    let mut zoom = MIN_ZOOM;
    for z in MIN_ZOOM..=MAX_ZOOM {
        let size = world_px(z) as f64;
        let w = span / 360.0 * size;
        let (_, y_top) = lonlat_to_world_px(z, 0.0, lat_max);
        let (_, y_bottom) = lonlat_to_world_px(z, 0.0, lat_min);
        let h = y_bottom - y_top;
        if w * pad <= width && h * pad <= height {
            zoom = z;
        } else {
            break;
        }
    }
    Some(StationFrame {
        lat_min,
        lat_max,
        lon_west: west,
        lon_span_deg: span,
        center_lat,
        center_lon,
        zoom,
        out_of_projection,
    })
}
