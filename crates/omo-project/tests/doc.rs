use std::fs;
use std::path::PathBuf;

use omo_project::{
    doc::{ProjectDoc, ProjectFileError, SqlTab},
    draft,
    preview_key::PreviewKey,
    stations::{ColumnMapping, import},
};
use tempfile::tempdir;

fn key(revision: u64) -> PreviewKey {
    PreviewKey {
        doc_id: "doc-1".into(),
        revision,
        view_id: "sql-1".into(),
        query_hash: "query-hash".into(),
        stations_hash: None,
        catalog_version: "catalog-0".into(),
        run_ref: "latest_complete".into(),
        window: omo_project::preview_key::PreviewWindow::default(),
        max_distance_m: 7200,
        worker_generation: 3,
    }
}

#[test]
fn revision_and_dirty_follow_edits_and_saves() {
    let mut doc = ProjectDoc::new("doc-1");
    assert_eq!(doc.revision(), 0);
    assert!(!doc.is_dirty());
    doc.record_edit().unwrap();
    assert_eq!(doc.revision(), 1);
    assert!(doc.is_dirty());
    doc.mark_saved();
    assert!(!doc.is_dirty());
    doc.record_edit().unwrap();
    assert!(doc.is_dirty());
}

#[test]
fn preview_key_equality_covers_every_input() {
    assert_eq!(key(2), key(2));
    assert_ne!(key(2), key(3));
    let mut changed = key(2);
    changed.worker_generation += 1;
    assert_ne!(key(2), changed);
}

#[test]
fn project_save_open_round_trip_and_tamper_detection() {
    let dir = tempdir().unwrap();
    let csv_path = dir.path().join("stations.csv");
    let orig_bytes = b"station_id,lat,lon,name\nA1,39.9,116.4,Beijing\n";
    fs::write(&csv_path, orig_bytes).unwrap();
    let parsed = import::import_csv(
        orig_bytes,
        csv_path.to_str().unwrap(),
        "2026-11-23T07:00:00Z",
    )
    .unwrap();
    let snapshot = import::confirm(
        &parsed,
        ColumnMapping {
            id: "station_id".into(),
            lat: "lat".into(),
            lon: "lon".into(),
            name: Some("name".into()),
            elevation: None,
        },
        csv_path.to_str().unwrap(),
        orig_bytes,
        "2026-11-23T07:00:00Z",
    )
    .unwrap();
    let expected_hash = snapshot.content_sha256.clone();

    let mut doc = ProjectDoc::new("doc-roundtrip");
    doc.stations = Some(snapshot);
    doc.sql_tabs.push(SqlTab {
        title: "Daily max".into(),
        sql: "SELECT station_id, MAX(value) FROM station_series GROUP BY station_id".into(),
    });
    doc.view.selected_station_ids = vec!["A1".into()];
    doc.record_edit().unwrap();

    let app_data = dir.path().join("app_data");
    draft::stage(&doc, &app_data, "2026-11-23T07:05:00Z").unwrap();
    assert_eq!(draft::list_recoverable(&app_data).unwrap().len(), 1);

    let proj_path = dir.path().join("test.omoproj");
    doc.save_to_path(&proj_path, Some(&app_data)).unwrap();
    assert!(!doc.is_dirty());
    assert_eq!(doc.path(), Some(PathBuf::from(&proj_path).as_path()));
    assert!(draft::list_recoverable(&app_data).unwrap().is_empty());

    // Modify original CSV -> informational notice, snapshot kept.
    fs::write(&csv_path, b"station_id,lat,lon,name\nB2,0,0,Other\n").unwrap();
    let opened = ProjectDoc::open_from_path(&proj_path).unwrap();
    assert!(opened.source_changed_notice.is_some());
    assert_eq!(
        opened.doc.stations.as_ref().unwrap().content_sha256,
        expected_hash
    );

    // Tamper inline row -> CorruptSnapshot error.
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(&proj_path).unwrap()).unwrap();
    json["stations"]["rows"][0]["latitude"] = serde_json::json!(40.0);
    fs::write(&proj_path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
    assert!(matches!(
        ProjectDoc::open_from_path(&proj_path),
        Err(ProjectFileError::CorruptSnapshot { .. })
    ));

    // Newer schema -> UnsupportedSchema error without modifying file.
    json["schema"] = serde_json::json!("omo-project/2");
    fs::write(&proj_path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
    assert!(matches!(
        ProjectDoc::open_from_path(&proj_path),
        Err(ProjectFileError::UnsupportedSchema(_))
    ));
}
