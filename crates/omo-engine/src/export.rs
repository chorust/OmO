//! Atomic publication of already staged export data and its sidecar.
use crate::{conn::BootstrappedConn, query::sql_requires_remote};
pub use omo_protocol::export::{SIDECAR_SCHEMA, STAGING_OWNER_MARKER, sidecar_path};
use sha2::{Digest, Sha256};
use std::{fs, io, io::Read, path::Path};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportStep {
    OldSidecarMoved,
    DataMoved,
    SidecarMoved,
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("overwrite confirmation is required")]
    OverwriteConfirmationRequired,
    #[error("staging and target must be on the same filesystem")]
    CrossFilesystem,
    #[error("invalid export target or staging directory")]
    InvalidPath,
    #[error("export file operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("export query failed: {0}")]
    Query(String),
}

fn rename(from: &Path, to: &Path) -> Result<(), ExportError> {
    fs::rename(from, to).map_err(|error| {
        if error.kind() == io::ErrorKind::CrossesDevices {
            ExportError::CrossFilesystem
        } else {
            ExportError::Io(error)
        }
    })
}

#[cfg(unix)]
fn same_filesystem(source: &Path, destination: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(fs::metadata(source)?.dev() == fs::metadata(destination)?.dev())
}

#[cfg(not(unix))]
fn same_filesystem(_source: &Path, _destination: &Path) -> io::Result<bool> {
    // The rename itself still maps a platform EXDEV error to CrossFilesystem.
    Ok(true)
}

/// Publish staged `data` and `sidecar`, invoking `after_step` after each commit boundary.
/// Staging must be a direct child of the target directory, so a rename never requires copying.
pub fn commit_staged(
    target: &Path,
    staging: &Path,
    overwrite_confirmed: bool,
    mut after_step: impl FnMut(ExportStep) -> io::Result<()>,
) -> Result<(), ExportError> {
    let parent = target.parent().ok_or(ExportError::InvalidPath)?;
    let name = target.file_name().ok_or(ExportError::InvalidPath)?;
    let mut staging_prefix = std::ffi::OsString::from(".");
    staging_prefix.push(name);
    staging_prefix.push(".omo-tmp-");
    let valid_name = staging
        .file_name()
        .and_then(|n| n.to_str())
        .zip(staging_prefix.to_str())
        .is_some_and(|(n, prefix)| {
            n.strip_prefix(prefix)
                .is_some_and(|suffix| !suffix.is_empty())
        });
    if !valid_name || !fs::symlink_metadata(staging)?.file_type().is_dir() {
        return Err(ExportError::InvalidPath);
    }
    match fs::read(staging.join(".omo-owner")) {
        Ok(marker) if marker == STAGING_OWNER_MARKER => {}
        _ => return Err(ExportError::InvalidPath),
    }
    if fs::canonicalize(staging.parent().ok_or(ExportError::InvalidPath)?)?
        != fs::canonicalize(parent)?
        || !same_filesystem(staging, parent)?
    {
        return Err(ExportError::CrossFilesystem);
    }
    let sidecar = sidecar_path(target);
    let mut old_name = std::ffi::OsString::from(".");
    old_name.push(name);
    old_name.push(".omo-old");
    let old = parent.join(old_name);
    let data = staging.join("data");
    let staged_sidecar = staging.join("sidecar");
    if (target.exists() || sidecar.exists()) && !overwrite_confirmed {
        return Err(ExportError::OverwriteConfirmationRequired);
    }
    if old.exists() {
        return Err(ExportError::Io(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "previous export requires recovery",
        )));
    }
    fs::File::open(&data)?.sync_all()?;
    fs::File::open(&staged_sidecar)?.sync_all()?;
    if sidecar.exists() {
        rename(&sidecar, &old)?;
    }
    after_step(ExportStep::OldSidecarMoved)?;
    rename(&data, target)?;
    after_step(ExportStep::DataMoved)?;
    rename(&staged_sidecar, &sidecar)?;
    fs::File::open(parent)?.sync_all()?;
    after_step(ExportStep::SidecarMoved)?;
    fs::remove_dir_all(staging)?;
    if old.exists() {
        fs::remove_file(old)?;
    }
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

/// Export SQL through the verified connection and publish the data with a provenance sidecar.
pub fn export_sql(
    conn: &BootstrappedConn,
    sql: &str,
    target: &Path,
    format: &str,
    job_snapshot: serde_json::Value,
    overwrite_confirmed: bool,
) -> Result<String, ExportError> {
    export_sql_with_rows(
        conn,
        sql,
        target,
        format,
        job_snapshot,
        overwrite_confirmed,
        None,
    )
}

pub fn export_sql_with_rows(
    conn: &BootstrappedConn,
    sql: &str,
    target: &Path,
    format: &str,
    job_snapshot: serde_json::Value,
    overwrite_confirmed: bool,
    row_count: Option<u64>,
) -> Result<String, ExportError> {
    export_sql_with_progress(
        conn,
        sql,
        target,
        format,
        job_snapshot,
        overwrite_confirmed,
        row_count,
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
pub fn export_sql_with_progress(
    conn: &BootstrappedConn,
    sql: &str,
    target: &Path,
    format: &str,
    job_snapshot: serde_json::Value,
    overwrite_confirmed: bool,
    row_count: Option<u64>,
    mut on_stage: impl FnMut(&'static str),
) -> Result<String, ExportError> {
    let format = match format {
        "csv" => "CSV",
        "parquet" => "PARQUET",
        _ => return Err(ExportError::Query("unsupported export format".into())),
    };
    let parent = target.parent().ok_or(ExportError::InvalidPath)?;
    let name = target
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or(ExportError::InvalidPath)?;
    if (target.exists() || sidecar_path(target).exists()) && !overwrite_confirmed {
        return Err(ExportError::OverwriteConfirmationRequired);
    }
    on_stage("writing");
    let staging = tempfile::Builder::new()
        .prefix(&format!(".{name}.omo-tmp-"))
        .tempdir_in(parent)?;
    fs::write(staging.path().join(".omo-owner"), STAGING_OWNER_MARKER)?;
    let data = staging.path().join("data");
    let escaped = data.to_string_lossy().replace('\'', "''");
    if sql_requires_remote(sql) {
        conn.require_remote()
            .map_err(|e| ExportError::Query(e.to_string()))?;
    }
    let statement = format!("COPY ({sql}) TO '{escaped}' (FORMAT {format})");
    conn.connection()
        .map_err(|e| ExportError::Query(e.to_string()))?
        .execute_batch(&statement)
        .map_err(|e| ExportError::Query(e.to_string()))?;
    let mut file = fs::File::open(&data)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    let hash = format!("{:x}", digest.finalize());
    let report = conn.report();
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ExportError::InvalidPath)?;
    let created_at = now_utc_rfc3339();
    let sidecar = if let Some(m1) = job_snapshot.get("m1_sidecar") {
        serde_json::json!({
            "schema": omo_protocol::export::SIDECAR_SCHEMA_V2,
            "job": job_snapshot,
            "bootstrap_report": report,
            "duckomo": {"version": report.duckomo_version, "channel": report.channel.as_str(), "sha256": report.artifact_sha256},
            "duckdb_version": report.duckdb_version,
            "data": {"file": file_name, "sha256": hash, "format": format, "rows": row_count},
            "stations": m1.get("stations").cloned().unwrap_or(serde_json::Value::Null),
            "catalog": m1.get("catalog").cloned().unwrap_or(serde_json::Value::Null),
            "run": m1.get("run").cloned().unwrap_or(serde_json::Value::Null),
            "window": m1.get("window").cloned().unwrap_or(serde_json::Value::Null),
            "plan": m1.get("plan").cloned().unwrap_or(serde_json::Value::Null),
            "query": m1.get("query").cloned().unwrap_or(serde_json::Value::Null),
            "attribution": m1.get("attribution").cloned().unwrap_or_else(|| serde_json::json!(["Weather data by Open-Meteo.com (CC BY 4.0)"])),
            "created_at": created_at
        })
    } else {
        serde_json::json!({
            "schema": SIDECAR_SCHEMA,
            "job": job_snapshot,
            "bootstrap_report": report,
            "duckomo": {"version": report.duckomo_version, "channel": report.channel.as_str(), "sha256": report.artifact_sha256},
            "duckdb_version": report.duckdb_version,
            "data": {"file": file_name, "sha256": hash, "format": format, "rows": row_count},
            "attribution": "Open-Meteo (CC BY 4.0), https://open-meteo.com/",
            "created_at": created_at
        })
    };
    fs::write(
        staging.path().join("sidecar"),
        serde_json::to_vec_pretty(&sidecar).map_err(|e| ExportError::Query(e.to_string()))?,
    )?;
    fs::File::open(staging.path())?.sync_all()?;
    on_stage("committing");
    // Persist the staging directory so failures after the first commit step remain recoverable.
    let staging = staging.keep();
    commit_staged(target, &staging, overwrite_confirmed, |_| Ok(()))?;
    Ok(hash)
}

fn now_utc_rfc3339() -> String {
    omo_protocol::utc::now_utc_rfc3339()
}

#[cfg(test)]
mod utc_tests {
    use omo_protocol::utc::format_utc_rfc3339;

    #[test]
    fn format_utc_rfc3339_handles_epoch_leap_and_year_boundaries() {
        assert_eq!(format_utc_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc_rfc3339(1_709_251_199), "2024-02-29T23:59:59Z");
        assert_eq!(format_utc_rfc3339(1_709_251_200), "2024-03-01T00:00:00Z");
        assert_eq!(format_utc_rfc3339(1_798_761_599), "2026-12-31T23:59:59Z");
        assert_eq!(format_utc_rfc3339(1_798_761_600), "2027-01-01T00:00:00Z");
        assert_eq!(format_utc_rfc3339(4_107_542_399), "2100-02-28T23:59:59Z");
        assert_eq!(format_utc_rfc3339(4_107_542_400), "2100-03-01T00:00:00Z");
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{ExportError, rename};
    use std::{fs, os::unix::fs::MetadataExt};
    use tempfile::tempdir;

    #[test]
    fn rename_maps_kernel_exdev_to_cross_filesystem() {
        let Some(destination_root) = std::path::Path::new("/dev/shm")
            .exists()
            .then(|| tempdir_in_shm())
            .flatten()
        else {
            return;
        };
        let source_root = tempdir().unwrap();
        if fs::metadata(source_root.path()).unwrap().dev()
            == fs::metadata(destination_root.path()).unwrap().dev()
        {
            return;
        }
        let source = source_root.path().join("staged-data");
        let destination = destination_root.path().join("target-data");
        fs::write(&source, b"data").unwrap();
        assert!(matches!(
            rename(&source, &destination),
            Err(ExportError::CrossFilesystem)
        ));
        assert!(source.exists());
        assert!(!destination.exists());
    }

    fn tempdir_in_shm() -> Option<tempfile::TempDir> {
        tempfile::Builder::new()
            .prefix("omo-exdev-test-")
            .tempdir_in("/dev/shm")
            .ok()
    }
}
