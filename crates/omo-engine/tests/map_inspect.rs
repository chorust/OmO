//! `PointReadout` tests (`T022`, `SC-003`): the numeric hover/click value is the exact `f32`
//! stored for the nearest O1280 cell, never decoded from rendered color.

use omo_engine::{
    grid::reduced_gaussian::O1280,
    map::{CachedField, GridValues, inspect_field, point_in_bbox},
};
use omo_protocol::map::{FieldStats, ForecastSampleState, MapBbox, SourceLayout, ValueProvenance};

const MAX_DISTANCE_M: u32 = 7_200;

fn field(coverage: MapBbox) -> CachedField {
    let mut values = GridValues::o1280_empty();
    // Value encodes the flat index so a mismatch is unmistakable.
    for (flat, value) in values.as_mut_slice().iter_mut().enumerate() {
        *value = (flat % 4093) as f32 * 0.01 - 20.0;
    }
    CachedField {
        field_id: "f1".into(),
        entry_id: "ecmwf_ifs".into(),
        reference_time_utc: "2026-11-25T00:00:00Z".into(),
        valid_time_utc: "2026-11-25T12:00:00Z".into(),
        variable: "temperature_2m".into(),
        unit: "°C".into(),
        source_layout: SourceLayout::DataSpatial,
        source_object: "s3://x/y.om".into(),
        object_version: None,
        coverage,
        values,
        stats: FieldStats {
            valid_count: 0,
            min: -20.0,
            max: 20.0,
            unit: "°C".into(),
        },
    }
}

fn global() -> MapBbox {
    MapBbox {
        lat_min: -90.0,
        lat_max: 90.0,
        lon_spans: vec![[-180.0, 180.0]],
    }
}

#[test]
fn value_matches_o1280_nearest_bit_for_bit() {
    let field = field(global());
    for (lat, lon) in [
        (39.9042, 116.4074),
        (-33.8688, 151.2093),
        (0.0, 0.0),
        (51.5, -0.12),
        (-22.9, -43.2),
        (64.1, -21.9),
        (35.0, 179.99),
        (35.0, -179.99),
        (60.0, 180.0),
    ] {
        let nearest = O1280::nearest(lat, lon);
        let sample = inspect_field(&field, lat, lon, MAX_DISTANCE_M);
        match sample {
            ForecastSampleState::Value {
                value,
                unit,
                grid_lat,
                grid_lon,
                distance_m,
                provenance,
                stale,
            } => {
                assert_eq!(
                    value.to_bits(),
                    field.values.get(nearest.flat_index).to_bits(),
                    "{lat},{lon}"
                );
                assert_eq!(f64::from(nearest.lat), grid_lat);
                assert_eq!(f64::from(nearest.lon), grid_lon);
                assert_eq!(nearest.distance_m, distance_m);
                assert_eq!(unit, "°C");
                assert_eq!(provenance, ValueProvenance::NativeNumeric);
                assert!(!stale);
            }
            other => panic!("{lat},{lon}: expected a value, got {other:?}"),
        }
    }
}

#[test]
fn nan_cells_report_no_data_with_the_matched_grid_point() {
    let mut field = field(global());
    let nearest = O1280::nearest(30.0, 100.0);
    field.values.as_mut_slice()[nearest.flat_index as usize] = f32::NAN;
    match inspect_field(&field, 30.0, 100.0, MAX_DISTANCE_M) {
        ForecastSampleState::NoData {
            grid_lat,
            grid_lon,
            distance_m,
        } => {
            assert_eq!(grid_lat, f64::from(nearest.lat));
            assert_eq!(grid_lon, f64::from(nearest.lon));
            assert_eq!(distance_m, nearest.distance_m);
        }
        other => panic!("expected NoData, got {other:?}"),
    }
}

#[test]
fn points_outside_the_loaded_extent_are_flagged() {
    let coverage = MapBbox {
        lat_min: 10.0,
        lat_max: 40.0,
        lon_spans: vec![[100.0, 140.0]],
    };
    let field = field(coverage);
    assert!(matches!(
        inspect_field(&field, 25.0, 120.0, MAX_DISTANCE_M),
        ForecastSampleState::Value { .. }
    ));
    for (lat, lon) in [(5.0, 120.0), (45.0, 120.0), (25.0, 90.0), (25.0, -120.0)] {
        assert_eq!(
            inspect_field(&field, lat, lon, MAX_DISTANCE_M),
            ForecastSampleState::OutsideCachedExtent,
            "{lat},{lon}"
        );
    }
    // Invalid coordinates never panic.
    for (lat, lon) in [
        (f64::NAN, 0.0),
        (0.0, f64::INFINITY),
        (95.0, 0.0),
        (-91.0, 0.0),
    ] {
        assert_eq!(
            inspect_field(&field, lat, lon, MAX_DISTANCE_M),
            ForecastSampleState::OutsideCachedExtent
        );
    }
}

#[test]
fn antimeridian_spans_are_honoured() {
    let coverage = MapBbox {
        lat_min: -20.0,
        lat_max: 20.0,
        lon_spans: vec![[170.0, 180.0], [-180.0, -170.0]],
    };
    assert!(point_in_bbox(&coverage, 0.0, 175.0));
    assert!(point_in_bbox(&coverage, 0.0, -175.0));
    assert!(point_in_bbox(&coverage, 0.0, 180.0));
    assert!(point_in_bbox(&coverage, 0.0, -180.0));
    assert!(!point_in_bbox(&coverage, 0.0, 0.0));
    // Longitudes beyond +-180 wrap before the test.
    assert!(point_in_bbox(&coverage, 0.0, 185.0));
    let field = field(coverage);
    assert!(matches!(
        inspect_field(&field, 0.0, -179.0, MAX_DISTANCE_M),
        ForecastSampleState::Value { .. }
    ));
}

#[test]
fn polar_points_use_the_first_and_last_rows() {
    let field = field(global());
    for lat in [89.99, 90.0, -89.99, -90.0] {
        let nearest = O1280::nearest(lat, 12.0);
        match inspect_field(&field, lat, 12.0, MAX_DISTANCE_M) {
            ForecastSampleState::Value {
                value, distance_m, ..
            } => {
                assert_eq!(
                    value.to_bits(),
                    field.values.get(nearest.flat_index).to_bits()
                );
                assert_eq!(distance_m, nearest.distance_m);
            }
            other => panic!("lat {lat}: {other:?}"),
        }
    }
}

#[test]
fn matches_farther_than_the_limit_are_no_data() {
    let field = field(global());
    // 1 m can never be satisfied by an off-grid point.
    assert!(matches!(
        inspect_field(&field, 39.9042, 116.4074, 1),
        ForecastSampleState::NoData { .. }
    ));
}

#[test]
fn regular_grid_inspect_matches_nearest_cell_including_across_antimeridian() {
    use omo_engine::map::raster::RegularGrid;
    let (nx, ny) = (360usize, 181usize);
    let mut values = vec![f32::NAN; nx * ny];
    for y in 0..ny {
        for x in 0..nx {
            values[y * nx + x] = (y * 1000 + x) as f32;
        }
    }
    let reg = CachedField {
        field_id: "reg-1".into(),
        entry_id: "gfs.temperature_2m".into(),
        reference_time_utc: "2026-11-25T00:00:00Z".into(),
        valid_time_utc: "2026-11-25T06:00:00Z".into(),
        variable: "temperature_2m".into(),
        unit: "°C".into(),
        source_layout: SourceLayout::DataSpatial,
        source_object: "s3://x/reg.om".into(),
        object_version: None,
        coverage: global(),
        values: GridValues::Regular(RegularGrid {
            lat_top: 90.0,
            lon_west: -180.0,
            dlat: 1.0,
            dlon: 1.0,
            nx,
            ny,
            values,
        }),
        stats: FieldStats {
            valid_count: (nx * ny) as u64,
            min: 0.0,
            max: 200_000.0,
            unit: "°C".into(),
        },
    };
    // 179.9°E is 0.1° from -180.0° (col 0) and 0.9° from +179.0° (col 359) -> must pick col 0!
    match inspect_field(&reg, 0.0, 179.9, 50_000) {
        ForecastSampleState::Value {
            value,
            grid_lat,
            grid_lon,
            provenance,
            ..
        } => {
            assert_eq!(value, (90 * 1000) as f32);
            assert_eq!(grid_lat, 0.0);
            assert_eq!(grid_lon, -180.0);
            assert_eq!(provenance, ValueProvenance::NativeNumeric);
        }
        other => panic!("expected Value across seam, got {other:?}"),
    }
}
