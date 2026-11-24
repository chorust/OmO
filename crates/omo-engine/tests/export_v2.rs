use std::{
    fs,
    io::{self, Read},
    path::Path,
};

use omo_engine::{export::export_sql_with_rows, test_support::new_unbootstrapped_conn};
use omo_protocol::export::sidecar_path;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

#[test]
fn sidecar_v2_exports_series_and_sql_metadata() {
    let dir = tempdir().unwrap();
    let conn = new_unbootstrapped_conn().unwrap();

    let target_series = dir.path().join("series.csv");
    let snapshot_series = json!({
        "kind": "series",
        "m1_sidecar": {
            "stations": {"sha256": "deadbeef", "count": 1},
            "catalog": {"entry_id": "ecmwf_ifs.temperature_2m", "version": "2026-11-23.1"},
            "run": {
                "reference_time_utc": "2026-11-23T00:00:00Z",
                "cycle": "00",
                "object_url": "s3://openmeteo/data_run/ecmwf_ifs/2026/11/23/0000Z/temperature_2m.om",
                "object_version": "\"etag1\""
            },
            "window": {"requested": [0, 168], "effective": [0, 168], "truncated": false},
            "plan": "G",
            "query": null,
            "attribution": ["Weather data by Open-Meteo.com (CC BY 4.0)", "Copyright © 2026 ECMWF"]
        }
    });
    let hash = export_sql_with_rows(
        &conn,
        "SELECT 'A1' AS station_id, 12.5::DOUBLE AS value",
        &target_series,
        "csv",
        snapshot_series,
        false,
        Some(1),
    )
    .unwrap();
    let sidecar: Value =
        serde_json::from_slice(&fs::read(sidecar_path(&target_series)).unwrap()).unwrap();
    assert_eq!(sidecar["schema"], "omo-export-sidecar/2");
    assert_eq!(sidecar["stations"]["count"], 1);
    assert_eq!(sidecar["catalog"]["entry_id"], "ecmwf_ifs.temperature_2m");
    assert_eq!(sidecar["run"]["cycle"], "00");
    assert_eq!(sidecar["plan"], "G");
    assert!(sidecar["query"].is_null());
    assert_eq!(sidecar["attribution"].as_array().unwrap().len(), 2);
    assert_eq!(sha256_file(&target_series).unwrap(), hash);

    // SQL export without an attached series view has `query` set and `run`/`window` null.
    let target_sql = dir.path().join("sql.parquet");
    let snapshot_sql = json!({
        "kind": "sql",
        "m1_sidecar": {
            "stations": {"sha256": "deadbeef", "count": 1},
            "catalog": {"entry_id": "ecmwf_ifs.temperature_2m", "version": "2026-11-23.1"},
            "run": null,
            "window": null,
            "plan": "G",
            "query": "SELECT * FROM stations",
            "attribution": ["Weather data by Open-Meteo.com (CC BY 4.0)"]
        }
    });
    export_sql_with_rows(
        &conn,
        "SELECT 'A1' AS station_id",
        &target_sql,
        "parquet",
        snapshot_sql,
        false,
        Some(1),
    )
    .unwrap();
    let sql_sidecar: Value =
        serde_json::from_slice(&fs::read(sidecar_path(&target_sql)).unwrap()).unwrap();
    assert_eq!(sql_sidecar["schema"], "omo-export-sidecar/2");
    assert_eq!(sql_sidecar["query"], "SELECT * FROM stations");
    assert!(sql_sidecar["run"].is_null());
    assert!(sql_sidecar["window"].is_null());
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
