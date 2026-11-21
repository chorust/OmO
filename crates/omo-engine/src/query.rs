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
