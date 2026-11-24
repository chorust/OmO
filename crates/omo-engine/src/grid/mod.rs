//! Shared spherical distance for station-series plans.
//!
//! Research R-01: `sphere_haversine_f64_v1` uses the mean earth radius 6,371,008.8 m.
//! The same function serves Plan A (caller-supplied radius) and O1280 nearest search.

pub mod reduced_gaussian;

/// Mean earth radius used by O1280 nearest search (research R-01).
pub const MEAN_EARTH_RADIUS_M: f64 = 6_371_008.8;

const DEG_TO_RAD: f64 = std::f64::consts::PI / 180.0;

/// Normalize longitude into the half-open interval [−180, 180).
pub fn normalize_longitude(longitude: f64) -> f64 {
    let mut normalized = longitude % 360.0;
    if normalized < -180.0 {
        normalized += 360.0;
    } else if normalized >= 180.0 {
        normalized -= 360.0;
    }
    if normalized == 0.0 { 0.0 } else { normalized }
}

/// Great-circle distance on a sphere. Identical longitudes and latitudes return 0.
pub fn sphere_haversine_f64_v1(
    lon_a: f64,
    lat_a: f64,
    lon_b: f64,
    lat_b: f64,
    earth_radius_m: f64,
) -> f64 {
    if lat_a == lat_b && normalize_longitude(lon_a) == normalize_longitude(lon_b) {
        return 0.0;
    }
    let delta_lon = ((lon_b - lon_a + 180.0).rem_euclid(360.0) - 180.0).to_radians();
    let phi_a = lat_a * DEG_TO_RAD;
    let phi_b = lat_b * DEG_TO_RAD;
    let delta_phi = phi_b - phi_a;
    let sin_lat = (delta_phi / 2.0).sin();
    let sin_lon = (delta_lon / 2.0).sin();
    let a = (sin_lat * sin_lat + phi_a.cos() * phi_b.cos() * sin_lon * sin_lon).clamp(0.0, 1.0);
    earth_radius_m * 2.0 * a.sqrt().atan2((1.0 - a).sqrt())
}
