use omo_engine::{
    BootstrapConfig, BootstrappedConn,
    catalog::{
        check_readiness,
        run::{ConnMetaSource, RunError, RunRef, effective_window, resolve, time_index_table},
    },
    limits::job_limits,
    open_bootstrapped,
    plot::create_plot_artifact,
    query::{QueryPurpose, execute_query},
    series::{
        SERIES_VARIABLE, SeriesError, plan_a, plan_b,
        plan_g::{
            self, ParallelDuckDbPointReader, ensure_stable_version, object_version_token,
            write_matches_arrow, write_series_parquet,
        },
        quote_sql_literal, read_canonical_stations, read_stations,
        schema::write_series_arrow,
    },
};
use omo_protocol::{
    result_ref::ResultLimits,
    series::{
        CatalogCheckRequest, CatalogCheckResult, ResolveRunRequest, ResolveRunResult, RunSelector,
        SeriesRequest, SeriesResultBody, SqlSessionRequest, SqlSessionResult,
    },
    state::FailKind,
};
use omo_worker::server::{BackendError, StageReporter, WorkerBackend, run_stdio};
use serde_json::Value;
use std::{collections::HashMap, io::Write, path::PathBuf, time::Instant};

#[derive(Clone)]
struct QueryInfo {
    sql: String,
    rows: u64,
}

#[derive(Default)]
struct EngineBackend {
    bootstrap_config: Option<BootstrapConfig>,
    conn: Option<BootstrappedConn>,
    result_root: PathBuf,
    generation: u64,
    query_by_result: HashMap<String, QueryInfo>,
    series_cancel: std::sync::Arc<omo_engine::series::SeriesCancel>,
    stage_reporter: Option<StageReporter>,
}

impl EngineBackend {
    fn report_stage(&self, stage: &'static str) {
        if let Some(reporter) = &self.stage_reporter {
            reporter.set_stage(stage);
        }
    }
}

impl WorkerBackend for EngineBackend {
    fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    fn set_stage_reporter(&mut self, reporter: Option<StageReporter>) {
        self.stage_reporter = reporter;
    }

    fn bootstrap(&mut self, config: &Value, diagnostics: &mut dyn Write) -> Result<Value, String> {
        let config: BootstrapConfig =
            serde_json::from_value(config.clone()).map_err(|e| e.to_string())?;
        self.result_root = config.data_dir.join("results");
        self.bootstrap_config = Some(config.clone());
        let mut conn = open_bootstrapped(config).map_err(|e| e.to_string())?;
        conn.set_generation(self.generation);
        let report = serde_json::to_value(conn.report()).map_err(|e| e.to_string())?;
        writeln!(
            diagnostics,
            "engine bootstrap completed for generation {}",
            self.generation
        )
        .map_err(|e| e.to_string())?;
        self.conn = Some(conn);
        Ok(report)
    }

    fn probe_task(&mut self) -> Result<(), String> {
        self.conn
            .as_mut()
            .ok_or("engine is not bootstrapped")?
            .probe_before_task()
            .map_err(|e| e.to_string())
    }

    fn cancellation(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
        let handle = self.conn.as_ref()?.interrupt_handle().ok()?;
        // Called once per accepted request, before its task starts: forget the last request.
        self.series_cancel.reset();
        let series_cancel = std::sync::Arc::clone(&self.series_cancel);
        Some(Box::new(move || {
            series_cancel.cancel();
            handle.interrupt();
        }))
    }

    fn query(&mut self, body: &Value) -> Result<Value, BackendError> {
        let started = Instant::now();
        self.report_stage("reading");
        let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
        let sql = body
            .get("sql")
            .and_then(Value::as_str)
            .ok_or("query sql is required")?;
        let id = body
            .get("result_id")
            .and_then(Value::as_str)
            .ok_or("result id is required")?;
        let limits: ResultLimits = serde_json::from_value(
            body.get("limits")
                .cloned()
                .ok_or("query limits are required")?,
        )
        .map_err(|e| e.to_string())?;
        let purpose = match body.get("purpose").and_then(Value::as_str) {
            Some("preview") => QueryPurpose::Preview,
            Some("job") => QueryPurpose::Job,
            _ => return Err("query purpose must be preview or job".into()),
        };
        let result = execute_query(conn, sql, purpose, limits, &self.result_root, id)
            .map_err(|e| e.to_string())?;
        let rows = result.rows;
        self.report_stage("writing");
        let plot = if purpose == QueryPurpose::Preview {
            Some(create_plot_artifact(&result, &self.result_root).map_err(|e| e.to_string())?)
        } else {
            None
        };
        let duration_ms = started.elapsed().as_millis() as u64;
        let scan_bytes = omo_engine::series::latest_scan_measurement(conn)
            .ok()
            .and_then(|m| {
                m.raw_metrics
                    .pointer("/legacy_v2/bytes_fetched")
                    .and_then(Value::as_u64)
                    .or(m.physical_read_bytes)
                    .or(m.data_bytes)
            })
            .unwrap_or_else(|| {
                std::fs::metadata(&result.path)
                    .map(|meta| meta.len())
                    .unwrap_or(0)
            });
        let result_ref = serde_json::to_value(result).map_err(|e| e.to_string())?;
        let plot_ref = plot
            .as_ref()
            .and_then(|artifact| artifact.reference.as_ref())
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| e.to_string())?;
        self.query_by_result.insert(
            id.to_owned(),
            QueryInfo {
                sql: sql.to_owned(),
                rows,
            },
        );
        Ok(serde_json::json!({
            "result_ref": result_ref,
            "plot_ref": plot_ref,
            "plot_source_rows": plot.map(|artifact| artifact.source_rows),
            "scan_metrics": {
                "bytes_read": scan_bytes,
                "duration_ms": duration_ms,
                "rows": rows,
            },
        }))
    }

    fn catalog_check(&mut self, body: &Value) -> Result<Value, BackendError> {
        let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
        let req: CatalogCheckRequest =
            serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        let readiness = check_readiness(conn, &req.entry);
        let result = CatalogCheckResult {
            entry_id: req.entry.id,
            readiness,
        };
        serde_json::to_value(result).map_err(|e| e.to_string().into())
    }

    fn resolve_run(&mut self, body: &Value) -> Result<Value, BackendError> {
        let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
        if !conn.capabilities().remote {
            return Err(BackendError::typed(
                FailKind::Capability("capability `remote` is unavailable".into()),
                "capability `remote` is unavailable",
            ));
        }
        let req: ResolveRunRequest =
            serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        let source = ConnMetaSource {
            conn,
            entry: &req.entry,
        };
        let selector = match &req.run {
            RunSelector::LatestComplete(_) => RunRef::LatestComplete,
            RunSelector::Explicit { reference_time_utc } => {
                RunRef::Explicit(reference_time_utc.clone())
            }
        };
        let run = resolve(&req.entry, &source, selector).map_err(map_run_error)?;
        serde_json::to_value(ResolveRunResult { run }).map_err(|e| e.to_string().into())
    }

    fn series(&mut self, body: &Value) -> Result<Value, BackendError> {
        let started = Instant::now();
        self.report_stage("planning");
        let config = self
            .bootstrap_config
            .clone()
            .ok_or("bootstrap config is missing")?;
        let req: SeriesRequest = serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        let stations = read_canonical_stations(std::path::Path::new(&req.stations_ref))
            .map_err(|e| e.to_string())?;

        let (forecast_run, eff_window, time_index, version_before) = {
            let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
            if !conn.capabilities().remote {
                return Err(BackendError::typed(
                    FailKind::Capability("capability `remote` is unavailable".into()),
                    "capability `remote` is unavailable",
                ));
            }
            let source = ConnMetaSource {
                conn,
                entry: &req.catalog_entry,
            };
            let selector = match &req.run {
                RunSelector::LatestComplete(_) => RunRef::LatestComplete,
                RunSelector::Explicit { reference_time_utc } => {
                    RunRef::Explicit(reference_time_utc.clone())
                }
            };
            let forecast_run =
                resolve(&req.catalog_entry, &source, selector).map_err(map_run_error)?;
            let eff_window = effective_window(req.window, &forecast_run);
            let time_index = time_index_table(&forecast_run, &eff_window).map_err(map_run_error)?;
            let version_before =
                object_version_token(conn, &forecast_run.object_url).map_err(map_series_error)?;
            (forecast_run, eff_window, time_index, version_before)
        };

        let series_stations = stations
            .iter()
            .map(|s| plan_g::SeriesStation {
                station_id: s.station_id.clone(),
                latitude: s.latitude,
                longitude: s.longitude,
            })
            .collect::<Vec<_>>();

        self.report_stage("reading");
        let (plan_name, assembled, scan_stats) = match req.catalog_entry.grid.kind.as_str() {
            "reduced_gaussian_o1280" => {
                let mut reader = ParallelDuckDbPointReader {
                    config: &config,
                    domain: &req.catalog_entry.grid.duckomo_domain,
                    max_workers: 20,
                    cancel: Some(std::sync::Arc::clone(&self.series_cancel)),
                    scan_stats: plan_g::PlanGScanStats::default(),
                };
                let assembled = plan_g::assemble(
                    &series_stations,
                    &req.catalog_entry.variable.name,
                    &forecast_run.object_url,
                    &version_before,
                    f64::from(req.max_distance_m),
                    &time_index,
                    &forecast_run.object_url,
                    &mut reader,
                )
                .map_err(map_series_error)?;
                ("G", assembled, reader.scan_stats)
            }
            "regular_latlon" => {
                let conn = self.conn.as_mut().ok_or("engine is not bootstrapped")?;
                let (assembled, stats) = plan_b::run_remote_domain(
                    conn,
                    &series_stations,
                    &req.catalog_entry.variable.name,
                    &req.catalog_entry.grid.duckomo_domain,
                    &forecast_run.object_url,
                    &version_before,
                    f64::from(req.max_distance_m),
                    &time_index,
                )
                .map_err(map_series_error)?;
                ("B", assembled, stats)
            }
            other => {
                return Err(BackendError::typed(
                    FailKind::GridMismatch,
                    format!("unsupported catalog grid kind `{other}`"),
                ));
            }
        };

        let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
        let version_after =
            object_version_token(conn, &forecast_run.object_url).map_err(map_series_error)?;
        ensure_stable_version(&version_before, &version_after).map_err(map_series_error)?;
        self.series_cancel.check().map_err(map_series_error)?;

        self.report_stage("writing");
        let result_ref = write_series_arrow(
            &assembled.rows,
            self.generation,
            req.limits,
            &self.result_root,
            &req.result_id,
        )
        .map_err(map_series_error)?;
        let matches_ref = write_matches_arrow(
            &assembled.matches,
            self.generation,
            req.limits,
            &self.result_root,
            &req.result_id,
        )
        .map_err(map_series_error)?;
        let parquet_path = write_series_parquet(
            conn,
            &assembled.rows,
            self.generation,
            &self.result_root,
            &req.result_id,
        )
        .map_err(map_series_error)?;
        let parquet_lit = quote_sql_literal(&parquet_path.to_string_lossy());
        self.query_by_result.insert(
            req.result_id.clone(),
            QueryInfo {
                sql: format!("SELECT * FROM read_parquet({parquet_lit})"),
                rows: assembled.rows.len() as u64,
            },
        );
        let plot_artifact =
            create_plot_artifact(&result_ref, &self.result_root).map_err(|e| e.to_string())?;
        let mut forecast_run = forecast_run;
        if forecast_run.meta_etag.is_none() {
            forecast_run.meta_etag = Some(version_after.clone());
        }
        let duration_ms = started.elapsed().as_millis() as u64;
        let bytes_read = if scan_stats.bytes_fetched > 0 {
            scan_stats.bytes_fetched
        } else {
            scan_stats.data_bytes
        };

        let result_body = SeriesResultBody {
            result_ref: serde_json::to_value(&result_ref).map_err(|e| e.to_string())?,
            parquet_path: parquet_path.to_string_lossy().into_owned(),
            matches_ref: serde_json::to_value(&matches_ref).map_err(|e| e.to_string())?,
            plot_ref: plot_artifact
                .reference
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(|e| e.to_string())?,
            run: forecast_run,
            effective_window: eff_window,
            plan: plan_name.to_owned(),
            scan_metrics: serde_json::json!({
                "version_token": version_after,
                "bytes_read": bytes_read,
                "bytes_fetched": scan_stats.bytes_fetched,
                "data_bytes": scan_stats.data_bytes,
                "read_requests": scan_stats.read_requests,
                "duration_ms": duration_ms,
                "matched_stations": assembled.matches.iter().filter(|m| matches!(m.status, plan_g::MatchStatus::Matched)).count(),
                "total_stations": assembled.matches.len(),
                "rows": assembled.rows.len(),
            }),
        };
        serde_json::to_value(result_body).map_err(|e| e.to_string().into())
    }

    fn sql_session(&mut self, body: &Value) -> Result<Value, BackendError> {
        let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
        let req: SqlSessionRequest =
            serde_json::from_value(body.clone()).map_err(|e| e.to_string())?;
        let views = omo_engine::query::setup_sql_session(
            conn,
            &req.stations_ref,
            req.series_parquet.as_deref(),
        )
        .map_err(|e| e.to_string())?;
        serde_json::to_value(SqlSessionResult { views }).map_err(|e| e.to_string().into())
    }

    fn series_plan(&mut self, body: &Value) -> Result<Value, BackendError> {
        let plan = body
            .get("plan")
            .and_then(Value::as_str)
            .ok_or("series_plan plan is required")?;
        if plan != "A" && plan != "B" {
            return Err(format!(
                "series_plan plan must be \"A\" or \"B\", got `{plan}`"
            )
            .into());
        }
        let stations_path = body
            .get("stations_ref")
            .and_then(|value| {
                value
                    .as_str()
                    .or_else(|| value.get("path").and_then(Value::as_str))
            })
            .map(PathBuf::from)
            .ok_or("series_plan stations_ref path is required")?;
        let variable = body
            .get("variable")
            .and_then(Value::as_str)
            .ok_or("series_plan variable is required")?;
        if variable != SERIES_VARIABLE {
            return Err(format!(
                "unsupported series_plan variable `{variable}`; expected `{SERIES_VARIABLE}`"
            )
            .into());
        }
        let (window_start_seconds, window_end_seconds) = parse_window_utc(
            body.get("window_utc")
                .ok_or("series_plan window_utc is required")?,
        )?;
        let catalog_entry = body
            .get("catalog_entry")
            .ok_or("series_plan catalog_entry is required")?;
        let series_prefix = catalog_entry
            .as_str()
            .or_else(|| {
                catalog_entry
                    .get("series_prefix")
                    .or_else(|| catalog_entry.get("prefix"))
                    .or_else(|| catalog_entry.get("path"))
                    .and_then(Value::as_str)
            })
            .map(PathBuf::from)
            .ok_or("series_plan catalog_entry must provide series_prefix/path")?;
        let object_path = catalog_entry
            .get("object_path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| series_prefix.join("chunk_0000.om"));
        let id = body
            .get("result_id")
            .and_then(Value::as_str)
            .ok_or("result id is required")?;
        let limits: ResultLimits = match body.get("limits") {
            Some(value) => serde_json::from_value(value.clone()).map_err(|e| e.to_string())?,
            None => job_limits(),
        };

        let stations = read_stations(&stations_path).map_err(|e| e.to_string())?;
        let conn = self.conn.as_mut().ok_or("engine is not bootstrapped")?;
        let run = match plan {
            "A" => plan_a::run(conn, &stations, &series_prefix, &object_path),
            "B" => plan_b::run(
                conn,
                &stations,
                &series_prefix,
                window_start_seconds,
                window_end_seconds,
            ),
            _ => unreachable!(),
        }
        .map_err(|e| e.to_string())?;
        let result = write_series_arrow(&run.rows, self.generation, limits, &self.result_root, id)
            .map_err(|e| e.to_string())?;
        let result_ref = serde_json::to_value(result).map_err(|e| e.to_string())?;
        let scan_metrics = serde_json::to_value(run.scan).map_err(|e| e.to_string())?;
        Ok(serde_json::json!({
            "result_ref": result_ref,
            "scan_metrics": scan_metrics,
            "plan_details": run.plan_details,
        }))
    }

    fn export(&mut self, body: &Value) -> Result<Value, BackendError> {
        let conn = self.conn.as_ref().ok_or("engine is not bootstrapped")?;
        let (sql, known_rows) = if let Some(sql) = body.get("sql").and_then(Value::as_str) {
            (
                sql.to_owned(),
                body.get("row_count").and_then(Value::as_u64),
            )
        } else {
            let id = body
                .get("result_id")
                .and_then(Value::as_str)
                .ok_or("export sql or result_id is required")?;
            let query = self.query_by_result.get(id).ok_or("unknown result_id")?;
            (query.sql.clone(), Some(query.rows))
        };
        let target = PathBuf::from(
            body.get("target")
                .and_then(Value::as_str)
                .ok_or("export target is required")?,
        );
        if !target.is_absolute() {
            return Err("export target must be absolute".into());
        }
        let format = body
            .get("format")
            .and_then(Value::as_str)
            .ok_or("export format is required")?;
        let snapshot = body
            .get("job_snapshot")
            .cloned()
            .ok_or("export job snapshot is required")?;
        let confirmed = body
            .get("overwrite_confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let stage_reporter = self.stage_reporter.clone();
        let hash = omo_engine::export::export_sql_with_progress(
            conn,
            &sql,
            &target,
            format,
            snapshot,
            confirmed,
            known_rows,
            |stage| {
                if let Some(reporter) = &stage_reporter {
                    reporter.set_stage(stage);
                }
            },
        )
        .map_err(|e| e.to_string())?;
        Ok(
            serde_json::json!({"request_id":body.get("request_id"),"target":target,"sidecar":omo_engine::export::sidecar_path(&target),"data_sha256":hash}),
        )
    }

    fn interrupt(&mut self, _request_id: u64) -> Result<(), String> {
        Err("no active query".into())
    }
    fn ensure_spatial(&mut self) -> Result<Value, String> {
        let conn = self.conn.as_mut().ok_or("engine is not bootstrapped")?;
        conn.ensure_spatial().map_err(|e| e.to_string())?;
        serde_json::to_value(conn.report()).map_err(|e| e.to_string())
    }
}

fn parse_window_utc(value: &Value) -> Result<(i64, i64), String> {
    let pair = value
        .as_array()
        .filter(|items| items.len() == 2)
        .ok_or("series_plan window_utc must be a 2-element [start, end] array")?;
    let start = parse_utc_instant(&pair[0])?;
    let end = parse_utc_instant(&pair[1])?;
    if end < start {
        return Err("series_plan window_utc end must be >= start".into());
    }
    Ok((start, end))
}

fn parse_utc_instant(value: &Value) -> Result<i64, String> {
    if let Some(secs) = value.as_i64() {
        return Ok(secs);
    }
    let text = value
        .as_str()
        .ok_or("window_utc entries must be Unix seconds or RFC 3339 UTC strings")?;
    parse_rfc3339_utc_seconds(text)
}

fn parse_rfc3339_utc_seconds(text: &str) -> Result<i64, String> {
    let core = text
        .strip_suffix('Z')
        .ok_or_else(|| format!("window_utc timestamp must end with Z: {text}"))?;
    let (date, time) = core
        .split_once('T')
        .ok_or_else(|| format!("invalid RFC 3339 UTC timestamp: {text}"))?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("invalid year in timestamp: {text}"))?;
    let month: u32 = date_parts
        .next()
        .and_then(|s| s.parse().ok())
        .filter(|m| (1..=12).contains(m))
        .ok_or_else(|| format!("invalid month in timestamp: {text}"))?;
    let day: u32 = date_parts
        .next()
        .and_then(|s| s.parse().ok())
        .filter(|d| (1..=31).contains(d))
        .ok_or_else(|| format!("invalid day in timestamp: {text}"))?;
    if date_parts.next().is_some() {
        return Err(format!("invalid date in timestamp: {text}"));
    }
    let mut time_parts = time.split(':');
    let hour: u32 = time_parts
        .next()
        .and_then(|s| s.parse().ok())
        .filter(|h| *h < 24)
        .ok_or_else(|| format!("invalid hour in timestamp: {text}"))?;
    let minute: u32 = time_parts
        .next()
        .and_then(|s| s.parse().ok())
        .filter(|m| *m < 60)
        .ok_or_else(|| format!("invalid minute in timestamp: {text}"))?;
    let second: u32 = time_parts
        .next()
        .and_then(|s| s.parse().ok())
        .filter(|s| *s < 60)
        .ok_or_else(|| format!("invalid second in timestamp: {text}"))?;
    if time_parts.next().is_some() {
        return Err(format!("invalid time in timestamp: {text}"));
    }

    let days = omo_protocol::utc::days_from_civil(year, i64::from(month), i64::from(day));
    Ok(days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second))
}

fn map_run_error(error: RunError) -> BackendError {
    match error {
        RunError::NoCompleteRun => BackendError::typed(FailKind::NoCompleteRun, error.to_string()),
        RunError::RunIncomplete { .. } => {
            BackendError::typed(FailKind::RunIncomplete, error.to_string())
        }
        RunError::VariableMissing { .. } => {
            BackendError::typed(FailKind::VariableMissing, error.to_string())
        }
        RunError::CapabilityUnavailable(ref msg) => {
            BackendError::typed(FailKind::Capability(msg.clone()), error.to_string())
        }
        other => BackendError::new(other.to_string()),
    }
}

fn map_series_error(error: SeriesError) -> BackendError {
    match error {
        SeriesError::GridMismatch(ref msg) => {
            BackendError::typed(FailKind::GridMismatch, msg.clone())
        }
        SeriesError::SourceChanged(ref msg) => {
            BackendError::typed(FailKind::SourceChanged, msg.clone())
        }
        other => BackendError::new(other.to_string()),
    }
}

fn main() {
    match run_stdio(EngineBackend::default()) {
        Ok(
            omo_worker::server::WorkerExit::InputClosed | omo_worker::server::WorkerExit::Shutdown,
        ) => {}
        Ok(omo_worker::server::WorkerExit::BootstrapFailed) => std::process::exit(78),
        Err(error) => {
            eprintln!("worker protocol error: {error}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse_window_utc;

    #[test]
    fn parse_window_utc_accepts_unix_seconds_and_rfc3339() {
        assert_eq!(parse_window_utc(&json!([0, 7_200])).unwrap(), (0, 7_200));
        assert_eq!(
            parse_window_utc(&json!(["1970-01-01T00:00:00Z", "1970-01-01T02:00:00Z"])).unwrap(),
            (0, 7_200)
        );
        assert_eq!(
            parse_window_utc(&json!(["2024-02-29T23:59:59Z", "2024-03-01T00:00:00Z"])).unwrap(),
            (1_709_251_199, 1_709_251_200)
        );
        assert!(parse_window_utc(&json!([7_200, 0])).is_err());
        assert!(
            parse_window_utc(&json!([
                "2024-02-29T00:00:00+08:00",
                "2024-03-01T00:00:00Z"
            ]))
            .is_err()
        );
    }
}
