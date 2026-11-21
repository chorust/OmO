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
                grid_lat: grid_lat,
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
        let civil = civil_from_days(days);
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
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_in_day / 3_600;
    let minute = (seconds_in_day % 3_600) / 60;
    let second = seconds_in_day % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"
    ))
}

// Howard Hinnant's civil-from-days transform, using days relative to 1970-01-01.
fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
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
