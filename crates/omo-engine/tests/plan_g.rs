//! Offline Plan G pipeline. The point reader is a fake; no network and no duckomo.

use omo_engine::grid::reduced_gaussian::O1280;
use omo_engine::series::SeriesError;
use omo_engine::series::plan_g::{
    NoMatchReason, PlanGOutput, PointReader, PointRow, SelectedPoint, SeriesStation, assemble,
    build_point_predicate,
};
use omo_engine::series::schema::STATION_SERIES_COLUMNS;

struct FakeReader {
    times: Vec<(usize, f64)>,
    fail_duplicate: bool,
    fail_missing: bool,
    fixed: Vec<PointRow>,
}

impl PointReader for FakeReader {
    fn read_points(
        &mut self,
        _object_url: &str,
        points: &[SelectedPoint],
    ) -> Result<Vec<PointRow>, SeriesError> {
        if self.fail_missing {
            return Ok(Vec::new());
        }
        if !self.fixed.is_empty() {
            return Ok(self.fixed.clone());
        }
        let mut rows = Vec::new();
        for point in points {
            for (time_index, value) in &self.times {
                rows.push(PointRow {
                    lat: point.lat,
                    lon: point.lon,
                    time_index: *time_index,
                    valid_time: None,
                    value: Some(*value),
                });
            }
            if self.fail_duplicate {
                rows.push(PointRow {
                    lat: point.lat + 5e-5,
                    lon: point.lon,
                    time_index: 0,
                    valid_time: None,
                    value: Some(1.0),
                });
            }
        }
        Ok(rows)
    }
}

#[test]
fn predicate_splits_the_antimeridian_and_the_pipeline_joins_time() {
    let wrapped = SelectedPoint {
        flat_index: 1,
        lat: 10.0,
        lon: -180.0,
    };
    let predicate = build_point_predicate(&[wrapped]);
    assert!(predicate.contains("lon BETWEEN -180"), "{predicate}");
    assert!(predicate.contains("OR"), "{predicate}");

    let (lat, lon) = O1280::point(O1280::flat_index(1000, 10));
    let (lat_b, lon_b) = O1280::point(O1280::flat_index(1000, 12));
    let station = SeriesStation {
        station_id: "beta".into(),
        latitude: f64::from(lat),
        longitude: f64::from(lon),
    };
    let other = SeriesStation {
        station_id: "alpha".into(),
        latitude: f64::from(lat_b),
        longitude: f64::from(lon_b),
    };
    let mut reader = FakeReader {
        times: vec![(1, 2.0), (0, 1.0)],
        fail_duplicate: false,
        fail_missing: false,
        fixed: Vec::new(),
    };
    let PlanGOutput { rows, matches } = assemble(
        &[other.clone(), station.clone()],
        "temperature_2m",
        "s3://openmeteo/data_run/ecmwf_ifs/2026/11/23/0000Z/temperature_2m.om",
        "etag",
        7_200.0,
        &[
            (0, "2026-11-23T00:00:00Z".into()),
            (1, "2026-11-23T01:00:00Z".into()),
        ],
        "s3://example/object.om",
        &mut reader,
    )
    .unwrap();
    assert!(
        matches.iter().all(|row| row.reason.is_none()),
        "{matches:?}"
    );
    assert_eq!(rows.len(), 4);
    assert!(rows.windows(2).all(|pair| {
        pair[0].station_id < pair[1].station_id
            || (pair[0].station_id == pair[1].station_id
                && pair[0].valid_time_utc <= pair[1].valid_time_utc)
    }));
    assert_eq!(rows[0].station_id, "alpha");
    assert_eq!(rows[0].valid_time_utc, "2026-11-23T00:00:00Z");
    assert_eq!(
        rows[0].source_object.as_deref(),
        Some("s3://openmeteo/data_run/ecmwf_ifs/2026/11/23/0000Z/temperature_2m.om")
    );
    for name in STATION_SERIES_COLUMNS {
        let _ = name;
    }
    assert_eq!(STATION_SERIES_COLUMNS.len(), 9);
}

#[test]
fn invalid_coordinates_and_grid_mismatches_are_reported() {
    let bad = SeriesStation {
        station_id: "bad".into(),
        latitude: 120.0,
        longitude: 10.0,
    };
    let mut reader = FakeReader {
        times: Vec::new(),
        fail_duplicate: false,
        fail_missing: false,
        fixed: Vec::new(),
    };
    let output = assemble(
        &[bad],
        "temperature_2m",
        "s3://example/object.om",
        "etag",
        7_200.0,
        &[],
        "s3://example/object.om",
        &mut reader,
    )
    .unwrap();
    assert_eq!(output.rows.len(), 0);
    assert_eq!(
        output.matches[0].reason,
        Some(NoMatchReason::InvalidCoordinate)
    );

    let (lat, lon) = O1280::point(O1280::flat_index(800, 4));
    let off = SeriesStation {
        station_id: "off".into(),
        latitude: f64::from(lat) + 0.05,
        longitude: f64::from(lon),
    };
    let tightened = assemble(
        &[off],
        "temperature_2m",
        "s3://example/object.om",
        "etag",
        1.0,
        &[],
        "s3://example/object.om",
        &mut reader,
    )
    .unwrap();
    assert_eq!(
        tightened.matches[0].reason,
        Some(NoMatchReason::BeyondMaxDistance)
    );

    let (lat, lon) = O1280::point(0);
    let station = SeriesStation {
        station_id: "pole".into(),
        latitude: f64::from(lat),
        longitude: f64::from(lon),
    };
    let mut missing = FakeReader {
        times: vec![(0, 0.0)],
        fail_duplicate: false,
        fail_missing: true,
        fixed: Vec::new(),
    };
    let error = assemble(
        &[station.clone()],
        "temperature_2m",
        "s3://example/object.om",
        "etag",
        7_200.0,
        &[(0, "2026-11-23T00:00:00Z".into())],
        "s3://example/object.om",
        &mut missing,
    )
    .unwrap_err();
    assert!(matches!(error, SeriesError::GridMismatch(_)), "{error}");

    let mut duplicate = FakeReader {
        times: vec![(0, 0.0)],
        fail_duplicate: true,
        fail_missing: false,
        fixed: Vec::new(),
    };
    let error = assemble(
        &[station],
        "temperature_2m",
        "s3://example/object.om",
        "etag",
        7_200.0,
        &[(0, "2026-11-23T00:00:00Z".into())],
        "s3://example/object.om",
        &mut duplicate,
    )
    .unwrap_err();
    assert!(matches!(error, SeriesError::GridMismatch(_)), "{error}");
}

#[test]
fn series_cancel_flag_is_checked_and_reset_between_requests() {
    use omo_engine::series::{SeriesCancel, SeriesError};

    let cancel = SeriesCancel::default();
    assert!(cancel.check().is_ok());
    cancel.cancel();
    assert!(cancel.is_cancelled());
    assert!(matches!(cancel.check(), Err(SeriesError::Cancelled)));
    cancel.reset();
    assert!(!cancel.is_cancelled());
    assert!(cancel.check().is_ok());
}
