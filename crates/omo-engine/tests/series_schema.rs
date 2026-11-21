use omo_engine::series::{
    SeriesError, deduplicate_rows, normalize_longitude,
    schema::{STATION_SERIES_COLUMNS, StationSeriesRow, write_csv},
};

fn row(station: &str, time: &str, value: Option<f64>) -> StationSeriesRow {
    StationSeriesRow {
        station_id: station.to_owned(),
        variable: "value".to_owned(),
        valid_time_utc: time.to_owned(),
        value,
        grid_lat: Some(22.0),
        grid_lon: Some(102.0),
        distance_m: Some(0.0),
        source_object: Some("sha256:fixture".to_owned()),
        object_version: None,
    }
}

#[test]
fn station_series_csv_has_the_stable_column_order() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("series.csv");
    write_csv(&path, &[]).unwrap();
    let header = std::fs::read_to_string(&path).unwrap();
    assert_eq!(header.trim_end(), STATION_SERIES_COLUMNS.join(","));
    assert_eq!(
        STATION_SERIES_COLUMNS,
        [
            "station_id",
            "variable",
            "valid_time",
            "value",
            "grid_lat",
            "grid_lon",
            "distance_m",
            "source_object",
            "object_version",
        ]
    );
}

#[test]
fn deduplication_uses_station_variable_and_utc_time_key() {
    let first = row("A", "1970-01-01T00:00:00Z", Some(10.0));
    let duplicate = first.clone();
    let other_station = row("B", "1970-01-01T00:00:00Z", Some(10.0));
    let other_time = row("A", "1970-01-01T01:00:00Z", Some(11.0));

    let (rows, removed) = deduplicate_rows([first, duplicate, other_station, other_time]).unwrap();

    assert_eq!(removed, 1);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].station_id, "A");
    assert_eq!(rows[0].valid_time_utc, "1970-01-01T00:00:00Z");
    assert_eq!(rows[1].valid_time_utc, "1970-01-01T01:00:00Z");
    assert_eq!(rows[2].station_id, "B");
}

#[test]
fn conflicting_duplicates_fail_instead_of_hiding_a_value_difference() {
    let first = row("A", "1970-01-01T00:00:00Z", Some(10.0));
    let conflicting = row("A", "1970-01-01T00:00:00Z", Some(99.0));

    assert!(matches!(
        deduplicate_rows([first, conflicting]),
        Err(SeriesError::ConflictingDuplicate { .. })
    ));
}

#[test]
fn longitude_normalization_uses_half_open_minus_180_to_180_convention() {
    assert_eq!(normalize_longitude(462.0), 102.0);
    assert_eq!(normalize_longitude(-258.0), 102.0);
    assert_eq!(normalize_longitude(180.0), -180.0);
    assert_eq!(normalize_longitude(-180.0), -180.0);
    assert_eq!(normalize_longitude(540.0), -180.0);
}

#[test]
fn station_series_time_must_be_explicit_utc() {
    assert!(row("A", "1970-01-01T00:00:00Z", None).validate().is_ok());
    assert!(row("A", "1970-01-01 00:00:00", None).validate().is_err());
    assert!(
        row("A", "1970-01-01T00:00:00+00:00", None)
            .validate()
            .is_err()
    );
}

#[test]
fn null_values_are_valid_station_series_observations() {
    assert!(row("A", "1970-01-01T00:00:00Z", None).validate().is_ok());
}
