//! Contract tests for `omo-project/1` file persistence and round-trip fixture (T052).

use std::{fs, path::PathBuf};

use omo_project::doc::{ProjectDoc, ProjectFileError};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

#[test]
fn roundtrip_fixture_opens_with_golden_stations_hash_and_saves_atomically() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/projects/m1-roundtrip.omoproj");
    let opened = match ProjectDoc::open_from_path(&fixture) {
        Ok(o) => o,
        Err(err) => {
            let bytes = include_bytes!("../../../fixtures/stations/m1-100.csv");
            let parsed = omo_project::stations::import::import_csv(
                bytes,
                "fixtures/stations/m1-100.csv",
                "2026-11-23T07:00:00Z",
            )
            .unwrap();
            let snap = omo_project::stations::import::confirm(
                &parsed,
                parsed.proposed_mapping.clone(),
                "fixtures/stations/m1-100.csv",
                bytes,
                "2026-11-23T07:00:00Z",
            )
            .unwrap();
            let raw_doc: ProjectDoc = serde_json::from_slice(&fs::read(&fixture).unwrap()).unwrap();
            let raw_rows = raw_doc.stations.unwrap().rows;
            let a = String::from_utf8_lossy(&omo_project::stations::import::canonical_bytes(
                &snap.rows,
            ))
            .into_owned();
            let b =
                String::from_utf8_lossy(&omo_project::stations::import::canonical_bytes(&raw_rows))
                    .into_owned();
            for (i, (la, lb)) in a.lines().zip(b.lines()).enumerate() {
                if la != lb {
                    panic!("err={err:?} line {i}: expected={la:?} actual={lb:?}");
                }
            }
            panic!(
                "err={err:?} same lines? a.len={} b.len={}",
                a.len(),
                b.len()
            );
        }
    };
    let mut doc = opened.doc;
    let stations = doc.stations.as_ref().unwrap();
    assert_eq!(
        stations.content_sha256,
        "8dfb18f5c208e6f32882bdfd841f72019af98749cae0e9620db49dbf245e0dfd"
    );
    assert_eq!(stations.rows.len(), 100);
    assert_eq!(doc.catalog_selection.entry_id, "ecmwf_ifs.temperature_2m");
    assert_eq!(doc.window.start_h, 0);
    assert_eq!(doc.window.end_h, 168);
    assert_eq!(doc.max_distance_m, 7200);
    assert_eq!(doc.sql_tabs.len(), 1);
    assert_eq!(doc.view.selected_station_ids, vec!["S001", "S002"]);

    let dir = tempdir().unwrap();
    let target = dir.path().join("saved.omoproj");
    doc.save_to_path(&target, None).unwrap();
    for entry in fs::read_dir(dir.path()).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!name.starts_with('.'), "leftover temp file: {name}");
    }
    let reopened = ProjectDoc::open_from_path(&target).unwrap().doc;
    assert_eq!(reopened.stations, doc.stations);
    assert_eq!(reopened.catalog_selection, doc.catalog_selection);
    assert_eq!(reopened.window, doc.window);
    assert_eq!(reopened.max_distance_m, doc.max_distance_m);
    assert_eq!(reopened.sql_tabs, doc.sql_tabs);
    assert_eq!(reopened.view, doc.view);

    // Unsupported major schema refuses and preserves bytes verbatim.
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
    json["schema"] = serde_json::json!("omo-project/2");
    let bytes = serde_json::to_vec_pretty(&json).unwrap();
    fs::write(&target, &bytes).unwrap();
    let before_sha = format!("{:x}", Sha256::digest(&bytes));
    assert!(matches!(
        ProjectDoc::open_from_path(&target),
        Err(ProjectFileError::UnsupportedSchema(_))
    ));
    let after_sha = format!("{:x}", Sha256::digest(fs::read(&target).unwrap()));
    assert_eq!(before_sha, after_sha);
}
