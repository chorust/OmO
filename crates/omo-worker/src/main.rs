use omo_engine::{
    BootstrapConfig, BootstrappedConn, limits::job_limits, open_bootstrapped,
    plot::create_plot_artifact,
    query::{QueryPurpose, execute_query},
    series::{SERIES_VARIABLE, plan_a, plan_b, read_stations, schema::write_series_arrow},
};
use omo_protocol::result_ref::ResultLimits;
use omo_worker::server::{WorkerBackend, run_stdio};
use serde_json::Value;
use std::{collections::HashMap, io::Write, path::PathBuf};

#[derive(Clone)]
struct QueryInfo {
    sql: String,
    rows: u64,
}

#[derive(Default)]
struct EngineBackend {
    conn: Option<BootstrappedConn>,
    result_root: PathBuf,
    generation: u64,
    query_by_result: HashMap<String, QueryInfo>,
}

impl WorkerBackend for EngineBackend {
    fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    fn bootstrap(&mut self, config: &Value, diagnostics: &mut dyn Write) -> Result<Value, String> {
        let config: BootstrapConfig =
            serde_json::from_value(config.clone()).map_err(|e| e.to_string())?;
        self.result_root = config.data_dir.join("results");
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
        Some(Box::new(move || handle.interrupt()))
    }

    fn query(&mut self, body: &Value) -> Result<Value, String> {
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
        let plot = if purpose == QueryPurpose::Preview {
            Some(create_plot_artifact(&result, &self.result_root).map_err(|e| e.to_string())?)
        } else {
            None
        };
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
        }))
    }

    fn series_plan(&mut self, body: &Value) -> Result<Value, String> {
        let plan = body
            .get("plan")
            .and_then(Value::as_str)
            .ok_or("series_plan plan is required")?;
        if plan != "A" && plan != "B" {
            return Err(format!("series_plan plan must be \"A\" or \"B\", got `{plan}`"));
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
            ));
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

    fn export(&mut self, body: &Value) -> Result<Value, String> {
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
        let hash = omo_engine::export::export_sql_with_rows(
            conn, &sql, &target, format, snapshot, confirmed, known_rows,
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

    // Howard Hinnant's days_from_civil algorithm (days since 1970-01-01).
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = u64::from(month);
    let d = u64::from(day);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    Ok(days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second))
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
        assert!(parse_window_utc(&json!(["2024-02-29T00:00:00+08:00", "2024-03-01T00:00:00Z"])).is_err());
    }
}
