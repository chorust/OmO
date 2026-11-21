//! The stable output schema shared by both M0 station-series plans.

use std::{fs, path::Path, sync::Arc};

use arrow::{
    array::{ArrayRef, Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use omo_protocol::result_ref::{ResultLimits, ResultRef};
use serde::{Deserialize, Serialize};

use super::{SeriesError, normalize_longitude};
use crate::results::write_arrow_batches;

pub const STATION_SERIES_COLUMNS: [&str; 9] = [
    "station_id",
    "variable",
    "valid_time",
    "value",
    "grid_lat",
    "grid_lon",
    "distance_m",
    "source_object",
    "object_version",
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct StationSeriesRow {
    pub station_id: String,
    pub variable: String,
    pub valid_time_utc: String,
    pub value: Option<f64>,
    pub grid_lat: Option<f64>,
    pub grid_lon: Option<f64>,
    pub distance_m: Option<f64>,
    pub source_object: Option<String>,
    pub object_version: Option<String>,
}

impl StationSeriesRow {
    pub fn validate(&self) -> Result<(), SeriesError> {
        if self.station_id.is_empty() || self.variable.is_empty() {
            return Err(SeriesError::InvalidInput(
                "station_series station_id and variable must be nonempty".to_owned(),
            ));
        }
        if !self.valid_time_utc.ends_with('Z') || !self.valid_time_utc.contains('T') {
            return Err(SeriesError::InvalidInput(format!(
                "station_series valid_time must be an explicit UTC timestamp: {}",
                self.valid_time_utc
            )));
        }
        for (name, value) in [
            ("value", self.value),
            ("grid_lat", self.grid_lat),
            ("grid_lon", self.grid_lon),
            ("distance_m", self.distance_m),
        ] {
            if value.is_some_and(|value| !value.is_finite()) {
                return Err(SeriesError::InvalidInput(format!(
                    "station_series {name} must be finite or NULL"
                )));
            }
        }
        if let Some(longitude) = self.grid_lon
            && normalize_longitude(longitude) != longitude
        {
            return Err(SeriesError::InvalidInput(
                "station_series grid_lon must use the half-open [-180,180) convention".to_owned(),
            ));
        }
        if self.distance_m.is_some_and(|distance| distance < 0.0) {
            return Err(SeriesError::InvalidInput(
                "station_series distance_m cannot be negative".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn same_semantics(&self, other: &Self) -> bool {
        self.station_id == other.station_id
            && self.variable == other.variable
            && self.valid_time_utc == other.valid_time_utc
            && self.value == other.value
            && self.grid_lat == other.grid_lat
            && self.grid_lon == other.grid_lon
            && self.distance_m == other.distance_m
    }
}

pub fn write_csv(path: &Path, rows: &[StationSeriesRow]) -> Result<(), SeriesError> {
    let mut output = String::new();
    output.push_str(&STATION_SERIES_COLUMNS.join(","));
    output.push('\n');
    for row in rows {
        row.validate()?;
        let fields = [
            csv_field(&row.station_id),
            csv_field(&row.variable),
            csv_field(&row.valid_time_utc),
            optional_float(row.value),
            optional_float(row.grid_lat),
            optional_float(row.grid_lon),
            optional_float(row.distance_m),
            row.source_object
                .as_deref()
                .map(csv_field)
                .unwrap_or_default(),
            row.object_version
                .as_deref()
                .map(csv_field)
                .unwrap_or_default(),
        ];
        output.push_str(&fields.join(","));
        output.push('\n');
    }
    fs::write(path, output)?;
    Ok(())
}

/// Write validated station-series rows to a generation-scoped Arrow IPC result file.
pub fn write_series_arrow(
    rows: &[StationSeriesRow],
    generation: u64,
    limits: ResultLimits,
    session_tmp: &Path,
    result_id: &str,
) -> Result<ResultRef, SeriesError> {
    for row in rows {
        row.validate()?;
    }
    let schema = Arc::new(Schema::new(vec![
        Field::new("station_id", DataType::Utf8, false),
        Field::new("variable", DataType::Utf8, false),
        Field::new("valid_time", DataType::Utf8, false),
        Field::new("value", DataType::Float64, true),
        Field::new("grid_lat", DataType::Float64, true),
        Field::new("grid_lon", DataType::Float64, true),
        Field::new("distance_m", DataType::Float64, true),
        Field::new("source_object", DataType::Utf8, true),
        Field::new("object_version", DataType::Utf8, true),
    ]));
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from_iter_values(
            rows.iter().map(|r| r.station_id.as_str()),
        )),
        Arc::new(StringArray::from_iter_values(
            rows.iter().map(|r| r.variable.as_str()),
        )),
        Arc::new(StringArray::from_iter_values(
            rows.iter().map(|r| r.valid_time_utc.as_str()),
        )),
        Arc::new(Float64Array::from_iter(rows.iter().map(|r| r.value))),
        Arc::new(Float64Array::from_iter(rows.iter().map(|r| r.grid_lat))),
        Arc::new(Float64Array::from_iter(rows.iter().map(|r| r.grid_lon))),
        Arc::new(Float64Array::from_iter(rows.iter().map(|r| r.distance_m))),
        Arc::new(StringArray::from_iter(
            rows.iter().map(|r| r.source_object.as_deref()),
        )),
        Arc::new(StringArray::from_iter(
            rows.iter().map(|r| r.object_version.as_deref()),
        )),
    ];
    let batch = RecordBatch::try_new(Arc::clone(&schema), columns).map_err(|error| {
        SeriesError::InvalidInput(format!("cannot build station_series Arrow batch: {error}"))
    })?;
    let batches = if rows.is_empty() {
        Vec::new()
    } else {
        vec![Ok::<RecordBatch, String>(batch)]
    };
    write_arrow_batches(schema, batches, session_tmp, result_id, generation, limits)
        .map_err(|error| SeriesError::InvalidInput(error.to_string()))
}

fn optional_float(value: Option<f64>) -> String {
    value.map_or_else(String::new, |value| format!("{value:.17}"))
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}
