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

#[test]
fn sidecar_v2_is_accepted_when_valid_and_incomplete_when_interrupted() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("series.csv");
    let data =
        b"station_id,variable,valid_time,value\nA1,temperature_2m,2026-11-23T00:00:00Z,5.0\n";
    fs::write(&target, data).unwrap();
    let hash = format!("{:x}", Sha256::digest(data));
    fs::write(
        sidecar_path(&target),
        serde_json::to_vec(
            &json!({"schema": omo_protocol::export::SIDECAR_SCHEMA_V2, "data": {"sha256": hash}}),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(is_saved(&target).unwrap());

    let incomplete_target = dir.path().join("interrupted.csv");
    fs::write(&incomplete_target, data).unwrap();
    let staging = dir.path().join(".interrupted.csv.omo-tmp-v2");
    fs::create_dir(&staging).unwrap();
    fs::write(staging.join(".omo-owner"), STAGING_OWNER_MARKER).unwrap();
    let entries = scan_export_dir(dir.path()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].target, incomplete_target);
    assert_eq!(entries[0].verdict, ExportVerdict::Incomplete);
}

/// A composite map PNG with an `omo-export-sidecar/3` is saved exactly like any other export,
/// and an interrupted one classifies as `incomplete` / `incomplete` / `finished` (`SC-007`).
#[test]
fn map_png_with_sidecar_v3_is_saved_and_kill_steps_classify_like_m0() {
    let png = b"\x89PNG\r\n\x1a\nnot a real image but hashed like one";
    let hash = format!("{:x}", Sha256::digest(png));
    let sidecar = |hash: &str| {
        serde_json::to_vec(&json!({
            "schema": omo_protocol::export::SIDECAR_SCHEMA_V3,
            "data": {"file": "map.png", "format": "png", "sha256": hash}
        }))
        .unwrap()
    };

    let dir = tempdir().unwrap();
    let target = dir.path().join("map.png");
    fs::write(&target, png).unwrap();
    fs::write(sidecar_path(&target), sidecar(&hash)).unwrap();
    assert!(is_saved(&target).unwrap());
    // A sidecar whose digest does not match the bytes never counts as saved.
    fs::write(sidecar_path(&target), sidecar("00")).unwrap();
    assert!(!is_saved(&target).unwrap());

    for (suffix, killed_at, expected) in [
        ("s1", ExportStep::OldSidecarMoved, ExportVerdict::Incomplete),
        ("s2", ExportStep::DataMoved, ExportVerdict::Incomplete),
        ("s3", ExportStep::SidecarMoved, ExportVerdict::Finished),
    ] {
        let dir = tempdir().unwrap();
        let target = dir.path().join("map.png");
        let staging = dir.path().join(format!(".map.png.omo-tmp-{suffix}"));
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join(".omo-owner"), STAGING_OWNER_MARKER).unwrap();
        fs::write(staging.join("data"), png).unwrap();
        fs::write(staging.join("sidecar"), sidecar(&hash)).unwrap();
        let result = commit_staged(&target, &staging, false, |step| {
            if step == killed_at {
                Err(io::Error::other("injected process kill"))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        let entries = scan_export_dir(dir.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].target, target);
        assert_eq!(entries[0].verdict, expected);
        assert_eq!(
            is_saved(&target).unwrap(),
            expected == ExportVerdict::Finished
        );
    }
}
