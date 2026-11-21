//! Host nearest-index Plan A. The series table function currently exposes no selected-point
//! argument, so the SQL predicate exercises the locked full-grid reader's slice feasibility.

use std::{collections::BTreeSet, path::Path, time::Instant};

use serde_json::json;

use crate::{
    BootstrappedConn,
    series::{
        CHUNK_TIME_LENGTH, DIMENSIONS_SQL, GRID_SQL, SERIES_VARIABLE, SPATIAL_AXES_SQL,
        SeriesError, SeriesRun, Station, TIME_STEP_SQL, deduplicate_rows, normalize_longitude,
        query_grid_descriptor, quote_sql_literal, read_latest_scan_measurement,
        regular_grid_from_descriptor, schema::StationSeriesRow,
    },
};

const DEG_TO_RAD: f64 = std::f64::consts::PI / 180.0;

#[derive(Clone, Copy, Debug)]
struct NearestCell {
    point_index: usize,
    distance_m: f64,
}

/// Run A: inspect the bound grid, select the nearest grid point on the host, then query the
/// series and join those point coordinates. No point-index pruning is provided by v0.4.0.
pub fn run(
    connection: &BootstrappedConn,
    stations: &[Station],
    series_prefix: &Path,
    object_path: &Path,
) -> Result<SeriesRun, SeriesError> {
    let started = Instant::now();
    let connection = connection
        .connection()
        .map_err(|error| SeriesError::DuckDb {
            stage: "connection",
            reason: error.to_string(),
        })?;
    let grid_info = query_grid_descriptor(connection, object_path)?;
    let grid = regular_grid_from_descriptor(&grid_info)?;

    let nearest = stations
        .iter()
        .map(|station| {
            (
                station.station_id.clone(),
                nearest_cell(&grid, station.longitude, station.latitude),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let selected_points = nearest
        .values()
        .filter_map(|value| *value)
        .map(|value| value.point_index)
        .collect::<BTreeSet<_>>();

    let values = stations
        .iter()
        .filter_map(|station| {
            nearest[&station.station_id].map(|cell| {
                format!(
                    "({}, {}::UBIGINT)",
                    quote_sql_literal(&station.station_id),
                    cell.point_index
                )
            })
        })
        .collect::<Vec<_>>();
    let series_path = series_prefix.to_string_lossy();
    let axes = grid_info
        .pointer("/layout/axes")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            SeriesError::InvalidInput("grid descriptor has no ordered axes".to_owned())
        })?;
    let member_axis = axes
        .iter()
        .position(|axis| axis.as_str() == Some("member"))
        .ok_or_else(|| {
            SeriesError::InvalidInput("station-series fixture has no member axis".to_owned())
        })?;
    let point_index_sql = format!(
        "((result.om_source.axis_indices[{}] * {}) + result.om_source.axis_indices[{}])",
        grid.latitude_axis + 1,
        grid.nx,
        grid.longitude_axis + 1
    );
    let member_index_sql = member_axis + 1;
    let sql = format!(
        "WITH selected(station_id, point_index) AS (VALUES {}) \
         SELECT selected.station_id, strftime(result.valid_time, '%Y-%m-%dT%H:%M:%S') || 'Z', \
                result.value::DOUBLE, result.lat::DOUBLE, result.lon::DOUBLE, \
                result.om_source.object_id, result.om_source.object_version \
         FROM read_om_series({}, chunk_time_length := {}, time_step := {}, \
                dimensions := {}, grid := {}, spatial_axes := {}, \
                valid_times := [TIMESTAMP '1970-01-01 00:00:00', \
                                TIMESTAMP '1970-01-01 02:00:00'], include_source := true) AS result \
         JOIN selected ON {} = selected.point_index \
         WHERE result.om_source.axis_indices[{}] = 0 \
         ORDER BY selected.station_id, result.valid_time",
        values.join(", "),
        quote_sql_literal(&series_path),
        CHUNK_TIME_LENGTH,
        TIME_STEP_SQL,
        DIMENSIONS_SQL,
        GRID_SQL,
        SPATIAL_AXES_SQL,
        point_index_sql,
        member_index_sql,
    );
    if values.is_empty() {
        return Err(SeriesError::InvalidInput(
            "Plan A has no stations inside the declared grid".to_owned(),
        ));
    }

    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| SeriesError::DuckDb {
            stage: "plan_a_prepare",
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
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(|error| SeriesError::DuckDb {
            stage: "plan_a_execute",
            reason: error.to_string(),
        })?;
    let mut rows = Vec::new();
    for row in result_rows {
        let (station_id, valid_time_utc, value, grid_lat, grid_lon, source_object, object_version) =
            row.map_err(|error| SeriesError::DuckDb {
                stage: "plan_a_read",
                reason: error.to_string(),
            })?;
        let cell = nearest[&station_id].expect("selected Plan A station has a nearest cell");
        rows.push(StationSeriesRow {
            station_id,
            variable: SERIES_VARIABLE.to_owned(),
            valid_time_utc,
            value,
            grid_lat,
            grid_lon: grid_lon.map(normalize_longitude),
            distance_m: Some(cell.distance_m),
            source_object,
            object_version,
        });
    }
    // The grid locator returns no candidate for out-of-domain stations. Preserve the stable
    // station/time schema with NULL spatial and value fields, matching read_om_regions.
    for station in stations
        .iter()
        .filter(|station| nearest[&station.station_id].is_none())
    {
        for time_index in 0..CHUNK_TIME_LENGTH {
            rows.push(StationSeriesRow {
                station_id: station.station_id.clone(),
                variable: SERIES_VARIABLE.to_owned(),
                valid_time_utc: format!("1970-01-01T{:02}:00:00Z", time_index),
                value: None,
                grid_lat: None,
                grid_lon: None,
                distance_m: None,
                source_object: None,
                object_version: None,
            });
        }
    }

    // The query rows are fully consumed before metrics are read, so the snapshot belongs to
    // this query and cannot be replaced by a later DuckDB scan.
    let scan = read_latest_scan_measurement(connection)?;
    let (rows, duplicate_rows_removed) = deduplicate_rows(rows)?;
    Ok(SeriesRun {
        rows,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        scan,
        duplicate_rows_removed,
        plan_details: json!({
            "grid_info": grid_info,
            "selected_unique_grid_points": selected_points.len(),
            "full_grid_points": grid.nx * grid.ny,
            "slice_api": "no selected-point argument in read_om_series v0.4.0; SQL coordinate join is applied after the table-function scan",
            "slice_feasibility": "full-grid read measured; selective grid-slice feasibility is no"
        }),
    })
}

fn nearest_cell(
    grid: &crate::series::RegularGridSpec,
    longitude: f64,
    latitude: f64,
) -> Option<NearestCell> {
    let latitude_last =
        grid.latitude_origin + grid.latitude_step * grid.ny.saturating_sub(1) as f64;
    if !latitude.is_finite()
        || !longitude.is_finite()
        || latitude < latitude_last.min(grid.latitude_origin)
        || latitude > latitude_last.max(grid.latitude_origin)
    {
        return None;
    }
    let normalized = normalize_longitude(longitude);
    let longitude_last =
        grid.longitude_origin + grid.longitude_step * grid.nx.saturating_sub(1) as f64;
    let grid_midpoint = (grid.longitude_origin + longitude_last) / 2.0;
    let unwrapped = normalized + 360.0 * ((grid_midpoint - normalized) / 360.0).round();
    if unwrapped < grid.longitude_origin.min(longitude_last)
        || unwrapped > grid.longitude_origin.max(longitude_last)
    {
        return None;
    }
    let mut nearest: Option<NearestCell> = None;
    for y in 0..grid.ny {
        let grid_lat = grid.latitude_origin + y as f64 * grid.latitude_step;
        for x in 0..grid.nx {
            let grid_lon = grid.longitude_origin + x as f64 * grid.longitude_step;
            let point_index = y * grid.nx + x;
            let distance_m = sphere_haversine_f64_v1(
                normalized,
                latitude,
                grid_lon,
                grid_lat,
                grid.earth_radius_m,
            );
            if nearest.is_none_or(|current| {
                distance_m < current.distance_m
                    || (distance_m == current.distance_m && point_index < current.point_index)
            }) {
                nearest = Some(NearestCell {
                    point_index,
                    distance_m,
                });
            }
        }
    }
    nearest
}

fn sphere_haversine_f64_v1(
    lon_a: f64,
    lat_a: f64,
    lon_b: f64,
    lat_b: f64,
    earth_radius_m: f64,
) -> f64 {
    if lat_a == lat_b && normalize_longitude(lon_a) == normalize_longitude(lon_b) {
        return 0.0;
    }
    let delta_lon = ((lon_b - lon_a + 180.0).rem_euclid(360.0) - 180.0).to_radians();
    let phi_a = lat_a * DEG_TO_RAD;
    let phi_b = lat_b * DEG_TO_RAD;
    let delta_phi = phi_b - phi_a;
    let sin_lat = (delta_phi / 2.0).sin();
    let sin_lon = (delta_lon / 2.0).sin();
    let a = (sin_lat * sin_lat + phi_a.cos() * phi_b.cos() * sin_lon * sin_lon).clamp(0.0, 1.0);
    earth_radius_m * 2.0 * a.sqrt().atan2((1.0 - a).sqrt())
}
