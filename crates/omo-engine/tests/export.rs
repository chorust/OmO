use std::{fs, io};

use omo_engine::{
    conn::BootstrappedConn,
    export::{
        ExportError, ExportStep, SIDECAR_SCHEMA, STAGING_OWNER_MARKER, commit_staged, export_sql,
        export_sql_with_rows, sidecar_path,
    },
    test_support::new_unbootstrapped_conn,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

fn staged(
    dir: &std::path::Path,
    target_name: &str,
    suffix: &str,
    data: &[u8],
) -> std::path::PathBuf {
    let path = dir.join(format!(".{target_name}.omo-tmp-{suffix}"));
    fs::create_dir(&path).unwrap();
    fs::write(path.join(".omo-owner"), STAGING_OWNER_MARKER).unwrap();
    fs::write(path.join("data"), data).unwrap();
    let hash = format!("{:x}", Sha256::digest(data));
    fs::write(
        path.join("sidecar"),
        serde_json::to_vec(&json!({
            "schema": SIDECAR_SCHEMA, "data":{"sha256":hash}
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

#[test]
fn overwrite_invalidates_old_sidecar_before_replacing_data() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("report.csv");
    fs::write(&target, b"old data").unwrap();
    fs::write(sidecar_path(&target), b"old sidecar").unwrap();
    let stage = staged(dir.path(), "report.csv", "overwrite", b"new data");
    let result = commit_staged(&target, &stage, false, |_| Ok(()));
    assert!(matches!(
        result,
        Err(ExportError::OverwriteConfirmationRequired)
    ));
    assert_eq!(fs::read(sidecar_path(&target)).unwrap(), b"old sidecar");
    let result = commit_staged(&target, &stage, true, |step| {
        if step == ExportStep::OldSidecarMoved {
            Err(io::Error::other("injected"))
        } else {
            Ok(())
        }
    });
    assert!(result.is_err());
    assert!(!sidecar_path(&target).exists());
    assert_eq!(fs::read(&target).unwrap(), b"old data");
    assert!(dir.path().join(".report.csv.omo-old").exists());
}

#[test]
fn cross_directory_staging_is_refused_without_copying() {
    let target_dir = tempdir().unwrap();
    let stage_dir = tempdir().unwrap();
    let target = target_dir.path().join("report.csv");
    let stage = staged(stage_dir.path(), "report.csv", "other-filesystem", b"data");
    let result = commit_staged(&target, &stage, false, |_| Ok(()));
    assert!(matches!(result, Err(ExportError::CrossFilesystem)));
    assert!(!target.exists());
    assert!(stage.exists());
}

#[test]
fn sql_export_commits_csv_and_parquet_with_hash_and_bootstrap_provenance() {
    let dir = tempdir().unwrap();
    let conn: BootstrappedConn = new_unbootstrapped_conn().unwrap();
    for (format, extension) in [("csv", "csv"), ("parquet", "parquet")] {
        let target = dir.path().join(format!("report.{extension}"));
        let hash = export_sql_with_rows(
            &conn,
            "SELECT 1 AS value",
            &target,
            format,
            json!({"job_id":9}),
            false,
            Some(1),
        )
        .unwrap();
        let bytes = fs::read(&target).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), hash);
        let sidecar: serde_json::Value =
            serde_json::from_slice(&fs::read(sidecar_path(&target)).unwrap()).unwrap();
        assert_eq!(sidecar["schema"], "omo-export-sidecar/1");
        assert_eq!(sidecar["bootstrap_report"]["channel"], "community-signed");
        assert_eq!(sidecar["job"]["job_id"], 9);
        assert_eq!(sidecar["data"]["sha256"], hash);
        assert_eq!(sidecar["data"]["format"], format.to_ascii_uppercase());
        assert_eq!(
            sidecar["data"]["file"],
            target.file_name().unwrap().to_str().unwrap()
        );
        assert_eq!(sidecar["data"]["rows"], 1);
        assert!(
            sidecar["attribution"]
                .as_str()
                .unwrap()
                .contains("CC BY 4.0")
        );
        let created_at = sidecar["created_at"]
            .as_str()
            .expect("sidecar must include RFC 3339 UTC created_at");
        assert_eq!(created_at.len(), 20);
        assert!(created_at.ends_with('Z') && created_at.contains('T'));
    }
}

#[test]
fn export_rejects_remote_sql_without_remote_capability() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("remote.csv");
    let conn = new_unbootstrapped_conn().unwrap();
    let result = export_sql(
        &conn,
        "SELECT * FROM read_csv_auto('https://example.invalid/data.csv')",
        &target,
        "csv",
        json!({}),
        false,
    );
    assert!(matches!(result, Err(ExportError::Query(_))));
    assert!(!target.exists());
}
