//! M1 protocol bodies stay additive. M0 envelopes are unchanged.

use omo_protocol::{
    messages::{Envelope, MessageBody, failed_body, parse_fail_kind},
    result_ref::ResultLimits,
    series::{
        CatalogCheckRequest, CatalogCheckResult, CatalogEntry, ForecastRun, LeadWindow, Readiness,
        ResolveRunRequest, ResolveRunResult, RunSelector, SeriesRequest, SeriesResultBody,
        SqlSessionRequest, SqlSessionResult,
    },
    state::FailKind,
};
use serde_json::json;

#[test]
fn m1_bodies_round_trip_and_new_fail_kinds_are_stable() {
    let entry: CatalogEntry = serde_json::from_value(sample_entry()).unwrap();
    let request = SeriesRequest {
        stations_ref: "stations.csv".into(),
        stations_sha256: "ab".repeat(32),
        catalog_entry: entry.clone(),
        catalog_version: "2026-11-23.1".into(),
        run: RunSelector::latest_complete(),
        window: LeadWindow::new(0, 168),
        max_distance_m: 7200,
        result_id: "series-1".into(),
        limits: ResultLimits {
            rows: 10,
            bytes: 20,
        },
    };
    assert_eq!(round_trip(&request), request);
    let explicit = RunSelector::explicit("2026-11-23T00:00:00Z");
    assert_eq!(round_trip(&explicit), explicit);

    let run = ForecastRun {
        reference_time_utc: "2026-11-22T18:00:00Z".into(),
        cycle: "18".into(),
        horizon_h: 144,
        valid_times: vec!["2026-11-22T18:00:00Z".into()],
        object_url: "s3://openmeteo/data_run/ecmwf_ifs/2026/11/22/1800Z/temperature_2m.om".into(),
        meta_etag: None,
    };
    let result = SeriesResultBody {
        result_ref: json!({"id": "series-1"}),
        parquet_path: "series-1.parquet".into(),
        matches_ref: json!({"id": "series-1-matches"}),
        plot_ref: None,
        run: run.clone(),
        effective_window: omo_protocol::series::EffectiveWindow {
            requested: [0, 168],
            effective: [0, 144],
            truncated: true,
        },
        plan: "G".into(),
        scan_metrics: json!({}),
    };
    assert_eq!(round_trip(&result), result);

    let check = CatalogCheckRequest {
        entry: entry.clone(),
    };
    assert_eq!(round_trip(&check), check);
    let ready = CatalogCheckResult {
        entry_id: entry.id.clone(),
        readiness: Readiness::Ready { run: run.clone() },
    };
    assert_eq!(round_trip(&ready), ready);
    let resolve = ResolveRunRequest {
        entry,
        run: RunSelector::latest_complete(),
    };
    assert_eq!(round_trip(&resolve), resolve);
    let resolved = ResolveRunResult { run };
    assert_eq!(round_trip(&resolved), resolved);
    let sql = SqlSessionRequest {
        stations_ref: "stations.csv".into(),
        series_parquet: Some("series.parquet".into()),
    };
    assert_eq!(round_trip(&sql), sql);
    let views = SqlSessionResult {
        views: vec!["stations".into(), "station_series".into()],
    };
    assert_eq!(round_trip(&views), views);

    for kind in [
        FailKind::NoCompleteRun,
        FailKind::RunIncomplete,
        FailKind::GridMismatch,
        FailKind::VariableMissing,
        FailKind::SourceChanged,
    ] {
        let body = failed_body(1, &kind, "m1");
        assert_eq!(parse_fail_kind(&body), Some(kind.clone()));
        let again = serde_json::to_string(&kind).unwrap();
        assert_eq!(serde_json::from_str::<FailKind>(&again).unwrap(), kind);
    }
    assert_eq!(
        serde_json::to_value(FailKind::NoCompleteRun).unwrap(),
        json!({"kind": "no_complete_run"})
    );
    assert_eq!(
        serde_json::to_value(FailKind::GridMismatch).unwrap(),
        json!({"kind": "grid_mismatch"})
    );

    let m0 = Envelope::new(1, 2, 3, "query", json!({"sql": "select 1"}));
    let encoded = serde_json::to_vec(&m0).unwrap();
    let decoded: Envelope<MessageBody> = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, m0);
}

fn round_trip<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq>(value: &T) -> T {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

fn sample_entry() -> serde_json::Value {
    json!({
        "id": "ecmwf_ifs.temperature_2m",
        "is_default": true,
        "product": {"provider": "ECMWF", "model": "IFS HRES", "open_meteo_domain": "ecmwf_ifs"},
        "variable": {"name": "temperature_2m", "unit": "°C", "label": "2 m air temperature"},
        "layout": {
            "kind": "data_run",
            "base": "s3://openmeteo/data_run/ecmwf_ifs/",
            "latest": "latest.json",
            "object": "{YYYY}/{MM}/{DD}/{HH}00Z/{variable}.om",
            "meta": "{YYYY}/{MM}/{DD}/{HH}00Z/meta.json"
        },
        "grid": {"kind": "reduced_gaussian_o1280", "duckomo_domain": "ecmwf_ifs", "points": 6599680},
        "cycles": {"00": 360, "06": 144, "12": 360, "18": 144},
        "default_window_h": [0, 168],
        "max_distance_m": 7200,
        "attribution": ["attr"]
    })
}
