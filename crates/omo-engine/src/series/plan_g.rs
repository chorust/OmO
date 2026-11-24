//! Plan G: nearest O1280 point in Rust, then a selective `read_om` of those points.
//!
//! Research R-01. The coordinate cross-check fails the request with `GridMismatch`
//! instead of falling back to another grid. No `s3://`-scoped DuckDB secret is created;
//! remote reads use the public HTTPS form of the object URL.

use crate::grid::reduced_gaussian::{Nearest, O1280};
use crate::series::schema::{STATION_SERIES_COLUMNS, StationSeriesRow};
use crate::series::{SeriesError, normalize_longitude};

pub const BOX_HALF_WIDTH_DEG: f64 = 1e-4;

#[derive(Clone, Debug, PartialEq)]
pub struct SelectedPoint {
    pub flat_index: u32,
    pub lat: f32,
    pub lon: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointRow {
    pub lat: f32,
    pub lon: f32,
    pub time_index: usize,
    pub valid_time: Option<String>,
    pub value: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeriesStation {
    pub station_id: String,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatchStatus {
    Matched,
    NoMatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoMatchReason {
    InvalidCoordinate,
    BeyondMaxDistance,
    MissingSourceData,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StationMatch {
    pub station_id: String,
    pub status: MatchStatus,
    pub flat_index: Option<u32>,
    pub grid_lat: Option<f64>,
    pub grid_lon: Option<f64>,
    pub distance_m: Option<f64>,
    pub reason: Option<NoMatchReason>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanGOutput {
    pub rows: Vec<StationSeriesRow>,
    pub matches: Vec<StationMatch>,
}

use crate::results::write_arrow_batches;
use arrow::array::{ArrayRef, Float64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use omo_protocol::result_ref::{ResultLimits, ResultRef};
use std::{path::Path, sync::Arc};

pub fn write_matches_arrow(
    matches: &[StationMatch],
    generation: u64,
    limits: ResultLimits,
    session_tmp: &Path,
    result_id: &str,
) -> Result<ResultRef, SeriesError> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("station_id", DataType::Utf8, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("flat_index", DataType::UInt32, true),
        Field::new("grid_lat", DataType::Float64, true),
        Field::new("grid_lon", DataType::Float64, true),
        Field::new("distance_m", DataType::Float64, true),
        Field::new("reason", DataType::Utf8, true),
    ]));
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from_iter_values(
            matches.iter().map(|m| m.station_id.as_str()),
        )),
        Arc::new(StringArray::from_iter_values(matches.iter().map(
            |m| match m.status {
                MatchStatus::Matched => "Matched",
                MatchStatus::NoMatch => "NoMatch",
            },
        ))),
        Arc::new(arrow::array::UInt32Array::from_iter(
            matches.iter().map(|m| m.flat_index),
        )),
        Arc::new(Float64Array::from_iter(matches.iter().map(|m| m.grid_lat))),
        Arc::new(Float64Array::from_iter(matches.iter().map(|m| m.grid_lon))),
        Arc::new(Float64Array::from_iter(
            matches.iter().map(|m| m.distance_m),
        )),
        Arc::new(StringArray::from_iter(matches.iter().map(|m| {
            m.reason.map(|r| match r {
                NoMatchReason::InvalidCoordinate => "InvalidCoordinate",
                NoMatchReason::BeyondMaxDistance => "BeyondMaxDistance",
                NoMatchReason::MissingSourceData => "MissingSourceData",
            })
        }))),
    ];
    let batch = RecordBatch::try_new(Arc::clone(&schema), columns).map_err(|error| {
        SeriesError::InvalidInput(format!("cannot build matches Arrow batch: {error}"))
    })?;
    let batches = if matches.is_empty() {
        Vec::new()
    } else {
        vec![Ok::<RecordBatch, String>(batch)]
    };
    write_arrow_batches(
        schema,
        batches,
        session_tmp,
        &format!("{result_id}-matches"),
        generation,
        limits,
    )
    .map_err(|error| SeriesError::InvalidInput(error.to_string()))
}

pub fn write_series_parquet(
    conn: &crate::conn::BootstrappedConn,
    rows: &[super::schema::StationSeriesRow],
    generation: u64,
    session_tmp: &Path,
    result_id: &str,
) -> Result<std::path::PathBuf, SeriesError> {
    let gen_dir = session_tmp.join(format!("g{generation}"));
    std::fs::create_dir_all(&gen_dir)?;
    let csv_tmp = gen_dir.join(format!(".{result_id}-series.csv.tmp"));
    let parquet_path = gen_dir.join(format!("{result_id}.parquet"));
    super::schema::write_csv(&csv_tmp, rows)?;
    let escaped_csv = super::quote_sql_literal(&csv_tmp.to_string_lossy());
    let escaped_parquet = super::quote_sql_literal(&parquet_path.to_string_lossy());
    let sql = format!(
        "COPY (SELECT station_id::VARCHAR AS station_id, variable::VARCHAR AS variable, \
         valid_time::VARCHAR AS valid_time, value::DOUBLE AS value, \
         grid_lat::DOUBLE AS grid_lat, grid_lon::DOUBLE AS grid_lon, \
         distance_m::DOUBLE AS distance_m, source_object::VARCHAR AS source_object, \
         object_version::VARCHAR AS object_version \
         FROM read_csv({escaped_csv}, header=true, types={{'station_id':'VARCHAR','variable':'VARCHAR','valid_time':'VARCHAR','value':'DOUBLE','grid_lat':'DOUBLE','grid_lon':'DOUBLE','distance_m':'DOUBLE','source_object':'VARCHAR','object_version':'VARCHAR'}})) \
         TO {escaped_parquet} (FORMAT PARQUET)"
    );
    let result = conn
        .connection()
        .map_err(|error| SeriesError::DuckDb {
            stage: "series_parquet_conn",
            reason: error.to_string(),
        })?
        .execute_batch(&sql)
        .map_err(|error| SeriesError::DuckDb {
            stage: "series_parquet_copy",
            reason: error.to_string(),
        });
    let _ = std::fs::remove_file(&csv_tmp);
    result?;
    Ok(parquet_path)
}

pub trait PointReader {
    fn read_points(
        &mut self,
        object_url: &str,
        points: &[SelectedPoint],
    ) -> Result<Vec<PointRow>, SeriesError>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanGScanStats {
    pub bytes_fetched: u64,
    pub read_requests: u64,
    pub data_bytes: u64,
}

/// Parallel per-station Plan G reader backed by `BootstrappedConn` worker connections.
/// S1 proved that single-point `read_om` boxes push down selectively (~1.95 KB/point),
/// whereas multi-box OR queries scan all 6.6M grid points.
pub struct ParallelDuckDbPointReader<'a> {
    pub config: &'a crate::BootstrapConfig,
    pub domain: &'a str,
    pub max_workers: usize,
    pub cancel: Option<Arc<crate::series::SeriesCancel>>,
    pub scan_stats: PlanGScanStats,
}

impl PointReader for ParallelDuckDbPointReader<'_> {
    fn read_points(
        &mut self,
        object_url: &str,
        points: &[SelectedPoint],
    ) -> Result<Vec<PointRow>, SeriesError> {
        if points.is_empty() {
            return Ok(Vec::new());
        }
        // Deduplicate identical grid points so multiple stations sharing one cell read once.
        let mut unique_points: Vec<SelectedPoint> = Vec::new();
        for point in points {
            if !unique_points.iter().any(|existing| {
                existing.lat.to_bits() == point.lat.to_bits()
                    && existing.lon.to_bits() == point.lon.to_bits()
            }) {
                unique_points.push(point.clone());
            }
        }
        let workers = self.max_workers.max(1).min(unique_points.len());
        let chunk_size = unique_points.len().div_ceil(workers);
        let https = crate::catalog::open_meteo_https(object_url);
        let https_ref = https.as_str();
        let domain = self.domain;
        let config = self.config;
        let cancel = self.cancel.clone();
        if let Some(cancel) = &cancel {
            cancel.check()?;
        }

        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(workers);
            for batch in unique_points.chunks(chunk_size) {
                let cancel = cancel.clone();
                handles.push(scope.spawn(move || -> Result<(Vec<PointRow>, PlanGScanStats), SeriesError> {
                    if let Some(cancel) = &cancel {
                        cancel.check()?;
                    }
                    let conn = crate::open_bootstrapped(config.clone()).map_err(|error| {
                        SeriesError::DuckDb {
                            stage: "plan_g_worker_open",
                            reason: error.to_string(),
                        }
                    })?;
                    let db = conn.connection().map_err(|error| SeriesError::DuckDb {
                        stage: "plan_g_worker_conn",
                        reason: error.to_string(),
                    })?;
                    if let Some(cancel) = &cancel {
                        cancel.register(conn.interrupt_handle().map_err(|error| {
                            SeriesError::DuckDb {
                                stage: "plan_g_worker_interrupt",
                                reason: error.to_string(),
                            }
                        })?);
                    }
                    let mut out = Vec::new();
                    let mut stats = PlanGScanStats::default();
                    for point in batch {
                        if let Some(cancel) = &cancel {
                            cancel.check()?;
                        }
                        let sql = read_om_sql(https_ref, domain, std::slice::from_ref(point));
                        let mut stmt = db.prepare(&sql).map_err(|error| SeriesError::DuckDb {
                            stage: "plan_g_prepare",
                            reason: error.to_string(),
                        })?;
                        let rows = stmt
                            .query_map([], |row| {
                                Ok(PointRow {
                                    lat: row.get::<_, f32>(0)?,
                                    lon: row.get::<_, f32>(1)?,
                                    time_index: 0,
                                    valid_time: row.get::<_, Option<String>>(2)?,
                                    value: row.get::<_, Option<f64>>(3)?,
                                })
                            })
                            .map_err(|error| SeriesError::DuckDb {
                                stage: "plan_g_query",
                                reason: error.to_string(),
                            })?;
                        for row in rows {
                            out.push(row.map_err(|error| SeriesError::DuckDb {
                                stage: "plan_g_read",
                                reason: error.to_string(),
                            })?);
                        }
                        if let Ok(measurement) = crate::series::read_latest_scan_measurement(db) {
                            stats.bytes_fetched = stats.bytes_fetched.saturating_add(
                                measurement
                                    .raw_metrics
                                    .pointer("/legacy_v2/bytes_fetched")
                                    .and_then(serde_json::Value::as_u64)
                                    .or(measurement.physical_read_bytes)
                                    .or(measurement.data_bytes)
                                    .unwrap_or(0),
                            );
                            stats.read_requests = stats.read_requests.saturating_add(
                                measurement
                                    .raw_metrics
                                    .pointer("/legacy_v2/read_requests")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0),
                            );
                            stats.data_bytes = stats
                                .data_bytes
                                .saturating_add(measurement.data_bytes.unwrap_or(0));
                        }
                    }
                    Ok((out, stats))
                }));
            }
            let mut merged = Vec::new();
            let mut total_stats = PlanGScanStats::default();
            for handle in handles {
                let (part, stats) = handle.join().map_err(|_| SeriesError::DuckDb {
                    stage: "plan_g_worker_join",
                    reason: "plan G reader thread panicked".into(),
                })??;
                merged.extend(part);
                total_stats.bytes_fetched =
                    total_stats.bytes_fetched.saturating_add(stats.bytes_fetched);
                total_stats.read_requests =
                    total_stats.read_requests.saturating_add(stats.read_requests);
                total_stats.data_bytes = total_stats.data_bytes.saturating_add(stats.data_bytes);
            }
            self.scan_stats.bytes_fetched = self
                .scan_stats
                .bytes_fetched
                .saturating_add(total_stats.bytes_fetched);
            self.scan_stats.read_requests = self
                .scan_stats
                .read_requests
                .saturating_add(total_stats.read_requests);
            self.scan_stats.data_bytes = self
                .scan_stats
                .data_bytes
                .saturating_add(total_stats.data_bytes);
            Ok(merged)
        })
    }
}

/// OR of tight lat/lon boxes. Boxes that cross ±180° are split so the antimeridian matches.
pub fn build_point_predicate(points: &[SelectedPoint]) -> String {
    if points.is_empty() {
        return "FALSE".to_owned();
    }
    points
        .iter()
        .map(|point| box_clause(f64::from(point.lat), f64::from(point.lon)))
        .collect::<Vec<_>>()
        .join(" OR ")
}

pub fn verify_exact(points: &[SelectedPoint], rows: &[PointRow]) -> Result<(), SeriesError> {
    for point in points {
        let mut coordinates = Vec::new();
        for row in rows {
            if !near(row.lat, point.lat) || !near(row.lon, point.lon) {
                continue;
            }
            let key = (row.lat.to_bits(), row.lon.to_bits());
            if !coordinates.contains(&key) {
                coordinates.push(key);
            }
        }
        let exact = (point.lat.to_bits(), point.lon.to_bits());
        if coordinates.len() != 1 || coordinates.first() != Some(&exact) {
            return Err(SeriesError::GridMismatch(format!(
                "point {} returned {} coordinates; expected exactly one Float32 match",
                point.flat_index,
                coordinates.len()
            )));
        }
    }
    Ok(())
}

/// Nearest-point selection, read, time join, and the C-10 `station_series` rows.
#[allow(clippy::too_many_arguments)]
pub fn assemble(
    stations: &[SeriesStation],
    variable: &str,
    source_object: &str,
    object_version: &str,
    max_distance_m: f64,
    time_index: &[(usize, String)],
    object_url: &str,
    reader: &mut dyn PointReader,
) -> Result<PlanGOutput, SeriesError> {
    let mut matches = Vec::with_capacity(stations.len());
    let mut selected: Vec<(String, Nearest)> = Vec::new();
    for station in stations {
        if !station.latitude.is_finite()
            || !station.longitude.is_finite()
            || !(-90.0..=90.0).contains(&station.latitude)
        {
            matches.push(no_match(
                &station.station_id,
                NoMatchReason::InvalidCoordinate,
            ));
            continue;
        }
        let nearest = O1280::nearest(station.latitude, station.longitude);
        if nearest.distance_m > max_distance_m {
            let mut row = no_match(&station.station_id, NoMatchReason::BeyondMaxDistance);
            row.flat_index = Some(nearest.flat_index);
            row.grid_lat = Some(f64::from(nearest.lat));
            row.grid_lon = Some(f64::from(nearest.lon));
            row.distance_m = Some(nearest.distance_m);
            matches.push(row);
            continue;
        }
        selected.push((station.station_id.clone(), nearest));
        matches.push(StationMatch {
            station_id: station.station_id.clone(),
            status: MatchStatus::Matched,
            flat_index: Some(nearest.flat_index),
            grid_lat: Some(f64::from(nearest.lat)),
            grid_lon: Some(f64::from(nearest.lon)),
            distance_m: Some(nearest.distance_m),
            reason: None,
        });
    }

    let points = selected
        .iter()
        .map(|(_, nearest)| SelectedPoint {
            flat_index: nearest.flat_index,
            lat: nearest.lat,
            lon: nearest.lon,
        })
        .collect::<Vec<_>>();
    let read = if points.is_empty() {
        Vec::new()
    } else {
        let rows = reader.read_points(object_url, &points)?;
        verify_exact(&points, &rows)?;
        rows
    };

    let mut series_rows = Vec::new();
    for (station_id, nearest) in &selected {
        let point_rows = read.iter().filter(|row| {
            row.lat.to_bits() == nearest.lat.to_bits() && row.lon.to_bits() == nearest.lon.to_bits()
        });
        let mut saw_value = false;
        for row in point_rows {
            let Some(valid_time) = resolve_valid_time(row, time_index) else {
                continue;
            };
            saw_value = true;
            series_rows.push(StationSeriesRow {
                station_id: station_id.clone(),
                variable: variable.to_owned(),
                valid_time_utc: valid_time.clone(),
                value: row.value,
                grid_lat: Some(f64::from(nearest.lat)),
                grid_lon: Some(normalize_longitude(f64::from(nearest.lon))),
                distance_m: Some(nearest.distance_m),
                source_object: Some(source_object.to_owned()),
                object_version: Some(object_version.to_owned()),
            });
        }
        if !saw_value
            && let Some(entry) = matches
                .iter_mut()
                .find(|entry| entry.station_id == *station_id)
        {
            entry.status = MatchStatus::NoMatch;
            entry.reason = Some(NoMatchReason::MissingSourceData);
        }
    }
    series_rows.sort_by(|left, right| {
        left.station_id
            .cmp(&right.station_id)
            .then_with(|| left.valid_time_utc.cmp(&right.valid_time_utc))
    });
    for row in &series_rows {
        row.validate()?;
    }
    let _ = STATION_SERIES_COLUMNS;
    Ok(PlanGOutput {
        rows: series_rows,
        matches,
    })
}

fn no_match(station_id: &str, reason: NoMatchReason) -> StationMatch {
    StationMatch {
        station_id: station_id.to_owned(),
        status: MatchStatus::NoMatch,
        flat_index: None,
        grid_lat: None,
        grid_lon: None,
        distance_m: None,
        reason: Some(reason),
    }
}

fn near(found: f32, expected: f32) -> bool {
    (f64::from(found) - f64::from(expected)).abs() <= BOX_HALF_WIDTH_DEG
}

fn box_clause(lat: f64, lon: f64) -> String {
    let lat_lo = lat - BOX_HALF_WIDTH_DEG;
    let lat_hi = lat + BOX_HALF_WIDTH_DEG;
    let lon_lo = lon - BOX_HALF_WIDTH_DEG;
    let lon_hi = lon + BOX_HALF_WIDTH_DEG;
    let lon_clause = if lon_lo < -180.0 {
        format!(
            "(lon BETWEEN -180 AND {lon_hi:.8} OR lon BETWEEN {:.8} AND 180)",
            lon_lo + 360.0
        )
    } else if lon_hi > 180.0 {
        format!(
            "(lon BETWEEN {lon_lo:.8} AND 180 OR lon BETWEEN -180 AND {:.8})",
            lon_hi - 360.0
        )
    } else if (lon - -180.0).abs() <= BOX_HALF_WIDTH_DEG
        || (lon - 180.0).abs() <= BOX_HALF_WIDTH_DEG
    {
        format!(
            "(lon BETWEEN -180 AND {:.8} OR lon BETWEEN {:.8} AND 180)",
            (-180.0 + BOX_HALF_WIDTH_DEG).max(lon_hi),
            (180.0 - BOX_HALF_WIDTH_DEG).min(lon_lo)
        )
    } else {
        format!("lon BETWEEN {lon_lo:.8} AND {lon_hi:.8}")
    };
    format!("(lat BETWEEN {lat_lo:.8} AND {lat_hi:.8} AND {lon_clause})")
}

/// SQL for the selective read. Column names follow research R-01 and are confirmed by S1.
pub fn read_om_sql(object_https: &str, domain: &str, points: &[SelectedPoint]) -> String {
    // S1 on DuckDB 2.0 / duckomo v0.4.0: `read_om(domain := 'ecmwf_ifs')` returns
    // `value`, `lat`, `lon`, and `valid_time` (TIMESTAMP). There is no time-index column.
    format!(
        "SELECT lat::FLOAT AS lat, lon::FLOAT AS lon, \
                strftime(valid_time, '%Y-%m-%dT%H:%M:%S') || 'Z' AS valid_time, \
                value::DOUBLE AS value \
         FROM read_om({}, domain := {}) WHERE {}",
        super::quote_sql_literal(object_https),
        super::quote_sql_literal(domain),
        build_point_predicate(points)
    )
}

fn resolve_valid_time(row: &PointRow, time_index: &[(usize, String)]) -> Option<String> {
    if let Some(valid_time) = &row.valid_time {
        return time_index
            .iter()
            .find(|(_, expected)| timestamps_equal(expected, valid_time))
            .map(|(_, expected)| expected.clone());
    }
    time_index
        .iter()
        .find(|(index, _)| *index == row.time_index)
        .map(|(_, valid_time)| valid_time.clone())
}

fn timestamps_equal(left: &str, right: &str) -> bool {
    let digits = |value: &str| {
        value
            .chars()
            .filter(|ch| ch.is_ascii_digit())
            .take(12)
            .collect::<String>()
    };
    digits(left) == digits(right)
}

pub struct DuckDbPointReader<'a> {
    pub conn: &'a crate::conn::BootstrappedConn,
    pub domain: &'a str,
}

impl PointReader for DuckDbPointReader<'_> {
    fn read_points(
        &mut self,
        object_url: &str,
        points: &[SelectedPoint],
    ) -> Result<Vec<PointRow>, SeriesError> {
        let https = crate::catalog::open_meteo_https(object_url);
        let db = self.conn.connection().map_err(|error| SeriesError::DuckDb {
            stage: "plan_g_conn",
            reason: error.to_string(),
        })?;
        let sql = read_om_sql(&https, self.domain, points);
        let mut statement = db
            .prepare(&sql)
            .map_err(|error| SeriesError::DuckDb {
                stage: "plan_g_prepare",
                reason: error.to_string(),
            })?;
        let rows = statement
            .query_map([], |row| {
                Ok(PointRow {
                    lat: row.get::<_, f32>(0)?,
                    lon: row.get::<_, f32>(1)?,
                    time_index: 0,
                    valid_time: row.get::<_, Option<String>>(2)?,
                    value: row.get::<_, Option<f64>>(3)?,
                })
            })
            .map_err(|error| SeriesError::DuckDb {
                stage: "plan_g_query",
                reason: error.to_string(),
            })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| SeriesError::DuckDb {
                stage: "plan_g_read",
                reason: error.to_string(),
            })
    }
}

/// Content-free object token (size and last-modified). A change between the pre-read
/// and post-read tokens is `SourceChanged`. DuckDB's httpfs HEAD carries the ETag
/// internally; this token is what the locked httpfs SQL surface exposes.
pub fn object_version_token(
    conn: &crate::conn::BootstrappedConn,
    object_url: &str,
) -> Result<String, SeriesError> {
    let https = crate::catalog::open_meteo_https(object_url);
    let db = conn.connection().map_err(|error| SeriesError::DuckDb {
        stage: "object_version_conn",
        reason: error.to_string(),
    })?;
    let sql = format!(
        "SELECT printf('%s|%s', size::VARCHAR, COALESCE(strftime(last_modified, '%Y-%m-%dT%H:%M:%SZ'), '')) \
         FROM read_text({})",
        super::quote_sql_literal(&https)
    );
    db.query_row(&sql, [], |row| row.get::<_, String>(0))
        .map_err(|error| SeriesError::DuckDb {
            stage: "object_version",
            reason: error.to_string(),
        })
}

/// Describe the columns returned by `read_om(object_url, domain := domain)`.
pub fn describe_om_columns(
    conn: &crate::conn::BootstrappedConn,
    object_url: &str,
    domain: &str,
) -> Result<Vec<serde_json::Value>, SeriesError> {
    let https = crate::catalog::open_meteo_https(object_url);
    let db = conn.connection().map_err(|error| SeriesError::DuckDb {
        stage: "describe_om_conn",
        reason: error.to_string(),
    })?;
    let sql = format!(
        "DESCRIBE SELECT * FROM read_om({}, domain := {}) LIMIT 0",
        super::quote_sql_literal(&https),
        super::quote_sql_literal(domain),
    );
    let mut stmt = db.prepare(&sql).map_err(|error| SeriesError::DuckDb {
        stage: "describe_om_prepare",
        reason: error.to_string(),
    })?;
    let rows = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "column_name": row.get::<_, String>(0)?,
                "column_type": row.get::<_, String>(1)?,
            }))
        })
        .map_err(|error| SeriesError::DuckDb {
            stage: "describe_om_query",
            reason: error.to_string(),
        })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| SeriesError::DuckDb {
            stage: "describe_om_read",
            reason: error.to_string(),
        })
}

/// Query the distinct `(lat, lon)` Float32 coordinates returned by duckomo for one point box.
pub fn probe_om_point_coordinates(
    conn: &crate::conn::BootstrappedConn,
    object_url: &str,
    domain: &str,
    point: &SelectedPoint,
) -> Result<Vec<(f32, f32)>, SeriesError> {
    let https = crate::catalog::open_meteo_https(object_url);
    let db = conn.connection().map_err(|error| SeriesError::DuckDb {
        stage: "probe_point_conn",
        reason: error.to_string(),
    })?;
    let sql = format!(
        "SELECT DISTINCT lat::FLOAT, lon::FLOAT FROM read_om({}, domain := {}) WHERE {}",
        super::quote_sql_literal(&https),
        super::quote_sql_literal(domain),
        build_point_predicate(std::slice::from_ref(point)),
    );
    let mut stmt = db.prepare(&sql).map_err(|error| SeriesError::DuckDb {
        stage: "probe_point_prepare",
        reason: error.to_string(),
    })?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, f32>(0)?, row.get::<_, f32>(1)?)))
        .map_err(|error| SeriesError::DuckDb {
            stage: "probe_point_query",
            reason: error.to_string(),
        })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| SeriesError::DuckDb {
            stage: "probe_point_read",
            reason: error.to_string(),
        })
}

/// Fallback regions-mode point probe using `read_om_regions(method := 'points')`.
pub fn read_om_regions_points(
    conn: &crate::conn::BootstrappedConn,
    object_url: &str,
    domain: &str,
    points: &[SelectedPoint],
) -> Result<Vec<(f32, f32)>, SeriesError> {
    let https = crate::catalog::open_meteo_https(object_url);
    let db = conn.connection().map_err(|error| SeriesError::DuckDb {
        stage: "regions_points_conn",
        reason: error.to_string(),
    })?;
    let regions = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let lat = f64::from(point.lat);
            let lon = f64::from(point.lon);
            let lat_min = (lat - BOX_HALF_WIDTH_DEG).max(-90.0);
            let lat_max = (lat + BOX_HALF_WIDTH_DEG).min(90.0);
            let lon_min = (lon - BOX_HALF_WIDTH_DEG).max(-180.0);
            let lon_max = (lon + BOX_HALF_WIDTH_DEG).min(180.0);
            let wkt = format!(
                "POLYGON(({lon_min:.6} {lat_min:.6}, {lon_max:.6} {lat_min:.6}, \
                          {lon_max:.6} {lat_max:.6}, {lon_min:.6} {lat_max:.6}, \
                          {lon_min:.6} {lat_min:.6}))"
            );
            format!(
                "{{'id': 'p{index}', 'geom': ST_GeomFromText({})}}",
                super::quote_sql_literal(&wkt)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT lat::FLOAT, lon::FLOAT FROM read_om_regions({}, domain := {}, \
         regions := [{regions}], method := 'points')",
        super::quote_sql_literal(&https),
        super::quote_sql_literal(domain),
    );
    let mut stmt = db.prepare(&sql).map_err(|error| SeriesError::DuckDb {
        stage: "regions_points_prepare",
        reason: error.to_string(),
    })?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, f32>(0)?, row.get::<_, f32>(1)?)))
        .map_err(|error| SeriesError::DuckDb {
            stage: "regions_points_query",
            reason: error.to_string(),
        })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| SeriesError::DuckDb {
            stage: "regions_points_read",
            reason: error.to_string(),
        })
}

pub fn ensure_stable_version(before: &str, after: &str) -> Result<(), SeriesError> {
    if before != after {
        Err(SeriesError::SourceChanged(format!(
            "object version changed from {before} to {after}"
        )))
    } else {
        Ok(())
    }
}
