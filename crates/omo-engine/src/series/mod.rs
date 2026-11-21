//! M0 station-series plans and their stable comparison schema.

pub mod plan_a;
pub mod plan_b;
pub mod reference;
pub mod schema;

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Station {
    pub station_id: String,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScanMeasurement {
    pub scan_id: Option<u64>,
    pub operation: Option<String>,
    pub data_bytes: Option<u64>,
    pub physical_read_bytes: Option<u64>,
    pub metadata_bytes: Option<u64>,
    pub index_bytes: Option<u64>,
    pub scan_complete: Option<bool>,
    pub raw_metrics: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct SeriesRun {
    pub rows: Vec<schema::StationSeriesRow>,
    pub elapsed_ms: u64,
    pub scan: ScanMeasurement,
    pub duplicate_rows_removed: usize,
    pub plan_details: Value,
}

#[derive(Debug, Error)]
pub enum SeriesError {
    #[error("series input I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("DuckDB failed during {stage}: {reason}")]
    DuckDb { stage: &'static str, reason: String },
    #[error("invalid series input: {0}")]
    InvalidInput(String),
    #[error(
        "duplicate station-series key has conflicting semantic values: {station_id}/{variable}/{valid_time}"
    )]
    ConflictingDuplicate {
        station_id: String,
        variable: String,
        valid_time: String,
    },
}

pub const GRID_SQL: &str =
    "{'nx':5,'ny':5,'lat0':20.0,'lon0':100.0,'dlat':1.0,'dlon':1.0,'order':'separate'}";
pub const DIMENSIONS_SQL: &str = "map(['/'], [['member','time','lat','lon']])";
pub const SPATIAL_AXES_SQL: &str = "['lat','lon']";
pub const TIME_STEP_SQL: &str = "INTERVAL '1 hour'";
pub const WINDOW_START_UTC: &str = "1970-01-01T00:00:00Z";
pub const WINDOW_END_UTC: &str = "1970-01-01T02:00:00Z";
pub const CHUNK_TIME_LENGTH: i64 = 3;
pub const TIME_STEP_SECONDS: i64 = 3_600;
pub const SERIES_VARIABLE: &str = "value";

pub fn read_stations(path: &Path) -> Result<Vec<Station>, SeriesError> {
    let content = fs::read_to_string(path)?;
    let mut lines = content.lines();
    if lines.next() != Some("station_id,latitude,longitude") {
        return Err(SeriesError::InvalidInput(
            "station CSV header must be station_id,latitude,longitude".to_owned(),
        ));
    }
    let mut stations = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    for (line_index, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields = line.split(',').collect::<Vec<_>>();
        if fields.len() != 3 || fields.iter().any(|field| field.contains('"')) {
            return Err(SeriesError::InvalidInput(format!(
                "station CSV line {} must contain three unquoted fields",
                line_index + 2
            )));
        }
        let station_id = fields[0].trim();
        if station_id.is_empty() || !ids.insert(station_id.to_owned()) {
            return Err(SeriesError::InvalidInput(format!(
                "station id on CSV line {} is empty or duplicated",
                line_index + 2
            )));
        }
        let latitude = fields[1].trim().parse::<f64>().map_err(|error| {
            SeriesError::InvalidInput(format!(
                "invalid latitude on CSV line {}: {error}",
                line_index + 2
            ))
        })?;
        let longitude = fields[2].trim().parse::<f64>().map_err(|error| {
            SeriesError::InvalidInput(format!(
                "invalid longitude on CSV line {}: {error}",
                line_index + 2
            ))
        })?;
        if !latitude.is_finite() || !longitude.is_finite() || !(-90.0..=90.0).contains(&latitude) {
            return Err(SeriesError::InvalidInput(format!(
                "station coordinates on CSV line {} must be finite and latitude in [-90,90]",
                line_index + 2
            )));
        }
        stations.push(Station {
            station_id: station_id.to_owned(),
            latitude,
            longitude,
        });
    }
    if stations.is_empty() {
        return Err(SeriesError::InvalidInput(
            "station CSV contains no stations".to_owned(),
        ));
    }
    Ok(stations)
}

pub fn normalize_longitude(longitude: f64) -> f64 {
    let mut normalized = longitude % 360.0;
    if normalized < -180.0 {
        normalized += 360.0;
    } else if normalized >= 180.0 {
        normalized -= 360.0;
    }
    if normalized == 0.0 { 0.0 } else { normalized }
}

pub fn quote_sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub fn deduplicate_rows(
    rows: impl IntoIterator<Item = schema::StationSeriesRow>,
) -> Result<(Vec<schema::StationSeriesRow>, usize), SeriesError> {
    use std::collections::BTreeMap;

    let mut unique = BTreeMap::new();
    let mut duplicates = 0usize;
    for row in rows {
        row.validate()?;
        let key = (
            row.station_id.clone(),
            row.variable.clone(),
            row.valid_time_utc.clone(),
        );
        match unique.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(row);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if !entry.get().same_semantics(&row) {
                    return Err(SeriesError::ConflictingDuplicate {
                        station_id: row.station_id,
                        variable: row.variable,
                        valid_time: row.valid_time_utc,
                    });
                }
                let current = entry.get();
                let candidate_provenance = (&row.source_object, &row.object_version);
                let current_provenance = (&current.source_object, &current.object_version);
                if candidate_provenance < current_provenance {
                    entry.insert(row);
                }
                duplicates += 1;
            }
        }
    }
    Ok((unique.into_values().collect(), duplicates))
}

pub fn read_latest_scan_measurement(
    connection: &duckdb::Connection,
) -> Result<ScanMeasurement, SeriesError> {
    let found = connection
        .query_row(
            "SELECT scan_id, metrics FROM duckomo_last_scan_metrics() ORDER BY scan_id DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| SeriesError::DuckDb {
            stage: "scan_metrics",
            reason: error.to_string(),
        })?;
    let raw_metrics: Value =
        serde_json::from_str(&found.1).map_err(|error| SeriesError::DuckDb {
            stage: "scan_metrics_json",
            reason: error.to_string(),
        })?;
    let data_bytes = raw_metrics
        .pointer("/reads/value_totals/data_bytes")
        .and_then(Value::as_u64);
    let physical_read_bytes = raw_metrics
        .pointer("/legacy_v3/physical_read_bytes")
        .and_then(Value::as_u64);
    let metadata_bytes = raw_metrics
        .pointer("/reads/metadata/bind_bytes")
        .and_then(Value::as_u64)
        .zip(
            raw_metrics
                .pointer("/reads/metadata/scan_bytes")
                .and_then(Value::as_u64),
        )
        .map(|(bind, scan)| bind.saturating_add(scan));
    let index_bytes = raw_metrics
        .pointer("/reads/value_totals/index_bytes")
        .and_then(Value::as_u64);
    let scan_complete = raw_metrics
        .pointer("/legacy_v3/scan_complete")
        .and_then(Value::as_bool)
        .or_else(|| {
            raw_metrics
                .pointer("/outcome/scan_complete")
                .and_then(Value::as_bool)
        });
    let operation = raw_metrics
        .get("operation")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(ScanMeasurement {
        scan_id: Some(found.0),
        operation,
        data_bytes,
        physical_read_bytes,
        metadata_bytes,
        index_bytes,
        scan_complete,
        raw_metrics,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct RegularGridSpec {
    pub nx: usize,
    pub ny: usize,
    pub latitude_origin: f64,
    pub longitude_origin: f64,
    pub latitude_step: f64,
    pub longitude_step: f64,
    pub earth_radius_m: f64,
    pub latitude_axis: usize,
    pub longitude_axis: usize,
}

pub fn regular_grid_from_descriptor(descriptor: &Value) -> Result<RegularGridSpec, SeriesError> {
    let parameters = descriptor.pointer("/definition/parameters");
    let layout = descriptor.get("layout");
    let crs = descriptor.get("crs");
    let output_crs = crs
        .and_then(|crs| crs.get("output_crs"))
        .and_then(Value::as_str);
    let axis_order = crs
        .and_then(|crs| crs.get("axis_order"))
        .and_then(Value::as_array);
    let invalid = || {
        SeriesError::InvalidInput(format!(
            "station-series plans require a regular spherical lon/lat grid in separate-axis order; got {descriptor}"
        ))
    };
    if descriptor.get("descriptor_version").and_then(Value::as_i64) != Some(1)
        || descriptor.get("grid_type").and_then(Value::as_str) != Some("regular_latlon")
        || output_crs != Some("+proj=longlat +R=6371229 +type=crs")
        || axis_order.is_none_or(|order| {
            order.len() != 2
                || order[0].as_str() != Some("longitude")
                || order[1].as_str() != Some("latitude")
        })
        || layout
            .and_then(|layout| layout.get("geometry"))
            .and_then(Value::as_str)
            != Some("regular")
        || layout
            .and_then(|layout| layout.get("order"))
            .and_then(Value::as_str)
            != Some("separate")
        || parameters
            .and_then(|parameters| parameters.get("order"))
            .and_then(Value::as_str)
            != Some("separate")
        || descriptor
            .pointer("/definition/earth/model")
            .and_then(Value::as_str)
            != Some("sphere")
    {
        return Err(invalid());
    }
    let parameter = |name: &str| parameters.and_then(|parameters| parameters.get(name));
    let nx = parameter("nx")
        .and_then(Value::as_u64)
        .ok_or_else(invalid)?;
    let ny = parameter("ny")
        .and_then(Value::as_u64)
        .ok_or_else(invalid)?;
    let nx = usize::try_from(nx).map_err(|_| invalid())?;
    let ny = usize::try_from(ny).map_err(|_| invalid())?;
    let latitude_origin = parameter("lat0")
        .and_then(Value::as_f64)
        .ok_or_else(invalid)?;
    let longitude_origin = parameter("lon0")
        .and_then(Value::as_f64)
        .ok_or_else(invalid)?;
    let latitude_step = parameter("dlat")
        .and_then(Value::as_f64)
        .ok_or_else(invalid)?;
    let longitude_step = parameter("dlon")
        .and_then(Value::as_f64)
        .ok_or_else(invalid)?;
    let earth_radius_m = crs
        .and_then(|crs| crs.pointer("/earth/radius_m"))
        .and_then(Value::as_f64)
        .ok_or_else(invalid)?;
    let latitude_axis = layout
        .and_then(|layout| layout.get("latitude_axis"))
        .and_then(Value::as_u64)
        .and_then(|axis| usize::try_from(axis).ok())
        .ok_or_else(invalid)?;
    let longitude_axis = layout
        .and_then(|layout| layout.get("longitude_axis"))
        .and_then(Value::as_u64)
        .and_then(|axis| usize::try_from(axis).ok())
        .ok_or_else(invalid)?;
    let shape = layout
        .and_then(|layout| layout.get("shape"))
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let axes = layout
        .and_then(|layout| layout.get("axes"))
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if nx == 0
        || ny == 0
        || !latitude_origin.is_finite()
        || !longitude_origin.is_finite()
        || !latitude_step.is_finite()
        || !longitude_step.is_finite()
        || latitude_step == 0.0
        || longitude_step == 0.0
        || !earth_radius_m.is_finite()
        || earth_radius_m <= 0.0
        || shape.len() != axes.len()
        || latitude_axis >= shape.len()
        || longitude_axis >= shape.len()
        || latitude_axis == longitude_axis
        || axes[latitude_axis].as_str() != Some("lat")
        || axes[longitude_axis].as_str() != Some("lon")
        || shape[latitude_axis].as_u64() != Some(ny as u64)
        || shape[longitude_axis].as_u64() != Some(nx as u64)
        || ((nx.saturating_sub(1) as f64) * longitude_step).abs() >= 360.0
    {
        return Err(invalid());
    }
    Ok(RegularGridSpec {
        nx,
        ny,
        latitude_origin,
        longitude_origin,
        latitude_step,
        longitude_step,
        earth_radius_m,
        latitude_axis,
        longitude_axis,
    })
}

pub fn validate_grid_descriptor(descriptor: &Value) -> Result<(), SeriesError> {
    regular_grid_from_descriptor(descriptor).map(|_| ())
}

pub fn query_grid_descriptor(
    connection: &duckdb::Connection,
    object_path: &Path,
) -> Result<Value, SeriesError> {
    let path = object_path.to_string_lossy();
    let sql = format!(
        "SELECT descriptor_version, grid_type, definition, layout, crs, capabilities, provenance FROM om_grid_info({}, dimensions := {}, grid := {}, spatial_axes := {})",
        quote_sql_literal(&path),
        DIMENSIONS_SQL,
        GRID_SQL,
        SPATIAL_AXES_SQL
    );
    connection
        .query_row(&sql, [], |row| {
            Ok(serde_json::json!({
                "descriptor_version": row.get::<_, i32>(0)?,
                "grid_type": row.get::<_, String>(1)?,
                "definition": serde_json::from_str::<Value>(&row.get::<_, String>(2)?).unwrap_or(Value::Null),
                "layout": serde_json::from_str::<Value>(&row.get::<_, String>(3)?).unwrap_or(Value::Null),
                "crs": serde_json::from_str::<Value>(&row.get::<_, String>(4)?).unwrap_or(Value::Null),
                "capabilities": serde_json::from_str::<Value>(&row.get::<_, String>(5)?).unwrap_or(Value::Null),
                "provenance": serde_json::from_str::<Value>(&row.get::<_, String>(6)?).unwrap_or(Value::Null),
            }))
        })
        .map_err(|error| SeriesError::DuckDb {
            stage: "grid_info",
            reason: error.to_string(),
        })
}
