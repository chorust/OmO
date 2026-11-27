//! M2 regional tile map: cached forecast fields, tile rasterization, local PMTiles layers,
//! and composite PNG export.
//!
//! Everything here is UI-free and pure Rust (Constitution Principle I).

pub mod compose_export;
pub mod fetch;
pub mod field;
pub mod pmtiles_layer;
pub mod raster;
pub mod text;

pub use field::{CachedField, FieldCache, GridValues};

use omo_protocol::{
    map::{ForecastSampleState, MapBbox, ValueProvenance},
    mercator::normalize_lon,
};

use crate::grid::reduced_gaussian::O1280;

/// True when `(lat, lon)` lies inside the bounding region; longitudes wrap into `[-180, 180)`
/// before the span test, and the `+180` end of a span matches the `-180` start of the next.
pub fn point_in_bbox(bbox: &MapBbox, lat: f64, lon: f64) -> bool {
    if !lat.is_finite() || !lon.is_finite() || lat < bbox.lat_min || lat > bbox.lat_max {
        return false;
    }
    let lon = normalize_lon(lon);
    bbox.lon_spans
        .iter()
        .any(|[west, east]| (*west..=*east).contains(&lon) || (lon == -180.0 && *east >= 180.0))
}

/// `PointReadout` (`FR-012`): the exact stored `f32` of the nearest grid cell (O1280 or regular
/// lon/lat), sampled from the numeric field and never decoded from rendered color.
pub fn inspect_field(
    field: &CachedField,
    latitude: f64,
    longitude: f64,
    max_distance_m: u32,
) -> ForecastSampleState {
    if !latitude.is_finite()
        || !longitude.is_finite()
        || latitude.abs() > 90.0
        || !point_in_bbox(&field.coverage, latitude, longitude)
    {
        return ForecastSampleState::OutsideCachedExtent;
    }
    let (grid_lat, grid_lon, distance_m, value) = match &field.values {
        GridValues::O1280Dense(_) => {
            let nearest = O1280::nearest(latitude, longitude);
            (
                f64::from(nearest.lat),
                f64::from(nearest.lon),
                nearest.distance_m,
                field.values.get(nearest.flat_index),
            )
        }
        GridValues::Regular(grid) => {
            let Some((y, x, grid_lat, grid_lon, distance_m)) = grid.nearest(latitude, longitude)
            else {
                return ForecastSampleState::OutsideCachedExtent;
            };
            (grid_lat, grid_lon, distance_m, grid.values[y * grid.nx + x])
        }
    };
    if distance_m > f64::from(max_distance_m) || value.is_nan() {
        return ForecastSampleState::NoData {
            grid_lat,
            grid_lon,
            distance_m,
        };
    }
    ForecastSampleState::Value {
        value,
        unit: field.unit.clone(),
        grid_lat,
        grid_lon,
        distance_m,
        provenance: ValueProvenance::NativeNumeric,
        stale: false,
    }
}
