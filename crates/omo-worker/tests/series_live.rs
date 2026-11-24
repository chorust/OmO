//! Network-gated live series integration test (T028).
//! Run with: `cargo +1.100.0 test -p omo-worker --test series_live -- --ignored`

use std::io::Write;

use omo_protocol::{
    frame::{decode_frame, encode_frame},
    messages::Envelope,
    series::{CatalogEntry, LeadWindow, RunSelector, SeriesRequest, SeriesResultBody},
};
use serde_json::{Value, json};
use tempfile::tempdir;

#[test]
#[ignore = "requires live Open-Meteo network and duckomo artifact"]
fn live_worker_series_3_stations_returns_valid_cycle_and_distances() {
    let dir = tempdir().unwrap();
    let stations_csv = dir.path().join("stations.csv");
    std::fs::write(
        &stations_csv,
        "station_id,latitude,longitude,name,elevation_m\n1,39.9042,116.4074,Beijing,\n2,51.5074,-0.1278,London,\n3,-33.8688,151.2093,Sydney,\n",
    )
    .unwrap();
    let artifact = std::env::var("OMOBENCH_DUCKOMO_ARTIFACT")
        .unwrap_or_else(|_| "/tmp/omobench-duckomo-artifacts/duckomo-v0.4.0-duckdb-v2.0.0-linux_arm64.duckdb_extension.gz".into());
    let catalog: Value =
        serde_json::from_str(&std::fs::read_to_string("../../catalog/products.json").unwrap())
            .unwrap();
    let entry: CatalogEntry = serde_json::from_value(catalog["entries"][0].clone()).unwrap();

    // Test via the real EngineBackend binary by spawning `omo-worker`.
    let exe = env!("CARGO_BIN_EXE_omo-worker");
    let mut child = std::process::Command::new(exe)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let send = |stdin: &mut std::process::ChildStdin, env: Envelope<Value>| {
        let frame = encode_frame(&serde_json::to_vec(&env).unwrap()).unwrap();
        stdin.write_all(&frame).unwrap();
        stdin.flush().unwrap();
    };

    send(
        &mut stdin,
        Envelope::new(
            1,
            0,
            1,
            "hello",
            json!({
                "bootstrap_config": {
                    "data_dir": dir.path(),
                    "channel": "github-release-unsigned",
                    "artifact_path": artifact,
                    "allow_remote": true
                }
            }),
        ),
    );
    let ready: Envelope<Value> =
        serde_json::from_slice(&decode_frame(&mut stdout).unwrap()).unwrap();
    assert_eq!(ready.kind, "ready");

    let req = SeriesRequest {
        stations_ref: stations_csv.to_string_lossy().into_owned(),
        stations_sha256: "test".into(),
        catalog_entry: entry,
        catalog_version: "2026-11-23.1".into(),
        run: RunSelector::latest_complete(),
        window: LeadWindow::new(0, 168),
        max_distance_m: 7200,
        result_id: "live-3".into(),
        limits: omo_protocol::result_ref::ResultLimits {
            rows: 100_000,
            bytes: 64 * 1024 * 1024,
        },
    };
    send(
        &mut stdin,
        Envelope::new(1, 1, 2, "series", serde_json::to_value(req).unwrap()),
    );
    let result_env = loop {
        let env: Envelope<Value> =
            serde_json::from_slice(&decode_frame(&mut stdout).unwrap()).unwrap();
        if env.request_id == 2 && matches!(env.kind.as_str(), "result" | "failed") {
            break env;
        }
    };
    assert_eq!(result_env.kind, "result", "failed: {:?}", result_env.body);
    let body: SeriesResultBody = serde_json::from_value(result_env.body).unwrap();
    assert!(body.run.object_url.starts_with("s3://"));
    let expected_truncated = matches!(body.run.cycle.as_str(), "06" | "18");
    assert_eq!(body.effective_window.truncated, expected_truncated);
    assert_eq!(
        body.result_ref["rows"].as_u64().unwrap(),
        if expected_truncated { 3 * 109 } else { 3 * 113 }
    );

    send(&mut stdin, Envelope::new(1, 0, 3, "shutdown", json!({})));
    let _ = child.wait();
}
