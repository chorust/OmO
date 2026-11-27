//! Region-nearest Plan B. The engine loads spatial after configuration lock and reads each
//! temporal chunk through `read_om_regions(method := 'nearest')` before union/dedup.

use std::{collections::BTreeMap, path::Path, time::Instant};

use serde_json::json;

use crate::{
    BootstrappedConn,
    series::{
        CHUNK_TIME_LENGTH, DIMENSIONS_SQL, GRID_SQL, SERIES_VARIABLE, SPATIAL_AXES_SQL,
        ScanMeasurement, SeriesError, SeriesRun, Station, TIME_STEP_SECONDS, TIME_STEP_SQL,
        deduplicate_rows, normalize_longitude, query_grid_descriptor, quote_sql_literal,
        read_latest_scan_measurement, schema::StationSeriesRow, validate_grid_descriptor,
    },
};

const BUFFER_DEGREES: f64 = 0.0001;

pub fn run(
    connection: &mut BootstrappedConn,
    stations: &[Station],
    series_prefix: &Path,
    window_start_seconds: i64,
    window_end_seconds: i64,
) -> Result<SeriesRun, SeriesError> {
    let started = Instant::now();
    connection
        .ensure_spatial()
        .map_err(|error| SeriesError::DuckDb {
            stage: "ensure_spatial",
            reason: error.to_string(),
        })?;
    let object_path = series_prefix.join("chunk_0000.om");
    let grid_info = query_grid_descriptor(
        connection
            .connection()
            .map_err(|error| SeriesError::DuckDb {
                stage: "connection",
                reason: error.to_string(),
            })?,
        &object_path,
    )?;
    validate_grid_descriptor(&grid_info)?;
    let connection_ref = connection
        .connection()
        .map_err(|error| SeriesError::DuckDb {
            stage: "connection",
            reason: error.to_string(),
        })?;

    let chunk_indices = chunk_indices(window_start_seconds, window_end_seconds)?;
    let station_by_id = stations
        .iter()
        .map(|station| (station.station_id.as_str(), station))
        .collect::<BTreeMap<_, _>>();
    let mut rows = Vec::new();
    let mut chunk_measurements = Vec::new();
    for chunk_index in &chunk_indices {
        let chunk_path = series_prefix.join(format!("chunk_{chunk_index:04}.om"));
        let origin_seconds = chunk_index
            .checked_mul(
                CHUNK_TIME_LENGTH
                    .checked_mul(TIME_STEP_SECONDS)
                    .ok_or_else(|| SeriesError::InvalidInput("chunk span overflow".to_owned()))?,
            )
            .ok_or_else(|| SeriesError::InvalidInput("chunk origin overflow".to_owned()))?;
        let time_origin = timestamp_sql(origin_seconds)?;
        let regions = stations
            .iter()
            .map(region_sql)
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT result.region_id, strftime(result.valid_time, '%Y-%m-%dT%H:%M:%S') || 'Z', \
                    result.value::DOUBLE, result.sampling.source_lat, result.sampling.source_lon, \
                    result.sampling.distance_m, result.sampling.status, \
                    result.om_source.object_id, result.om_source.object_version \
             FROM read_om_regions({}, regions := [{}], region_crs := 'grid_lonlat', \
                    dimensions := {}, grid := {}, spatial_axes := {}, \
                    axes := {{'member':{{'axis':'member','values':[0::BIGINT,1::BIGINT]}}, \
                             'time':{{'axis':'time','start':TIMESTAMP {},'step':{}}}}}, \
                    method := 'nearest', out_of_bounds := 'null', include_source := true) AS result \
             WHERE result.\"member\" = 0 \
             ORDER BY result.region_id, result.valid_time",
            quote_sql_literal(&chunk_path.to_string_lossy()),
            regions,
            DIMENSIONS_SQL,
            GRID_SQL,
            SPATIAL_AXES_SQL,
            quote_sql_literal(&time_origin),
            TIME_STEP_SQL,
        );
        let mut statement = connection_ref
            .prepare(&sql)
            .map_err(|error| SeriesError::DuckDb {
                stage: "plan_b_prepare",
                reason: error.to_string(),
            })?;
        let result_rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<f64>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<f64>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })
            .map_err(|error| SeriesError::DuckDb {
                stage: "plan_b_execute",
                reason: error.to_string(),
            })?;
        for row in result_rows {
            let (
                station_id,
                valid_time_utc,
                value,
                grid_lat,
                grid_lon,
                distance_m,
                _status,
                source_object,
                object_version,
            ) = row.map_err(|error| SeriesError::DuckDb {
                stage: "plan_b_read",
                reason: error.to_string(),
            })?;
            if !station_by_id.contains_key(station_id.as_str()) {
                return Err(SeriesError::InvalidInput(format!(
                    "read_om_regions returned unknown region id {station_id}"
                )));
            }
            rows.push(StationSeriesRow {
                station_id,
                variable: SERIES_VARIABLE.to_owned(),
                valid_time_utc,
                value,
                grid_lat,
                grid_lon: grid_lon.map(normalize_longitude),
                distance_m,
                source_object,
                object_version,
            });
        }
        // Metrics are captured after this chunk has been fully consumed and before the next
        // chunk query can replace the connection's latest scan snapshot.
        chunk_measurements.push(read_latest_scan_measurement(connection_ref)?);
    }

    let (rows, duplicate_rows_removed) = deduplicate_rows(rows)?;
    let scan = combine_measurements(chunk_measurements);
    Ok(SeriesRun {
        rows,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        scan,
        duplicate_rows_removed,
        plan_details: json!({
            "grid_info": grid_info,
            "chunk_indices": chunk_indices,
            "region_count": stations.len(),
            "buffer_degrees": BUFFER_DEGREES,
            "geometry_representative": "point_on_surface_v1",
            "distance_model": "sphere_haversine_f64_v1"
        }),
    })
}

fn chunk_indices(start_seconds: i64, end_seconds: i64) -> Result<Vec<i64>, SeriesError> {
    if start_seconds > end_seconds {
        return Err(SeriesError::InvalidInput(
            "window start must not exceed window end".to_owned(),
        ));
    }
    let span = CHUNK_TIME_LENGTH
        .checked_mul(TIME_STEP_SECONDS)
        .ok_or_else(|| SeriesError::InvalidInput("chunk span overflow".to_owned()))?;
    let first = start_seconds.div_euclid(span);
    let last = end_seconds.div_euclid(span);
    let count = last - first + 1;
    if count <= 0 || count > 4096 {
        return Err(SeriesError::InvalidInput(format!(
            "window expands to {count} chunks; maximum is 4096"
        )));
    }
    Ok((first..=last).collect())
}

fn region_sql(station: &Station) -> String {
    let normalized_longitude = normalize_longitude(station.longitude);
    let min_lon = normalized_longitude - BUFFER_DEGREES;
    let max_lon = normalized_longitude + BUFFER_DEGREES;
    let min_lat = station.latitude - BUFFER_DEGREES;
    let max_lat = station.latitude + BUFFER_DEGREES;
    let wkt = format!(
        "POLYGON(({min_lon:.17} {min_lat:.17},{max_lon:.17} {min_lat:.17},{max_lon:.17} {max_lat:.17},{min_lon:.17} {max_lat:.17},{min_lon:.17} {min_lat:.17}))"
    );
    format!(
        "{{'id':{},'wkb':ST_AsWKB(ST_GeomFromText({}))}}",
        quote_sql_literal(&station.station_id),
        quote_sql_literal(&wkt)
    )
}

fn timestamp_sql(seconds_since_epoch: i64) -> Result<String, SeriesError> {
    if seconds_since_epoch % 86_400 == 0 {
        let days = seconds_since_epoch.div_euclid(86_400);
        let civil = omo_protocol::utc::civil_from_days(days);
        return Ok(format!(
            "{:04}-{:02}-{:02} 00:00:00",
            civil.0, civil.1, civil.2
        ));
    }
    if seconds_since_epoch < 0 || seconds_since_epoch > 86_400 * 36_500 {
        return Err(SeriesError::InvalidInput(
            "Plan B fixture timestamp is outside the supported formatting range".to_owned(),
        ));
    }
    let days = seconds_since_epoch.div_euclid(86_400);
    let seconds_in_day = seconds_since_epoch.rem_euclid(86_400);
    let (year, month, day) = omo_protocol::utc::civil_from_days(days);
    let hour = seconds_in_day / 3_600;
    let minute = (seconds_in_day % 3_600) / 60;
    let second = seconds_in_day % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"
    ))
}

/// Plan B on an eligible regular-grid remote OM domain using `read_om_regions(method := 'nearest')`.
#[allow(clippy::too_many_arguments)]
pub fn run_remote_domain(
    conn: &mut BootstrappedConn,
    stations: &[crate::series::plan_g::SeriesStation],
    variable: &str,
    domain: &str,
    object_url: &str,
    object_version: &str,
    max_distance_m: f64,
    time_index: &[(usize, String)],
) -> Result<
    (
        crate::series::plan_g::PlanGOutput,
        crate::series::plan_g::PlanGScanStats,
    ),
    SeriesError,
> {
    use crate::series::plan_g::{
        MatchStatus, NoMatchReason, PlanGOutput, PlanGScanStats, StationMatch,
    };

    conn.ensure_spatial().map_err(|error| SeriesError::DuckDb {
        stage: "ensure_spatial",
        reason: error.to_string(),
    })?;
    let mut matches = Vec::with_capacity(stations.len());
    let mut valid_stations = Vec::new();
    for station in stations {
        if !station.latitude.is_finite()
            || !station.longitude.is_finite()
            || !(-90.0..=90.0).contains(&station.latitude)
        {
            matches.push(StationMatch {
                station_id: station.station_id.clone(),
                status: MatchStatus::NoMatch,
                flat_index: None,
                grid_lat: None,
                grid_lon: None,
                distance_m: None,
                reason: Some(NoMatchReason::InvalidCoordinate),
            });
            continue;
        }
        valid_stations.push(Station {
            station_id: station.station_id.clone(),
            latitude: station.latitude,
            longitude: station.longitude,
        });
    }
    if valid_stations.is_empty() {
        return Ok((
            PlanGOutput {
                rows: Vec::new(),
                matches,
            },
            PlanGScanStats::default(),
        ));
    }

    let https = crate::catalog::open_meteo_https(object_url);
    let regions = valid_stations
        .iter()
        .map(region_sql)
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT result.region_id, strftime(result.valid_time, '%Y-%m-%dT%H:%M:%S') || 'Z', \
                result.value::DOUBLE, result.sampling.source_lat, result.sampling.source_lon, \
                result.sampling.distance_m, result.sampling.status \
         FROM read_om_regions({}, domain := {}, regions := [{}], region_crs := 'grid_lonlat', \
                method := 'nearest', out_of_bounds := 'null', include_source := true) AS result \
         ORDER BY result.region_id, result.valid_time",
        quote_sql_literal(&https),
        quote_sql_literal(domain),
        regions,
    );
    let db = conn.connection().map_err(|error| SeriesError::DuckDb {
        stage: "plan_b_remote_conn",
        reason: error.to_string(),
    })?;
    let mut statement = db.prepare(&sql).map_err(|error| SeriesError::DuckDb {
        stage: "plan_b_remote_prepare",
        reason: error.to_string(),
    })?;
    let query_rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<f64>>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, Option<f64>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|error| SeriesError::DuckDb {
            stage: "plan_b_remote_execute",
            reason: error.to_string(),
        })?;

    let allowed_times: std::collections::BTreeSet<&str> =
        time_index.iter().map(|(_, ts)| ts.as_str()).collect();
    let mut rows = Vec::new();
    let mut matched_by_id: BTreeMap<String, StationMatch> = BTreeMap::new();
    for item in query_rows {
        let (station_id, valid_time, value, source_lat, source_lon, distance_m, status) = item
            .map_err(|error| SeriesError::DuckDb {
                stage: "plan_b_remote_row",
                reason: error.to_string(),
            })?;
        if !allowed_times.is_empty() && !allowed_times.contains(valid_time.as_str()) {
            continue;
        }
        let dist = distance_m.unwrap_or(f64::INFINITY);
        if status != "ok" || !dist.is_finite() {
            matched_by_id
                .entry(station_id.clone())
                .or_insert(StationMatch {
                    station_id,
                    status: MatchStatus::NoMatch,
                    flat_index: None,
                    grid_lat: source_lat,
                    grid_lon: source_lon,
                    distance_m,
                    reason: Some(NoMatchReason::MissingSourceData),
                });
            continue;
        }
        if dist > max_distance_m {
            matched_by_id
                .entry(station_id.clone())
                .or_insert(StationMatch {
                    station_id,
                    status: MatchStatus::NoMatch,
                    flat_index: None,
                    grid_lat: source_lat,
                    grid_lon: source_lon,
                    distance_m: Some(dist),
                    reason: Some(NoMatchReason::BeyondMaxDistance),
                });
            continue;
        }
        matched_by_id
            .entry(station_id.clone())
            .or_insert(StationMatch {
                station_id: station_id.clone(),
                status: MatchStatus::Matched,
                flat_index: None,
                grid_lat: source_lat,
                grid_lon: source_lon,
                distance_m: Some(dist),
                reason: None,
            });
        rows.push(StationSeriesRow {
            station_id,
            variable: variable.to_owned(),
            valid_time_utc: valid_time,
            value,
            grid_lat: source_lat,
            grid_lon: source_lon,
            distance_m: Some(dist),
            source_object: Some(object_url.to_owned()),
            object_version: Some(object_version.to_owned()),
        });
    }
    for station in &valid_stations {
        if let Some(found) = matched_by_id.remove(&station.station_id) {
            matches.push(found);
        } else {
            matches.push(StationMatch {
                station_id: station.station_id.clone(),
                status: MatchStatus::NoMatch,
                flat_index: None,
                grid_lat: None,
                grid_lon: None,
                distance_m: None,
                reason: Some(NoMatchReason::MissingSourceData),
            });
        }
    }
    let (deduped, _) = deduplicate_rows(rows)?;
    let scan_stats = match read_latest_scan_measurement(db) {
        Ok(measurement) => PlanGScanStats {
            bytes_fetched: measurement
                .raw_metrics
                .pointer("/legacy_v2/bytes_fetched")
                .and_then(serde_json::Value::as_u64)
                .or(measurement.physical_read_bytes)
                .or(measurement.data_bytes)
                .unwrap_or(0),
            read_requests: measurement
                .raw_metrics
                .pointer("/legacy_v2/read_requests")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            data_bytes: measurement.data_bytes.unwrap_or(0),
        },
        Err(_) => PlanGScanStats::default(),
    };
    Ok((
        PlanGOutput {
            rows: deduped,
            matches,
        },
        scan_stats,
    ))
}

fn combine_measurements(measurements: Vec<ScanMeasurement>) -> ScanMeasurement {
    let sum = |select: fn(&ScanMeasurement) -> Option<u64>| {
        measurements.iter().try_fold(0u64, |total, measurement| {
            select(measurement).map(|bytes| total.saturating_add(bytes))
        })
    };
    let scan_complete = if measurements
        .iter()
        .any(|measurement| measurement.scan_complete == Some(false))
    {
        Some(false)
    } else if !measurements.is_empty()
        && measurements
            .iter()
            .all(|measurement| measurement.scan_complete == Some(true))
    {
        Some(true)
    } else {
        None
    };
    let last = measurements.last();
    ScanMeasurement {
        scan_id: last.and_then(|measurement| measurement.scan_id),
        operation: Some("read_om_regions".to_owned()),
        data_bytes: sum(|measurement| measurement.data_bytes),
        physical_read_bytes: sum(|measurement| measurement.physical_read_bytes),
        metadata_bytes: sum(|measurement| measurement.metadata_bytes),
        index_bytes: sum(|measurement| measurement.index_bytes),
        scan_complete,
        raw_metrics: json!({ "per_chunk": measurements }),
    }
}
