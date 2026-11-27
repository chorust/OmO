//! Network-gated live reads for the map field fetcher (`T018`/`T020`).
//!
//! `OMO_LIVE_BOOTSTRAP=/path/bootstrap.json cargo +1.100.0 test -p omo-engine --test map_fetch_live -- --ignored --nocapture`
//! The JSON is a `BootstrapConfig`; `data_dir` is replaced by a fresh temporary directory.
//! `OMO_LIVE_RUN` (default `2026-11-25T00:00:00Z`) and `OMO_LIVE_VALID`
//! (default `2026-11-25T06:00:00Z`) pin the run and valid time.

use std::time::Instant;

use omo_engine::{
    config::BootstrapConfig,
    map::fetch::{LoadFieldParams, load_field},
};
use omo_protocol::{
    map::{MapBbox, SourceLayout},
    series::CatalogEntry,
};

fn live_config(dir: &std::path::Path) -> Option<BootstrapConfig> {
    let path = std::env::var("OMO_LIVE_BOOTSTRAP").ok()?;
    let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    value["data_dir"] = serde_json::json!(dir);
    serde_json::from_value(value).ok()
}

fn entry() -> CatalogEntry {
    let catalog: serde_json::Value = serde_json::from_slice(
        &std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../catalog/products.json"
        ))
        .unwrap(),
    )
    .unwrap();
    serde_json::from_value(catalog["entries"][0].clone()).unwrap()
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

#[test]
#[ignore = "requires live Open-Meteo network and the duckomo artifact"]
fn spatial_and_data_run_agree_bit_for_bit_on_a_small_patch() {
    let dir = tempfile::tempdir().unwrap();
    let Some(config) = live_config(dir.path()) else {
        panic!("set OMO_LIVE_BOOTSTRAP to a BootstrapConfig JSON");
    };
    let entry = entry();
    let run = env_or("OMO_LIVE_RUN", "2026-11-25T00:00:00Z");
    let valid = env_or("OMO_LIVE_VALID", "2026-11-25T06:00:00Z");
    let bbox = MapBbox {
        lat_min: 39.0,
        lat_max: 41.0,
        lon_spans: vec![[115.0, 117.0]],
    };
    let mut stages = Vec::new();
    let mut progress = |stage: &'static str, detail: Option<&str>| {
        stages.push((stage, detail.map(str::to_owned)));
    };
    let started = Instant::now();
    let spatial = load_field(
        &LoadFieldParams {
            config: &config,
            entry: &entry,
            field_id: "live-spatial",
            reference_time_utc: &run,
            valid_time_utc: &valid,
            bbox: &bbox,
            force_data_run: false,
            readers: 4,
            cancel: None,
        },
        &mut progress,
    )
    .unwrap();
    println!(
        "data_spatial {:?} metrics {:?}",
        started.elapsed(),
        spatial.metrics
    );
    assert_eq!(spatial.field.source_layout, SourceLayout::DataSpatial);
    assert!(spatial.fallback_notice.is_none());
    assert!(
        spatial.field.stats.valid_count > 100,
        "{:?}",
        spatial.field.stats
    );

    let started = Instant::now();
    let archive = load_field(
        &LoadFieldParams {
            config: &config,
            entry: &entry,
            field_id: "live-run",
            reference_time_utc: &run,
            valid_time_utc: &valid,
            bbox: &bbox,
            force_data_run: true,
            readers: 4,
            cancel: None,
        },
        &mut progress,
    )
    .unwrap();
    println!(
        "data_run {:?} metrics {:?}",
        started.elapsed(),
        archive.metrics
    );
    assert_eq!(archive.field.source_layout, SourceLayout::DataRun);

    let (a, b) = (
        spatial.field.values.as_slice(),
        archive.field.values.as_slice(),
    );
    let mut max_delta = 0.0f32;
    let mut mismatches = 0usize;
    for (x, y) in a.iter().zip(b) {
        if x.to_bits() != y.to_bits() && !(x.is_nan() && y.is_nan()) {
            mismatches += 1;
            max_delta = max_delta.max((x - y).abs());
        }
    }
    println!(
        "valid {} mismatches {mismatches} max|d| {max_delta}",
        spatial.field.stats.valid_count
    );
    assert_eq!(mismatches, 0);
    assert_eq!(spatial.field.stats, archive.field.stats);
}

#[test]
#[ignore = "requires live Open-Meteo network and the duckomo artifact"]
fn expired_spatial_object_falls_back_once_to_data_run_or_reports_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let Some(config) = live_config(dir.path()) else {
        panic!("set OMO_LIVE_BOOTSTRAP to a BootstrapConfig JSON");
    };
    let entry = entry();
    let bbox = MapBbox {
        lat_min: 39.0,
        lat_max: 41.0,
        lon_spans: vec![[115.0, 117.0]],
    };
    let mut details = Vec::new();
    let mut progress = |_: &'static str, detail: Option<&str>| {
        if let Some(detail) = detail {
            details.push(detail.to_owned());
        }
    };
    // Months-old run: data_spatial is gone and data_run is gone too, so both 404.
    let result = load_field(
        &LoadFieldParams {
            config: &config,
            entry: &entry,
            field_id: "live-old",
            reference_time_utc: "2026-10-01T00:00:00Z",
            valid_time_utc: "2026-10-01T00:00:00Z",
            bbox: &bbox,
            force_data_run: false,
            readers: 2,
            cancel: None,
        },
        &mut progress,
    );
    println!(
        "expired result: {:?}",
        result.as_ref().map(|l| &l.fallback_notice)
    );
    assert_eq!(
        details,
        vec!["data_spatial expired; falling back to data_run archive"],
        "{result:?}"
    );
    match result {
        Err(omo_engine::map::fetch::FetchError::ValidTimeUnavailable(_)) => {}
        Ok(loaded) => {
            assert_eq!(loaded.field.source_layout, SourceLayout::DataRun);
            assert!(loaded.fallback_notice.is_some());
        }
        Err(other) => panic!("unexpected {other:?}"),
    }
}

#[test]
#[ignore = "requires live Open-Meteo network and the duckomo artifact"]
fn remote_tuning_is_visible_on_a_bootstrapped_connection() {
    let dir = tempfile::tempdir().unwrap();
    let config = live_config(dir.path()).expect("set OMO_LIVE_BOOTSTRAP");
    let conn = omo_engine::open_bootstrapped(config).unwrap();
    // Bootstrap re-reads both settings after `SET lock_configuration = true` and fails on a
    // mismatch, so a successful open with a report proves the tuning survived the lock.
    let tuning = conn.report().remote_tuning.expect("remote session");
    assert_eq!(tuning.max_request_bytes, 4_194_304);
    assert_eq!(tuning.merge_gap_bytes, 65_536);
}
