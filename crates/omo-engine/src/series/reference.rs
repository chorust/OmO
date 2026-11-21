//! Independent station-series oracle parsing and per-field semantic comparison.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use serde::Serialize;

use super::{SeriesError, schema::StationSeriesRow};

const EXPECTED_HEADER: &str =
    "station_id,variable,valid_time,value,grid_lat,grid_lon,distance_m,expected_point_index";
const VALUE_TOLERANCE: f64 = 1e-6;
const COORDINATE_TOLERANCE: f64 = 1e-10;
const DISTANCE_TOLERANCE_M: f64 = 1e-4;
const GRID_NX: i64 = 5;
const GRID_NY: i64 = 5;
const GRID_LAT0: f64 = 20.0;
const GRID_LON0: f64 = 100.0;
const GRID_STEP: f64 = 1.0;

#[derive(Clone, Debug)]
struct ReferenceRow {
    station_id: String,
    variable: String,
    valid_time_utc: String,
    value: Option<f64>,
    grid_lat: Option<f64>,
    grid_lon: Option<f64>,
    distance_m: Option<f64>,
    expected_point_index: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FieldCheck {
    pub status: &'static str,
    pub compared: usize,
    pub mismatches: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReferenceComparison {
    pub status: &'static str,
    pub expected_rows: usize,
    pub actual_rows: usize,
    pub actual_unique_keys: usize,
    pub semantic_checks: BTreeMap<String, FieldCheck>,
    pub provenance_check: FieldCheck,
    pub tolerances: BTreeMap<String, f64>,
}

impl ReferenceComparison {
    pub fn passed(&self) -> bool {
        self.status == "passed"
    }
}

pub fn compare_reference(
    actual: &[StationSeriesRow],
    reference_path: &Path,
) -> Result<ReferenceComparison, SeriesError> {
    let expected = read_reference(reference_path)?;
    let mut actual_by_key = BTreeMap::new();
    let mut key_mismatches = Vec::new();
    for row in actual {
        row.validate()?;
        let key = (
            row.station_id.clone(),
            row.variable.clone(),
            row.valid_time_utc.clone(),
        );
        if actual_by_key.insert(key.clone(), row).is_some() {
            key_mismatches.push(format!(
                "duplicate actual key: {}/{}/{}",
                key.0, key.1, key.2
            ));
        }
    }
    let mut expected_by_key = BTreeMap::new();
    for row in &expected {
        let key = (
            row.station_id.clone(),
            row.variable.clone(),
            row.valid_time_utc.clone(),
        );
        if expected_by_key.insert(key.clone(), row).is_some() {
            return Err(SeriesError::InvalidInput(format!(
                "reference contains duplicate key: {}/{}/{}",
                key.0, key.1, key.2
            )));
        }
    }

    let expected_keys = expected_by_key.keys().cloned().collect::<BTreeSet<_>>();
    let actual_keys = actual_by_key.keys().cloned().collect::<BTreeSet<_>>();
    if expected_keys != actual_keys {
        for key in expected_keys.difference(&actual_keys) {
            key_mismatches.push(format!("missing actual key: {}/{}/{}", key.0, key.1, key.2));
        }
        for key in actual_keys.difference(&expected_keys) {
            key_mismatches.push(format!(
                "unexpected actual key: {}/{}/{}",
                key.0, key.1, key.2
            ));
        }
    }

    let mut checks = BTreeMap::new();
    checks.insert(
        "station_variable_valid_time_key".to_owned(),
        field_check(expected.len(), key_mismatches),
    );
    let (value_count, value_mismatches) = compare_optional_numeric(
        &expected_by_key,
        &actual_by_key,
        |row| row.value,
        |row| row.value,
        VALUE_TOLERANCE,
        "value",
    );
    checks.insert(
        "value".to_owned(),
        field_check(value_count, value_mismatches),
    );
    let (lat_count, lat_mismatches) = compare_optional_numeric(
        &expected_by_key,
        &actual_by_key,
        |row| row.grid_lat,
        |row| row.grid_lat,
        COORDINATE_TOLERANCE,
        "grid_lat",
    );
    checks.insert(
        "grid_lat".to_owned(),
        field_check(lat_count, lat_mismatches),
    );
    let (lon_count, lon_mismatches) = compare_optional_numeric(
        &expected_by_key,
        &actual_by_key,
        |row| row.grid_lon,
        |row| row.grid_lon,
        COORDINATE_TOLERANCE,
        "grid_lon",
    );
    checks.insert(
        "grid_lon".to_owned(),
        field_check(lon_count, lon_mismatches),
    );
    let (distance_count, distance_mismatches) = compare_optional_numeric(
        &expected_by_key,
        &actual_by_key,
        |row| row.distance_m,
        |row| row.distance_m,
        DISTANCE_TOLERANCE_M,
        "distance_m",
    );
    checks.insert(
        "distance_m".to_owned(),
        field_check(distance_count, distance_mismatches),
    );

    let (index_count, index_mismatches) = compare_point_index(&expected_by_key, &actual_by_key);
    checks.insert(
        "nearest_point_index".to_owned(),
        field_check(index_count, index_mismatches),
    );
    let provenance_check = check_provenance(actual);
    let passed = checks.values().all(|check| check.status == "passed")
        && provenance_check.status == "passed"
        && actual.len() == expected.len();
    Ok(ReferenceComparison {
        status: if passed { "passed" } else { "failed" },
        expected_rows: expected.len(),
        actual_rows: actual.len(),
        actual_unique_keys: actual_by_key.len(),
        semantic_checks: checks,
        provenance_check,
        tolerances: BTreeMap::from([
            ("value_absolute".to_owned(), VALUE_TOLERANCE),
            (
                "grid_coordinate_absolute_degrees".to_owned(),
                COORDINATE_TOLERANCE,
            ),
            ("distance_absolute_meters".to_owned(), DISTANCE_TOLERANCE_M),
        ]),
    })
}

fn read_reference(path: &Path) -> Result<Vec<ReferenceRow>, SeriesError> {
    let content = fs::read_to_string(path)?;
    let mut lines = content.lines();
    if lines.next() != Some(EXPECTED_HEADER) {
        return Err(SeriesError::InvalidInput(format!(
            "reference CSV header must be {EXPECTED_HEADER}"
        )));
    }
    let mut rows = Vec::new();
    for (line_index, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields = line.split(',').collect::<Vec<_>>();
        if fields.len() != 8 {
            return Err(SeriesError::InvalidInput(format!(
                "reference CSV line {} must contain eight fields",
                line_index + 2
            )));
        }
        rows.push(ReferenceRow {
            station_id: fields[0].to_owned(),
            variable: fields[1].to_owned(),
            valid_time_utc: fields[2].to_owned(),
            value: parse_optional(fields[3], line_index)?,
            grid_lat: parse_optional(fields[4], line_index)?,
            grid_lon: parse_optional(fields[5], line_index)?,
            distance_m: parse_optional(fields[6], line_index)?,
            expected_point_index: if fields[7].is_empty() {
                None
            } else {
                Some(fields[7].parse::<usize>().map_err(|error| {
                    SeriesError::InvalidInput(format!(
                        "invalid point index on reference CSV line {}: {error}",
                        line_index + 2
                    ))
                })?)
            },
        });
    }
    if rows.is_empty() {
        return Err(SeriesError::InvalidInput(
            "reference CSV is empty".to_owned(),
        ));
    }
    Ok(rows)
}

fn parse_optional(value: &str, zero_based_line_index: usize) -> Result<Option<f64>, SeriesError> {
    if value.is_empty() {
        Ok(None)
    } else {
        let parsed = value.parse::<f64>().map_err(|error| {
            SeriesError::InvalidInput(format!(
                "invalid number on reference CSV line {}: {error}",
                zero_based_line_index + 2
            ))
        })?;
        if !parsed.is_finite() {
            return Err(SeriesError::InvalidInput(format!(
                "non-finite number on reference CSV line {}",
                zero_based_line_index + 2
            )));
        }
        Ok(Some(parsed))
    }
}

fn compare_optional_numeric(
    expected: &BTreeMap<(String, String, String), &ReferenceRow>,
    actual: &BTreeMap<(String, String, String), &StationSeriesRow>,
    expected_value: impl Fn(&ReferenceRow) -> Option<f64>,
    actual_value: impl Fn(&StationSeriesRow) -> Option<f64>,
    tolerance: f64,
    field: &str,
) -> (usize, Vec<String>) {
    let mut compared = 0usize;
    let mut mismatches = Vec::new();
    for (key, expected_row) in expected {
        let Some(actual_row) = actual.get(key) else {
            continue;
        };
        compared += 1;
        match (expected_value(expected_row), actual_value(actual_row)) {
            (None, None) => {}
            (Some(expected), Some(actual)) if (expected - actual).abs() <= tolerance => {}
            (expected, actual) => mismatches.push(format!(
                "{field} mismatch at {}/{}/{}: expected {expected:?}, actual {actual:?}",
                key.0, key.1, key.2
            )),
        }
    }
    (compared, mismatches)
}

fn compare_point_index(
    expected: &BTreeMap<(String, String, String), &ReferenceRow>,
    actual: &BTreeMap<(String, String, String), &StationSeriesRow>,
) -> (usize, Vec<String>) {
    let mut compared = 0usize;
    let mut mismatches = Vec::new();
    for (key, expected_row) in expected {
        let Some(actual_row) = actual.get(key) else {
            continue;
        };
        compared += 1;
        let actual_index = match (actual_row.grid_lat, actual_row.grid_lon) {
            (Some(latitude), Some(longitude)) => {
                let y = ((latitude - GRID_LAT0) / GRID_STEP).round() as i64;
                let x = ((longitude - GRID_LON0) / GRID_STEP).round() as i64;
                if y < 0
                    || y >= GRID_NY
                    || x < 0
                    || x >= GRID_NX
                    || (GRID_LAT0 + y as f64 * GRID_STEP - latitude).abs() > COORDINATE_TOLERANCE
                    || (GRID_LON0 + x as f64 * GRID_STEP - longitude).abs() > COORDINATE_TOLERANCE
                {
                    None
                } else {
                    Some((y * GRID_NX + x) as usize)
                }
            }
            (None, None) => None,
            _ => Some(usize::MAX),
        };
        if actual_index != expected_row.expected_point_index {
            mismatches.push(format!(
                "nearest point index mismatch at {}/{}/{}: expected {:?}, actual {:?}",
                key.0, key.1, key.2, expected_row.expected_point_index, actual_index
            ));
        }
    }
    (compared, mismatches)
}

fn check_provenance(rows: &[StationSeriesRow]) -> FieldCheck {
    let mut source_objects = BTreeSet::new();
    let mut versions = BTreeSet::new();
    let mut mismatches = Vec::new();
    let mut checked = 0usize;
    for row in rows {
        if row.grid_lat.is_some() && row.grid_lon.is_some() {
            checked += 1;
            match row.source_object.as_deref() {
                Some(source) if !source.is_empty() => {
                    source_objects.insert(source.to_owned());
                }
                _ => mismatches.push(format!(
                    "missing source_object at {}/{}/{}",
                    row.station_id, row.variable, row.valid_time_utc
                )),
            }
            versions.insert(row.object_version.clone());
        } else if row.source_object.is_some() || row.object_version.is_some() {
            mismatches.push(format!(
                "out-of-domain row carries source identity at {}/{}/{}",
                row.station_id, row.variable, row.valid_time_utc
            ));
        }
    }
    if source_objects.len() != 1 {
        mismatches.push(format!(
            "expected one source_object identity, found {}",
            source_objects.len()
        ));
    }
    if versions.len() > 1 {
        mismatches.push(format!(
            "source object_version changes within the fixture: {versions:?}"
        ));
    }
    field_check(checked, mismatches)
}

fn field_check(compared: usize, mut mismatches: Vec<String>) -> FieldCheck {
    let status = if mismatches.is_empty() {
        "passed"
    } else {
        "failed"
    };
    mismatches.truncate(50);
    FieldCheck {
        status,
        compared,
        mismatches,
    }
}
