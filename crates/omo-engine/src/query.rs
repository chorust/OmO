use std::path::Path;

use omo_protocol::result_ref::{ResultLimits, ResultRef};
use thiserror::Error;

use crate::{
    bootstrap::BootstrapError,
    conn::BootstrappedConn,
    limits::{interactive_limits, job_limits, validate_limits},
    results::{ResultWriteError, write_arrow_batches},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryPurpose {
    Preview,
    Job,
}

#[derive(Debug, Error)]
pub enum QueryError {
    #[error(transparent)]
    Bootstrap(#[from] BootstrapError),
    #[error(transparent)]
    ResultWrite(#[from] ResultWriteError),
    #[error("DuckDB query failed: {0}")]
    DuckDb(String),
}

/// Execute a query using only a bootstrapped connection and stream its batches to a
/// generation-scoped Arrow IPC file.
pub fn execute_query(
    conn: &BootstrappedConn,
    sql: &str,
    purpose: QueryPurpose,
    limits: ResultLimits,
    session_tmp: &Path,
    result_id: &str,
) -> Result<ResultRef, QueryError> {
    let cap = match purpose {
        QueryPurpose::Preview => interactive_limits(),
        QueryPurpose::Job => job_limits(),
    };
    let limits = ResultLimits {
        rows: limits.rows.min(cap.rows),
        bytes: limits.bytes.min(cap.bytes),
    };
    validate_limits(limits).map_err(ResultWriteError::from)?;
    if sql_requires_remote(sql) {
        conn.require_remote()?;
    }
    let connection = conn.connection()?;
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| QueryError::DuckDb(error.to_string()))?;
    let stream = statement
        .stream_arrow([])
        .map_err(|error| QueryError::DuckDb(error.to_string()))?;
    let schema = stream.get_schema();
    let result = write_arrow_batches(
        schema,
        stream,
        session_tmp,
        result_id,
        conn.generation(),
        limits,
    )?;
    Ok(result)
}

pub(crate) fn sql_requires_remote(sql: &str) -> bool {
    let normalized = sql.to_ascii_lowercase();
    ["http://", "https://", "s3://"]
        .iter()
        .any(|scheme| normalized.contains(scheme))
}

/// Bind `stations` (and optional `station_series`) temporary views on a bootstrapped connection.
pub fn setup_sql_session(
    conn: &BootstrappedConn,
    stations_csv_path: &str,
    series_parquet_path: Option<&str>,
) -> Result<Vec<String>, QueryError> {
    let db = conn.connection()?;
    let stations_lit = crate::series::quote_sql_literal(stations_csv_path);
    let create_stations = format!(
        "CREATE OR REPLACE TEMP VIEW stations AS SELECT * FROM read_csv({stations_lit}, header=true, auto_detect=true)"
    );
    db.execute_batch(&create_stations)
        .map_err(|error| QueryError::DuckDb(error.to_string()))?;
    let mut views = vec!["stations".to_owned()];
    if let Some(parquet) = series_parquet_path {
        let parquet_lit = crate::series::quote_sql_literal(parquet);
        let create_series = format!(
            "CREATE OR REPLACE TEMP VIEW station_series AS SELECT * FROM read_parquet({parquet_lit})"
        );
        db.execute_batch(&create_series)
            .map_err(|error| QueryError::DuckDb(error.to_string()))?;
        views.push("station_series".to_owned());
    } else {
        let _ = db.execute_batch("DROP VIEW IF EXISTS station_series");
    }
    Ok(views)
}
