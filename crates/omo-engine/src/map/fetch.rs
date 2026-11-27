//! Regional forecast field loading (`contracts/worker-protocol-m2.md`, `map_load_field`).
//!
//! The field is read from the Open-Meteo `data_spatial` single-timestep object with a bounded
//! pool of latitude-band readers (each its own bootstrapped connection, all sharing one
//! [`SeriesCancel`]). A missing `data_spatial` object (`[object_not_found]` + `(HTTP 404)`)
//! falls back **once** to the `data_run` archive for the same run, valid time and variable.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use omo_protocol::{
    map::{MapBbox, SourceLayout},
    series::CatalogEntry,
};
use serde_json::{Value, json};
use thiserror::Error;

use super::{
    field::{CachedField, GridValues, compute_stats},
    raster::RegularGrid,
};
use crate::{
    catalog::{object_url, open_meteo_https, spatial_object_url},
    config::BootstrapConfig,
    grid::reduced_gaussian::O1280,
    series::{
        SeriesCancel, SeriesError, plan_g::object_version_token, read_latest_scan_measurement,
    },
};

/// Upper bound on concurrent latitude-band readers.
pub const MAX_READERS: usize = 16;
/// Largest synoptic window loaded in one request without clipping (degrees).
pub const MAX_SYNOPTIC_LAT_SPAN_DEG: f64 = 60.0;
pub const MAX_SYNOPTIC_LON_SPAN_DEG: f64 = 80.0;

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("map field load interrupted")]
    Cancelled,
    #[error("remote object is missing: {0}")]
    ObjectNotFound(String),
    #[error("the requested valid time is not published: {0}")]
    ValidTimeUnavailable(String),
    #[error("source object changed during the read: {0}")]
    SourceChanged(String),
    #[error("grid mismatch: {0}")]
    GridMismatch(String),
    #[error("DuckDB failed during {stage}: {reason}")]
    DuckDb { stage: &'static str, reason: String },
    #[error("invalid map field request: {0}")]
    InvalidInput(String),
}

impl From<SeriesError> for FetchError {
    fn from(error: SeriesError) -> Self {
        match error {
            SeriesError::Cancelled => Self::Cancelled,
            SeriesError::SourceChanged(detail) => Self::SourceChanged(detail),
            SeriesError::GridMismatch(detail) => Self::GridMismatch(detail),
            SeriesError::DuckDb { stage, reason } => classify_duckdb_error(stage, reason),
            other => Self::InvalidInput(other.to_string()),
        }
    }
}

/// True for the exact `duckomo` signature of an expired or missing remote object.
pub fn is_object_not_found(message: &str) -> bool {
    message.contains("[object_not_found]") && message.contains("(HTTP 404)")
}

/// The version-token probe (`read_text`) reports a missing object as an httpfs `HEAD` error
/// instead of the `duckomo` `[object_not_found]` text.
pub fn is_head_not_found(message: &str) -> bool {
    message.contains("HTTP HEAD error") && message.contains("HTTP 404")
}

fn classify_version_probe(error: SeriesError) -> FetchError {
    match error {
        SeriesError::DuckDb { reason, .. } if is_head_not_found(&reason) => {
            FetchError::ObjectNotFound(reason)
        }
        other => other.into(),
    }
}

pub fn classify_duckdb_error(stage: &'static str, reason: String) -> FetchError {
    if is_object_not_found(&reason) {
        FetchError::ObjectNotFound(reason)
    } else {
        FetchError::DuckDb { stage, reason }
    }
}

/// Total longitude extent of the bbox in degrees.
pub fn lon_extent(bbox: &MapBbox) -> f64 {
    bbox.lon_spans
        .iter()
        .map(|[lo, hi]| (hi - lo).max(0.0))
        .sum()
}

/// True when the bbox exceeds the `60 x 80` degree synoptic window.
pub fn needs_clip(bbox: &MapBbox) -> bool {
    (bbox.lat_max - bbox.lat_min) > MAX_SYNOPTIC_LAT_SPAN_DEG
        || lon_extent(bbox) > MAX_SYNOPTIC_LON_SPAN_DEG
}

/// Clip a wide request to the visible viewport (plus `gutter_deg`) when one is given, else to a
/// synoptic window around the bbox center. Windows within the limit are returned unchanged.
pub fn clip_bbox(bbox: &MapBbox, viewport: Option<&MapBbox>, gutter_deg: f64) -> MapBbox {
    if !needs_clip(bbox) {
        return bbox.clone();
    }
    let lat_min_all = bbox.lat_min;
    let lat_max_all = bbox.lat_max;
    let (lat_min, lat_max, spans) = match viewport {
        Some(view) => (
            (view.lat_min - gutter_deg).max(lat_min_all),
            (view.lat_max + gutter_deg).min(lat_max_all),
            view.lon_spans
                .iter()
                .map(|[lo, hi]| [(lo - gutter_deg).max(-180.0), (hi + gutter_deg).min(180.0)])
                .collect::<Vec<_>>(),
        ),
        None => {
            let center_lat = (lat_min_all + lat_max_all) / 2.0;
            let half_lat = MAX_SYNOPTIC_LAT_SPAN_DEG / 2.0;
            let first = bbox.lon_spans.first().copied().unwrap_or([-180.0, 180.0]);
            let center_lon = (first[0] + first[1]) / 2.0;
            let half_lon = MAX_SYNOPTIC_LON_SPAN_DEG / 2.0;
            (
                (center_lat - half_lat).max(lat_min_all),
                (center_lat + half_lat).min(lat_max_all),
                vec![[
                    (center_lon - half_lon).max(-180.0),
                    (center_lon + half_lon).min(180.0),
                ]],
            )
        }
    };
    // A viewport can still be wider than the window; shrink around its center.
    let mut spans = spans;
    let mut total: f64 = spans.iter().map(|[lo, hi]| hi - lo).sum();
    if total > MAX_SYNOPTIC_LON_SPAN_DEG {
        let [lo, hi] = spans[0];
        let center = (lo + hi) / 2.0;
        spans = vec![[
            (center - MAX_SYNOPTIC_LON_SPAN_DEG / 2.0).max(-180.0),
            (center + MAX_SYNOPTIC_LON_SPAN_DEG / 2.0).min(180.0),
        ]];
        total = MAX_SYNOPTIC_LON_SPAN_DEG;
    }
    let _ = total;
    let (mut lat_min, mut lat_max) = (lat_min, lat_max);
    if lat_max - lat_min > MAX_SYNOPTIC_LAT_SPAN_DEG {
        let center = (lat_min + lat_max) / 2.0;
        lat_min = center - MAX_SYNOPTIC_LAT_SPAN_DEG / 2.0;
        lat_max = center + MAX_SYNOPTIC_LAT_SPAN_DEG / 2.0;
    }
    MapBbox {
        lat_min,
        lat_max,
        lon_spans: spans,
    }
}

/// One rectangular read: a latitude band crossed with one longitude segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandTask {
    pub lat_lo: f64,
    pub lat_hi: f64,
    pub lon_lo: f64,
    pub lon_hi: f64,
}

/// O1280 rows `y` (north to south) whose latitude lies in `[lat_min, lat_max]`.
pub fn rows_in_lat_range(lat_min: f64, lat_max: f64) -> Option<std::ops::RangeInclusive<usize>> {
    if !(lat_min.is_finite() && lat_max.is_finite()) || lat_min > lat_max {
        return None;
    }
    let lat = |y: usize| f64::from(O1280::lat_of_row(y));
    let last = O1280::rows() - 1;
    let mut first_row = O1280::row_near_lat(lat_max);
    while first_row < last && lat(first_row) > lat_max {
        first_row += 1;
    }
    while first_row > 0 && lat(first_row - 1) <= lat_max {
        first_row -= 1;
    }
    let mut last_row = O1280::row_near_lat(lat_min);
    while last_row > 0 && lat(last_row) < lat_min {
        last_row -= 1;
    }
    while last_row < last && lat(last_row + 1) >= lat_min {
        last_row += 1;
    }
    if lat(first_row) > lat_max || lat(last_row) < lat_min || first_row > last_row {
        return None;
    }
    Some(first_row..=last_row)
}

/// Split `bbox` into at most `readers` latitude bands (balanced by row count) per longitude
/// segment. Band edges sit halfway between rows, so every row belongs to exactly one band.
pub fn plan_bands(bbox: &MapBbox, readers: usize) -> Vec<BandTask> {
    let Some(rows) = rows_in_lat_range(bbox.lat_min, bbox.lat_max) else {
        return Vec::new();
    };
    let lat = |y: usize| f64::from(O1280::lat_of_row(y));
    let (first, last) = (*rows.start(), *rows.end());
    let row_count = last - first + 1;
    let bands = readers.clamp(1, MAX_READERS).min(row_count);
    let mut edges = Vec::with_capacity(bands);
    for band in 0..bands {
        let r0 = first + band * row_count / bands;
        let r1 = first + (band + 1) * row_count / bands - 1;
        let hi = if band == 0 {
            bbox.lat_max
        } else {
            (lat(r0 - 1) + lat(r0)) / 2.0
        };
        let lo = if band + 1 == bands {
            bbox.lat_min
        } else {
            (lat(r1) + lat(r1 + 1)) / 2.0
        };
        edges.push((lo, hi));
    }
    let mut tasks = Vec::new();
    for [lon_lo, lon_hi] in &bbox.lon_spans {
        for &(lat_lo, lat_hi) in &edges {
            tasks.push(BandTask {
                lat_lo,
                lat_hi,
                lon_lo: *lon_lo,
                lon_hi: *lon_hi,
            });
        }
    }
    tasks
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// SQL for one band. `data_spatial` objects hold a single timestep and need an explicit
/// `variable :=`; `data_run` objects are per variable and are filtered on `valid_time`.
pub fn band_sql(
    layout: SourceLayout,
    object_https: &str,
    domain: &str,
    variable: &str,
    valid_time_utc: &str,
    task: &BandTask,
) -> String {
    let source = match layout {
        SourceLayout::DataSpatial => format!(
            "read_om({}, domain := {}, variable := {})",
            quote(object_https),
            quote(domain),
            quote(variable)
        ),
        SourceLayout::DataRun => {
            format!(
                "read_om({}, domain := {})",
                quote(object_https),
                quote(domain)
            )
        }
    };
    let time_filter = match layout {
        SourceLayout::DataSpatial => String::new(),
        SourceLayout::DataRun => format!(
            "valid_time = {}::TIMESTAMP AND ",
            quote(&valid_time_utc.trim_end_matches('Z').replace('T', " "))
        ),
    };
    // `variable :=` names the value column after the variable; `data_run` uses `value`.
    let value_column = match layout {
        SourceLayout::DataSpatial => format!("\"{}\"", variable.replace('"', "\"\"")),
        SourceLayout::DataRun => "value".to_owned(),
    };
    format!(
        "SELECT lat::FLOAT, lon::FLOAT, {value_column}::FLOAT FROM {source} \
         WHERE {time_filter}lat BETWEEN {:.8} AND {:.8} AND lon BETWEEN {:.8} AND {:.8}",
        task.lat_lo, task.lat_hi, task.lon_lo, task.lon_hi
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandRow {
    pub lat: f32,
    pub lon: f32,
    /// `NAN` when the source reports no value.
    pub value: f32,
}

/// Place rows into a dense O1280 value array. Rows must match the grid exactly.
pub fn fill_o1280(rows: &[BandRow]) -> Result<GridValues, FetchError> {
    let mut values = GridValues::o1280_empty();
    let slice = values.as_mut_slice();
    for row in rows {
        let y = O1280::row_near_lat(f64::from(row.lat));
        if O1280::lat_of_row(y).to_bits() != row.lat.to_bits() {
            return Err(FetchError::GridMismatch(format!(
                "latitude {} is not an O1280 row",
                row.lat
            )));
        }
        let x = O1280::column_of(y, row.lon).ok_or_else(|| {
            FetchError::GridMismatch(format!(
                "longitude {} is not an O1280 column in row {y}",
                row.lon
            ))
        })?;
        slice[O1280::flat_index(y, x) as usize] = row.value;
    }
    Ok(values)
}

/// North-up regular lon/lat grid geometry derived from `om_grid_info` (`FR-009`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegularFieldSpec {
    pub lat_top: f64,
    pub lon_west: f64,
    pub dlat: f64,
    pub dlon: f64,
    pub nx: usize,
    pub ny: usize,
}

impl RegularFieldSpec {
    pub fn from_om_descriptor(
        grid_type: &str,
        definition: &Value,
        expected_points: u64,
    ) -> Result<Self, FetchError> {
        if grid_type != "regular_latlon" {
            return Err(FetchError::GridMismatch(format!(
                "om_grid_info reported `{grid_type}`, expected `regular_latlon`"
            )));
        }
        let params = definition.pointer("/parameters").ok_or_else(|| {
            FetchError::GridMismatch("om_grid_info omitted definition.parameters".into())
        })?;
        let get_u64 = |k: &str| {
            params
                .get(k)
                .and_then(Value::as_u64)
                .ok_or_else(|| FetchError::GridMismatch(format!("missing grid parameter `{k}`")))
        };
        let get_f64 = |k: &str| {
            params
                .get(k)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .ok_or_else(|| FetchError::GridMismatch(format!("invalid grid parameter `{k}`")))
        };
        let nx_u64 = get_u64("nx")?;
        let ny_u64 = get_u64("ny")?;
        if nx_u64 == 0 || ny_u64 == 0 || nx_u64.checked_mul(ny_u64) != Some(expected_points) {
            return Err(FetchError::GridMismatch(format!(
                "om_grid_info dimensions {nx_u64}x{ny_u64} do not match catalog points {expected_points}"
            )));
        }
        let nx = usize::try_from(nx_u64)
            .map_err(|_| FetchError::GridMismatch("nx exceeds usize".into()))?;
        let ny = usize::try_from(ny_u64)
            .map_err(|_| FetchError::GridMismatch("ny exceeds usize".into()))?;
        let lat0 = get_f64("lat0")?;
        let lon0 = get_f64("lon0")?;
        let raw_dlat = get_f64("dlat")?;
        let raw_dlon = get_f64("dlon")?;
        if raw_dlat == 0.0 || raw_dlon == 0.0 {
            return Err(FetchError::GridMismatch(
                "regular grid steps must be non-zero".into(),
            ));
        }
        let (lat_top, dlat) = if raw_dlat < 0.0 {
            (lat0, -raw_dlat)
        } else {
            (lat0 + (ny - 1) as f64 * raw_dlat, raw_dlat)
        };
        let (lon_west, dlon) = if raw_dlon > 0.0 {
            (lon0, raw_dlon)
        } else {
            (lon0 + (nx - 1) as f64 * raw_dlon, -raw_dlon)
        };
        Ok(Self {
            lat_top,
            lon_west: omo_protocol::mercator::normalize_lon(lon_west),
            dlat,
            dlon,
            nx,
            ny,
        })
    }

    pub fn lat_of_row(&self, y: usize) -> f64 {
        self.lat_top - (y as f64) * self.dlat
    }

    pub fn is_global_lon(&self) -> bool {
        self.nx >= 2 && ((self.nx as f64) * self.dlon - 360.0).abs() <= 1e-4
    }
}

/// Regular-grid rows `y` (north to south, `0..spec.ny`) whose latitude lies in `[lat_min, lat_max]`.
pub fn regular_rows_in_lat_range(
    spec: &RegularFieldSpec,
    lat_min: f64,
    lat_max: f64,
) -> Option<std::ops::RangeInclusive<usize>> {
    if !(lat_min.is_finite() && lat_max.is_finite()) || lat_min > lat_max || spec.ny == 0 {
        return None;
    }
    let first = ((spec.lat_top - lat_max) / spec.dlat).ceil().max(0.0);
    let last = ((spec.lat_top - lat_min) / spec.dlat)
        .floor()
        .min((spec.ny - 1) as f64);
    if first > last || first >= spec.ny as f64 || last < 0.0 {
        return None;
    }
    Some(first as usize..=last as usize)
}

/// Partition `bbox` across at most `readers` non-overlapping latitude bands on a regular grid.
pub fn plan_regular_bands(
    bbox: &MapBbox,
    spec: &RegularFieldSpec,
    readers: usize,
) -> Vec<BandTask> {
    let Some(rows) = regular_rows_in_lat_range(spec, bbox.lat_min, bbox.lat_max) else {
        return Vec::new();
    };
    let (first, last) = (*rows.start(), *rows.end());
    let row_count = last - first + 1;
    let bands = readers.clamp(1, MAX_READERS).min(row_count);
    let mut edges = Vec::with_capacity(bands);
    for band in 0..bands {
        let r0 = first + band * row_count / bands;
        let r1 = first + (band + 1) * row_count / bands - 1;
        let hi = if band == 0 {
            bbox.lat_max
        } else {
            (spec.lat_of_row(r0 - 1) + spec.lat_of_row(r0)) / 2.0
        };
        let lo = if band + 1 == bands {
            bbox.lat_min
        } else {
            (spec.lat_of_row(r1) + spec.lat_of_row(r1 + 1)) / 2.0
        };
        edges.push((lo, hi));
    }
    let mut tasks = Vec::new();
    for [lon_lo, lon_hi] in &bbox.lon_spans {
        for &(lat_lo, lat_hi) in &edges {
            tasks.push(BandTask {
                lat_lo,
                lat_hi,
                lon_lo: *lon_lo,
                lon_hi: *lon_hi,
            });
        }
    }
    tasks
}

/// Place rows into a north-up [`RegularGrid`], rejecting coordinates off the declared grid.
pub fn fill_regular(spec: &RegularFieldSpec, rows: &[BandRow]) -> Result<GridValues, FetchError> {
    const TOL_STEPS: f64 = 0.02;
    let total = spec
        .nx
        .checked_mul(spec.ny)
        .ok_or_else(|| FetchError::GridMismatch("regular grid size overflows".into()))?;
    let mut values = vec![f32::NAN; total];
    let global = spec.is_global_lon();
    for row in rows {
        let fy = (spec.lat_top - f64::from(row.lat)) / spec.dlat;
        let yr = fy.round();
        if !fy.is_finite() || yr < 0.0 || yr >= spec.ny as f64 || (fy - yr).abs() > TOL_STEPS {
            return Err(FetchError::GridMismatch(format!(
                "latitude {} does not align with regular grid (lat_top={}, dlat={})",
                row.lat, spec.lat_top, spec.dlat
            )));
        }
        let mut delta = (f64::from(row.lon) - spec.lon_west).rem_euclid(360.0);
        if !global && delta > 360.0 - spec.dlon * TOL_STEPS {
            delta -= 360.0;
        }
        let fx = delta / spec.dlon;
        let xr = fx.round();
        let x = if global {
            if !fx.is_finite() || (fx - xr).abs() > TOL_STEPS {
                return Err(FetchError::GridMismatch(format!(
                    "longitude {} does not align with global regular grid",
                    row.lon
                )));
            }
            (xr as usize) % spec.nx
        } else {
            if !fx.is_finite() || xr < 0.0 || xr >= spec.nx as f64 || (fx - xr).abs() > TOL_STEPS {
                return Err(FetchError::GridMismatch(format!(
                    "longitude {} lies outside regular grid",
                    row.lon
                )));
            }
            xr as usize
        };
        values[(yr as usize) * spec.nx + x] = row.value;
    }
    Ok(GridValues::Regular(RegularGrid {
        lat_top: spec.lat_top,
        lon_west: spec.lon_west,
        dlat: spec.dlat,
        dlon: spec.dlon,
        nx: spec.nx,
        ny: spec.ny,
        values,
    }))
}

fn probe_regular_grid_spec(
    config: &BootstrapConfig,
    cancel: &Option<Arc<SeriesCancel>>,
    layout: SourceLayout,
    object_url: &str,
    domain: &str,
    variable: &str,
    expected_points: u64,
) -> Result<RegularFieldSpec, FetchError> {
    check_cancel(cancel)?;
    let https = open_meteo_https(object_url);
    let conn = open_conn(config, cancel, "map_grid_info_open")?;
    let db = conn.connection().map_err(|error| FetchError::DuckDb {
        stage: "map_grid_info_conn",
        reason: error.to_string(),
    })?;
    let query_with = |with_var: bool| -> Result<(String, String), duckdb::Error> {
        let sql = if with_var {
            format!(
                "SELECT grid_type, definition FROM om_grid_info({}, domain := {}, variable := {})",
                quote(&https),
                quote(domain),
                quote(variable)
            )
        } else {
            format!(
                "SELECT grid_type, definition FROM om_grid_info({}, domain := {})",
                quote(&https),
                quote(domain)
            )
        };
        db.query_row(&sql, [], |row| Ok((row.get(0)?, row.get(1)?)))
    };
    let res = match layout {
        SourceLayout::DataSpatial => query_with(true).or_else(|err| {
            let text = err.to_string();
            if is_object_not_found(&text) || is_head_not_found(&text) {
                Err(err)
            } else {
                query_with(false)
            }
        }),
        SourceLayout::DataRun => query_with(false),
    };
    let (grid_type, def_json) = res.map_err(|error| {
        let msg = error.to_string();
        if is_object_not_found(&msg) || is_head_not_found(&msg) {
            FetchError::ObjectNotFound(msg)
        } else {
            FetchError::DuckDb {
                stage: "map_grid_info",
                reason: msg,
            }
        }
    })?;
    let def: Value = serde_json::from_str(&def_json)
        .map_err(|error| FetchError::GridMismatch(format!("invalid om_grid_info JSON: {error}")))?;
    RegularFieldSpec::from_om_descriptor(&grid_type, &def, expected_points)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanMetrics {
    pub bytes_fetched: u64,
    pub read_requests: u64,
    pub data_bytes: u64,
    pub tasks: usize,
    pub readers: usize,
    /// Slowest reader's connection bootstrap time.
    pub open_ms_max: u64,
    /// Sum of all band query times across readers.
    pub query_ms_sum: u64,
}

impl ScanMetrics {
    pub fn to_json(self) -> Value {
        json!({
            "bytes_fetched": self.bytes_fetched,
            "read_requests": self.read_requests,
            "data_bytes": self.data_bytes,
            "tasks": self.tasks,
            "readers": self.readers,
            "open_ms_max": self.open_ms_max,
            "query_ms_sum": self.query_ms_sum,
        })
    }
}

pub struct BandSource<'a> {
    pub config: &'a BootstrapConfig,
    pub layout: SourceLayout,
    /// `s3://openmeteo/...` or `https://...` object URL.
    pub object_url: &'a str,
    pub domain: &'a str,
    pub variable: &'a str,
    pub valid_time_utc: &'a str,
    pub readers: usize,
    pub cancel: Option<Arc<SeriesCancel>>,
}

#[derive(Debug)]
pub struct FetchedBands {
    pub rows: Vec<BandRow>,
    pub object_version: Option<String>,
    pub metrics: ScanMetrics,
}

fn check_cancel(cancel: &Option<Arc<SeriesCancel>>) -> Result<(), FetchError> {
    match cancel {
        Some(cancel) => Ok(cancel.check()?),
        None => Ok(()),
    }
}

/// Read every band in parallel. The object's version token is read before and after; a change
/// is [`FetchError::SourceChanged`].
pub fn fetch_bands(
    source: &BandSource<'_>,
    tasks: &[BandTask],
) -> Result<FetchedBands, FetchError> {
    check_cancel(&source.cancel)?;
    let https = open_meteo_https(source.object_url);
    let readers = source.readers.clamp(1, MAX_READERS).min(tasks.len().max(1));

    let probe = open_conn(source.config, &source.cancel, "map_fetch_probe_open")?;
    let version_before = object_version_token(&probe, &https).map_err(classify_version_probe)?;

    let next = AtomicUsize::new(0);
    type WorkerResult = Result<(Vec<BandRow>, ScanMetrics), FetchError>;
    let collected: Mutex<Vec<WorkerResult>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..readers {
            scope.spawn(|| {
                let result = read_worker(source, &https, tasks, &next);
                collected
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(result);
            });
        }
    });

    let mut rows = Vec::new();
    let mut metrics = ScanMetrics {
        tasks: tasks.len(),
        readers,
        ..ScanMetrics::default()
    };
    let mut first_error: Option<FetchError> = None;
    for result in collected.into_inner().unwrap_or_else(|e| e.into_inner()) {
        match result {
            Ok((mut part, part_metrics)) => {
                rows.append(&mut part);
                metrics.bytes_fetched += part_metrics.bytes_fetched;
                metrics.read_requests += part_metrics.read_requests;
                metrics.data_bytes += part_metrics.data_bytes;
                metrics.open_ms_max = metrics.open_ms_max.max(part_metrics.open_ms_max);
                metrics.query_ms_sum += part_metrics.query_ms_sum;
            }
            Err(error) => {
                // Cancellation wins over incidental interrupt errors from other readers.
                let replace = match (&first_error, &error) {
                    (None, _) => true,
                    (Some(FetchError::Cancelled), _) => false,
                    (Some(_), FetchError::Cancelled) => true,
                    (Some(FetchError::ObjectNotFound(_)), _) => false,
                    (Some(_), FetchError::ObjectNotFound(_)) => true,
                    _ => false,
                };
                if replace {
                    first_error = Some(error);
                }
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    check_cancel(&source.cancel)?;
    let version_after = object_version_token(&probe, &https).map_err(classify_version_probe)?;
    if version_before != version_after {
        return Err(FetchError::SourceChanged(format!(
            "{https}: {version_before} -> {version_after}"
        )));
    }
    Ok(FetchedBands {
        rows,
        object_version: Some(version_before),
        metrics,
    })
}

fn open_conn(
    config: &BootstrapConfig,
    cancel: &Option<Arc<SeriesCancel>>,
    stage: &'static str,
) -> Result<crate::conn::BootstrappedConn, FetchError> {
    let conn = crate::open_bootstrapped(config.clone()).map_err(|error| FetchError::DuckDb {
        stage,
        reason: error.to_string(),
    })?;
    if let Some(cancel) = cancel {
        cancel.register(
            conn.interrupt_handle()
                .map_err(|error| FetchError::DuckDb {
                    stage,
                    reason: error.to_string(),
                })?,
        );
    }
    Ok(conn)
}

fn read_worker(
    source: &BandSource<'_>,
    https: &str,
    tasks: &[BandTask],
    next: &AtomicUsize,
) -> Result<(Vec<BandRow>, ScanMetrics), FetchError> {
    check_cancel(&source.cancel)?;
    let open_started = std::time::Instant::now();
    let conn = open_conn(source.config, &source.cancel, "map_fetch_open")?;
    let db = conn.connection().map_err(|error| FetchError::DuckDb {
        stage: "map_fetch_conn",
        reason: error.to_string(),
    })?;
    let mut rows = Vec::new();
    let mut metrics = ScanMetrics {
        open_ms_max: open_started.elapsed().as_millis() as u64,
        ..ScanMetrics::default()
    };
    loop {
        check_cancel(&source.cancel)?;
        let task_started = std::time::Instant::now();
        let index = next.fetch_add(1, Ordering::SeqCst);
        let Some(task) = tasks.get(index) else { break };
        let sql = band_sql(
            source.layout,
            https,
            source.domain,
            source.variable,
            source.valid_time_utc,
            task,
        );
        let mut statement = db
            .prepare(&sql)
            .map_err(|error| classify_duckdb_error("map_fetch_prepare", error.to_string()))?;
        let mapped = statement
            .query_map([], |row| {
                Ok(BandRow {
                    lat: row.get::<_, f32>(0)?,
                    lon: row.get::<_, f32>(1)?,
                    value: row.get::<_, Option<f32>>(2)?.unwrap_or(f32::NAN),
                })
            })
            .map_err(|error| classify_duckdb_error("map_fetch_query", error.to_string()))?;
        for row in mapped {
            rows.push(
                row.map_err(|error| classify_duckdb_error("map_fetch_read", error.to_string()))?,
            );
        }
        metrics.query_ms_sum += task_started.elapsed().as_millis() as u64;
        if let Ok(measurement) = read_latest_scan_measurement(db) {
            metrics.bytes_fetched += measurement
                .raw_metrics
                .pointer("/legacy_v2/bytes_fetched")
                .and_then(Value::as_u64)
                .or(measurement.physical_read_bytes)
                .or(measurement.data_bytes)
                .unwrap_or(0);
            metrics.read_requests += measurement
                .raw_metrics
                .pointer("/legacy_v2/read_requests")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            metrics.data_bytes += measurement.data_bytes.unwrap_or(0);
        }
    }
    Ok((rows, metrics))
}

/// Everything `map_load_field` needs once the run and valid time are resolved.
pub struct LoadFieldParams<'a> {
    pub config: &'a BootstrapConfig,
    pub entry: &'a CatalogEntry,
    pub field_id: &'a str,
    pub reference_time_utc: &'a str,
    pub valid_time_utc: &'a str,
    pub bbox: &'a MapBbox,
    pub force_data_run: bool,
    pub readers: usize,
    pub cancel: Option<Arc<SeriesCancel>>,
}

#[derive(Debug)]
pub struct LoadedField {
    pub field: CachedField,
    pub fallback_notice: Option<String>,
    pub metrics: ScanMetrics,
}

/// Progress callback: `(stage, detail)`.
pub type Progress<'a> = &'a mut dyn FnMut(&'static str, Option<&str>);

#[derive(Clone, Copy)]
enum GridMode {
    O1280,
    Regular,
}

/// Load a regional field, falling back from `data_spatial` to `data_run` at most once.
pub fn load_field(
    params: &LoadFieldParams<'_>,
    progress: Progress<'_>,
) -> Result<LoadedField, FetchError> {
    let mode = match params.entry.grid.kind.as_str() {
        "reduced_gaussian_o1280" => {
            if params.entry.grid.points != u64::from(O1280::points()) {
                return Err(FetchError::GridMismatch(format!(
                    "O1280 entry declared {} points; expected {}",
                    params.entry.grid.points,
                    O1280::points()
                )));
            }
            GridMode::O1280
        }
        "regular_latlon" => {
            if params.entry.grid.points == 0 {
                return Err(FetchError::GridMismatch(
                    "regular_latlon entry declared 0 points".into(),
                ));
            }
            GridMode::Regular
        }
        other => {
            return Err(FetchError::GridMismatch(format!(
                "unsupported catalog grid kind `{other}`"
            )));
        }
    };
    let bbox = clip_bbox(params.bbox, None, 0.0);
    let domain = params.entry.grid.duckomo_domain.as_str();
    let variable = params.entry.variable.name.as_str();

    let run_attempt = |layout: SourceLayout,
                       url: &str|
     -> Result<(Option<RegularFieldSpec>, FetchedBands), FetchError> {
        let (spec, tasks) = match mode {
            GridMode::O1280 => {
                let tasks = plan_bands(&bbox, params.readers);
                if tasks.is_empty() {
                    return Err(FetchError::InvalidInput(
                        "the requested bbox contains no grid rows".to_owned(),
                    ));
                }
                (None, tasks)
            }
            GridMode::Regular => {
                let spec = probe_regular_grid_spec(
                    params.config,
                    &params.cancel,
                    layout,
                    url,
                    domain,
                    variable,
                    params.entry.grid.points,
                )?;
                let tasks = plan_regular_bands(&bbox, &spec, params.readers);
                if tasks.is_empty() {
                    return Err(FetchError::InvalidInput(
                        "the requested bbox contains no grid rows".to_owned(),
                    ));
                }
                (Some(spec), tasks)
            }
        };
        let source = BandSource {
            config: params.config,
            layout,
            object_url: url,
            domain,
            variable,
            valid_time_utc: params.valid_time_utc,
            readers: params.readers,
            cancel: params.cancel.clone(),
        };
        let fetched = fetch_bands(&source, &tasks)?;
        Ok((spec, fetched))
    };

    let spatial_url = if params.force_data_run {
        None
    } else {
        spatial_object_url(
            params.entry,
            params.reference_time_utc,
            params.valid_time_utc,
        )
    };
    let mut fallback_notice = None;
    let mut attempt = None;
    if let Some(url) = spatial_url {
        progress("reading", None);
        match run_attempt(SourceLayout::DataSpatial, &url) {
            Ok((spec, fetched)) => attempt = Some((SourceLayout::DataSpatial, url, spec, fetched)),
            Err(FetchError::ObjectNotFound(_)) => {
                let detail = "data_spatial expired; falling back to data_run archive";
                progress("reading", Some(detail));
                fallback_notice = Some(detail.to_owned());
            }
            Err(error) => return Err(error),
        }
    }
    let (layout, source_object, regular_spec, fetched) = match attempt {
        Some(done) => done,
        None => {
            let url = object_url(params.entry, params.reference_time_utc);
            if fallback_notice.is_none() {
                progress("reading", None);
            }
            match run_attempt(SourceLayout::DataRun, &url) {
                Ok((spec, fetched)) => (SourceLayout::DataRun, url, spec, fetched),
                Err(FetchError::ObjectNotFound(detail)) => {
                    return Err(FetchError::ValidTimeUnavailable(format!(
                        "{} is not published by data_spatial or data_run: {detail}",
                        params.valid_time_utc
                    )));
                }
                Err(error) => return Err(error),
            }
        }
    };

    progress("writing", None);
    let values = match regular_spec {
        None => fill_o1280(&fetched.rows)?,
        Some(spec) => fill_regular(&spec, &fetched.rows)?,
    };
    let stats = compute_stats(values.as_slice(), &params.entry.variable.unit);
    Ok(LoadedField {
        field: CachedField {
            field_id: params.field_id.to_owned(),
            entry_id: params.entry.id.clone(),
            reference_time_utc: params.reference_time_utc.to_owned(),
            valid_time_utc: params.valid_time_utc.to_owned(),
            variable: variable.to_owned(),
            unit: params.entry.variable.unit.clone(),
            source_layout: layout,
            source_object,
            object_version: fetched.object_version,
            coverage: bbox,
            values,
            stats,
        },
        fallback_notice,
        metrics: fetched.metrics,
    })
}
