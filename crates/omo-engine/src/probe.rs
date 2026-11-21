use duckdb::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    bootstrap::BootstrapError,
    config::{BootstrapConfig, Channel},
};

pub const REQUIRED_FUNCTIONS: [&str; 4] = [
    "read_om",
    "read_om_series",
    "om_grid_info",
    "duckomo_last_scan_metrics",
];
pub const EXPECTED_DUCKDB_VERSION: &str = "v2.0.0";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProbeReport {
    pub duckdb_version: String,
    pub platform: String,
    pub duckomo_version: String,
    pub channel: Channel,
    pub functions_present: Vec<String>,
}

pub fn verify_extension_version(expected: &str, actual: &str) -> Result<(), BootstrapError> {
    if expected == actual {
        return Ok(());
    }
    Err(BootstrapError::ProbeFailed {
        probe: "duckomo_version",
        expected: expected.to_owned(),
        actual: actual.to_owned(),
    })
}

pub fn verify_duckdb_version(actual: &str) -> Result<(), BootstrapError> {
    if actual == EXPECTED_DUCKDB_VERSION {
        return Ok(());
    }
    Err(BootstrapError::ProbeFailed {
        probe: "duckdb_version",
        expected: EXPECTED_DUCKDB_VERSION.to_owned(),
        actual: actual.to_owned(),
    })
}

pub fn verify_platform(expected: &str, actual: &str) -> Result<(), BootstrapError> {
    if expected == actual {
        return Ok(());
    }
    Err(BootstrapError::ProbeFailed {
        probe: "platform",
        expected: expected.to_owned(),
        actual: actual.to_owned(),
    })
}

pub(crate) fn collect_bootstrap_probes(
    conn: &Connection,
    config: &BootstrapConfig,
    channel: Channel,
) -> Result<ProbeReport, BootstrapError> {
    let duckdb_version = conn
        .query_row("SELECT version()", [], |row| row.get::<_, String>(0))
        .map_err(|error| BootstrapError::DuckDb {
            stage: "duckdb_version",
            reason: error.to_string(),
        })?;
    verify_duckdb_version(&duckdb_version)?;

    let platform = conn
        .query_row("PRAGMA platform", [], |row| row.get::<_, String>(0))
        .map_err(|error| BootstrapError::DuckDb {
            stage: "platform",
            reason: error.to_string(),
        })?;
    verify_platform(config.platform.as_str(), &platform)?;

    let duckomo_version = conn
        .query_row(
            "SELECT extension_version FROM duckdb_extensions() WHERE extension_name = 'duckomo'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|error| BootstrapError::ProbeFailed {
            probe: "duckomo_version",
            expected: config.artifact.version.clone(),
            actual: format!("duckomo is not installed: {error}"),
        })?
        .ok_or_else(|| BootstrapError::ProbeFailed {
            probe: "duckomo_version",
            expected: config.artifact.version.clone(),
            actual: "DuckDB reported no extension version".to_owned(),
        })?;
    verify_extension_version(&config.artifact.version, &duckomo_version)?;

    let functions_present = verify_required_functions(conn)?;
    Ok(ProbeReport {
        duckdb_version,
        platform,
        duckomo_version,
        channel,
        functions_present,
    })
}

pub(crate) fn verify_required_functions(conn: &Connection) -> Result<Vec<String>, BootstrapError> {
    let mut statement = conn
        .prepare("SELECT function_name FROM duckdb_functions() WHERE function_name = ? LIMIT 1")
        .map_err(|error| BootstrapError::DuckDb {
            stage: "function_probe",
            reason: error.to_string(),
        })?;
    let mut present = Vec::with_capacity(REQUIRED_FUNCTIONS.len());
    let mut missing = Vec::new();
    for name in REQUIRED_FUNCTIONS {
        let found = statement
            .query_map([name], |row| row.get::<_, String>(0))
            .map_err(|error| BootstrapError::DuckDb {
                stage: "function_probe",
                reason: error.to_string(),
            })?
            .next()
            .transpose()
            .map_err(|error| BootstrapError::DuckDb {
                stage: "function_probe",
                reason: error.to_string(),
            })?;
        if found.is_some() {
            present.push(name.to_owned());
        } else {
            missing.push(name);
        }
    }
    if !missing.is_empty() {
        return Err(BootstrapError::ProbeFailed {
            probe: "required_functions",
            expected: REQUIRED_FUNCTIONS.join(", "),
            actual: format!("missing {}", missing.join(", ")),
        });
    }
    Ok(present)
}

pub(crate) fn golden_result_hash(
    conn: &Connection,
    fixture_path: &std::path::Path,
) -> Result<(String, usize), BootstrapError> {
    let fixture = fixture_path.to_string_lossy().replace('\'', "''");
    let sql =
        format!("SELECT value FROM read_om('{fixture}') ORDER BY value ASC NULLS FIRST LIMIT 64");
    let mut statement = conn.prepare(&sql).map_err(|error| BootstrapError::DuckDb {
        stage: "golden_query",
        reason: error.to_string(),
    })?;
    let rows = statement
        .query_map([], |row| row.get::<_, Option<f32>>(0))
        .map_err(|error| BootstrapError::DuckDb {
            stage: "golden_query",
            reason: error.to_string(),
        })?;
    let mut hasher = Sha256::new();
    let mut count = 0usize;
    for row in rows {
        let value = row.map_err(|error| BootstrapError::DuckDb {
            stage: "golden_query",
            reason: error.to_string(),
        })?;
        match value {
            Some(value) => {
                hasher.update([1]);
                hasher.update(value.to_bits().to_be_bytes());
            }
            None => hasher.update([0]),
        }
        count += 1;
    }
    if count == 0 {
        return Err(BootstrapError::ProbeFailed {
            probe: "golden_query",
            expected: "at least one ordered result row".to_owned(),
            actual: "zero rows".to_owned(),
        });
    }
    Ok((hex::encode(hasher.finalize()), count))
}
