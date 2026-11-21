use std::{fs, io};

use omo_app::host::export_scan::{ExportVerdict, is_saved, scan_export_dir};
use omo_engine::export::{
    ExportStep, SIDECAR_SCHEMA, STAGING_OWNER_MARKER, commit_staged, sidecar_path,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

#[test]
fn matching_sidecar_cleans_finished_leftovers() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("report.csv");
    let data = b"a,b\n1,2\n";
    fs::write(&target, data).unwrap();
    let hash = format!("{:x}", Sha256::digest(data));
    fs::write(
        sidecar_path(&target),
        serde_json::to_vec(&json!({"schema":SIDECAR_SCHEMA,"data":{"sha256":hash}})).unwrap(),
    )
    .unwrap();
    let staging = dir.path().join(".report.csv.omo-tmp-123");
    fs::create_dir(&staging).unwrap();
    fs::write(staging.join(".omo-owner"), STAGING_OWNER_MARKER).unwrap();
    fs::write(
        dir.path().join(".report.csv.omo-old"),
        r#"{"schema":"omo-export-sidecar/1"}"#,
    )
    .unwrap();
    let entries = scan_export_dir(dir.path()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].verdict, ExportVerdict::Finished);
    assert!(!dir.path().join(".report.csv.omo-tmp-123").exists());
    assert!(!dir.path().join(".report.csv.omo-old").exists());
    assert!(scan_export_dir(dir.path()).unwrap().is_empty());
}

#[test]
fn mismatched_or_unknown_sidecar_preserves_incomplete_files() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("report.csv");
    fs::write(&target, b"new data").unwrap();
    fs::write(
        sidecar_path(&target),
        r#"{"schema":"future","data":{"sha256":"bad"}}"#,
    )
    .unwrap();
    let staging = dir.path().join(".report.csv.omo-tmp-123");
    fs::create_dir(&staging).unwrap();
    fs::write(staging.join(".omo-owner"), STAGING_OWNER_MARKER).unwrap();
    let entries = scan_export_dir(dir.path()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].verdict, ExportVerdict::Incomplete);
    assert!(staging.exists());
    assert!(target.exists());
    assert!(!is_saved(&target).unwrap());
}

#[test]
fn recovery_ignores_unowned_lookalike_paths() {
    let dir = tempdir().unwrap();
    let staging = dir.path().join(".report.csv.omo-tmp-not-owned");
    fs::create_dir(&staging).unwrap();
    fs::write(staging.join("data"), b"unrelated").unwrap();
    let old = dir.path().join(".notes.omo-old");
    fs::write(&old, b"unrelated").unwrap();

    assert!(scan_export_dir(dir.path()).unwrap().is_empty());
    assert!(staging.exists());
    assert!(old.exists());
}

fn make_staging(dir: &std::path::Path, suffix: &str, bytes: &[u8]) -> std::path::PathBuf {
    let staging = dir.join(format!(".report.csv.omo-tmp-{suffix}"));
    fs::create_dir(&staging).unwrap();
    fs::write(staging.join(".omo-owner"), STAGING_OWNER_MARKER).unwrap();
    fs::write(staging.join("data"), bytes).unwrap();
    let hash = format!("{:x}", Sha256::digest(bytes));
    fs::write(
        staging.join("sidecar"),
        serde_json::to_vec(&json!({"schema":"omo-export-sidecar/1","data":{"sha256":hash}}))
            .unwrap(),
    )
    .unwrap();
    staging
}

#[test]
fn kill_injection_at_each_commit_step_has_the_contract_recovery_verdict() {
    for (suffix, killed_at, expected) in [
        (
            "step1",
            ExportStep::OldSidecarMoved,
            ExportVerdict::Incomplete,
        ),
        ("step2", ExportStep::DataMoved, ExportVerdict::Incomplete),
        ("step3", ExportStep::SidecarMoved, ExportVerdict::Finished),
    ] {
        let dir = tempdir().unwrap();
        let target = dir.path().join("report.csv");
        fs::write(&target, b"old data").unwrap();
        let old_hash = format!("{:x}", Sha256::digest(b"old data"));
        fs::write(
            sidecar_path(&target),
            serde_json::to_vec(
                &json!({"schema":"omo-export-sidecar/1","data":{"sha256":old_hash}}),
            )
            .unwrap(),
        )
        .unwrap();
        let staging = make_staging(dir.path(), suffix, b"new data");
        let result = commit_staged(&target, &staging, true, |step| {
            if step == killed_at {
                Err(io::Error::other("injected process kill"))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        let entries = scan_export_dir(dir.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].verdict, expected);
        match expected {
            ExportVerdict::Incomplete => assert!(staging.exists()),
            ExportVerdict::Finished => {
                assert!(!staging.exists());
                assert!(is_saved(&target).unwrap());
            }
        }
    }
}
