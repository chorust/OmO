//! `.omoproj` M2 persistence (`T038`, `SC-008`): backward-compatible open of an M1 project,
//! deterministic canonical bytes (`-0.0 → 0.0`), pre-deserialize schema rejection and draft
//! recovery of the map state.

use std::{fs, path::PathBuf};

use omo_project::{
    doc::{ProjectDoc, ProjectFileError},
    draft,
    map::{ColorScaleConfig, MapLayerConfig, MapLayerKind, MapViewState, MapViewport},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/m1-roundtrip.omoproj")
}

fn custom_state() -> MapViewState {
    let mut state = MapViewState {
        viewport: Some(MapViewport {
            center_lat: -0.0,
            center_lon: -0.0,
            zoom: 5,
        }),
        valid_time_lead_h: 6,
        color_scale: ColorScaleConfig {
            palette: "viridis".into(),
            locked_min: Some(-0.0),
            locked_max: Some(30.0),
        },
        ..Default::default()
    };
    state
        .add_local_pmtiles(PathBuf::from("/data/base.pmtiles"))
        .unwrap();
    state.set_layer_opacity("pmtiles-1", -0.0).unwrap();
    state.set_layer_visible("coastlines", false).unwrap();
    state
}

fn sha(path: &std::path::Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

#[test]
fn an_m1_project_opens_with_the_default_map_and_resaves_without_a_map_key() {
    let opened = ProjectDoc::open_from_path(&fixture()).expect("M1 project opens under M2");
    assert!(opened.doc.map.is_default());
    let dir = tempdir().unwrap();
    let target = dir.path().join("copy.omoproj");
    let mut doc = opened.doc;
    doc.save_to_path(&target, None).unwrap();
    let text = fs::read_to_string(&target).unwrap();
    assert!(
        !text.contains("\"map\""),
        "default map state must not be written"
    );
    assert!(text.contains("omo-project/1"));
}

#[test]
fn map_state_round_trips_and_negative_zero_is_canonicalised_to_the_same_bytes() {
    let dir = tempdir().unwrap();
    let a = dir.path().join("a.omoproj");
    let b = dir.path().join("b.omoproj");
    let mut doc = ProjectDoc::open_from_path(&fixture()).unwrap().doc;
    doc.map = custom_state();
    doc.save_to_path(&a, None).unwrap();
    let text = fs::read_to_string(&a).unwrap();
    assert!(
        !text.contains("-0.0"),
        "negative zero leaked into the file: {text}"
    );
    assert!(text.contains("\"map\""));

    // Reopen: identical map, and saving again gives byte-identical output.
    let mut reopened = ProjectDoc::open_from_path(&a).unwrap().doc;
    assert_eq!(reopened.map, custom_state().canonical().unwrap());
    reopened.save_to_path(&b, None).unwrap();
    assert_eq!(sha(&a), sha(&b));

    // The same logical state built with +0.0 hashes identically.
    let mut positive = custom_state();
    positive.viewport = Some(MapViewport {
        center_lat: 0.0,
        center_lon: 0.0,
        zoom: 5,
    });
    positive.color_scale.locked_min = Some(0.0);
    let c = dir.path().join("c.omoproj");
    let mut other = ProjectDoc::open_from_path(&fixture()).unwrap().doc;
    other.map = positive;
    other.save_to_path(&c, None).unwrap();
    assert_eq!(sha(&a), sha(&c));
}

#[test]
fn layer_order_and_archive_paths_survive_in_order() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("p.omoproj");
    let mut doc = ProjectDoc::open_from_path(&fixture()).unwrap().doc;
    doc.map = custom_state();
    doc.save_to_path(&path, None).unwrap();
    let reopened = ProjectDoc::open_from_path(&path).unwrap().doc;
    let ids: Vec<&str> = reopened.map.layers.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, ["coastlines", "forecast", "pmtiles-1", "stations"]);
    assert!(matches!(
        &reopened.map.layers[2].kind,
        MapLayerKind::LocalPmtiles { path } if path == &PathBuf::from("/data/base.pmtiles")
    ));
    assert!(!reopened.map.layers[0].visible);
}

#[test]
fn a_future_major_schema_is_rejected_before_deserialisation() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("future.omoproj");
    // Field types that no v1 reader could parse: the error must still be UnsupportedSchema.
    fs::write(
        &path,
        serde_json::to_vec(&json!({"schema": "omo-project/2", "doc_id": 7, "window": 5})).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        ProjectDoc::open_from_path(&path),
        Err(ProjectFileError::UnsupportedSchema(s)) if s == "omo-project/2"
    ));
}

#[test]
fn an_invalid_map_state_is_reported_not_silently_defaulted() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("bad.omoproj");
    let mut raw: serde_json::Value = serde_json::from_slice(&fs::read(fixture()).unwrap()).unwrap();
    raw["map"] = json!({"color_scale": {"palette": "rainbow-unicorn"}});
    fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
    assert!(matches!(
        ProjectDoc::open_from_path(&path),
        Err(ProjectFileError::InvalidMapState(_))
    ));
    raw["map"] = json!({"layers": [
        {"id": "x", "kind": {"type": "forecast_field"}},
        {"id": "x", "kind": {"type": "station_overlay"}}
    ]});
    fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
    assert!(matches!(
        ProjectDoc::open_from_path(&path),
        Err(ProjectFileError::InvalidMapState(_))
    ));
}

#[test]
fn a_staged_draft_recovers_the_map_state() {
    let dir = tempdir().unwrap();
    let app_data = dir.path().join("app");
    let mut doc = ProjectDoc::new("map-draft");
    doc.map = custom_state();
    doc.record_edit().unwrap();
    draft::stage(&doc, &app_data, "2026-11-26T08:00:00Z").unwrap();
    let recoverable = draft::list_recoverable(&app_data).unwrap();
    assert_eq!(recoverable.len(), 1);
    let recovered = &recoverable[0].draft.doc;
    assert_eq!(recovered.map, custom_state().canonical().unwrap());
    let _ = MapLayerConfig::new("x", MapLayerKind::ForecastField, true, 1.0);
}
