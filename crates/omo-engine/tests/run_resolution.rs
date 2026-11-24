//! Run completeness and the effective lead window (research R-02).

use std::collections::HashMap;

use omo_engine::catalog::run::{
    MetaSource, RunError, RunMeta, RunRef, effective_window, resolve, synthetic_valid_times,
    time_index_table,
};
use omo_protocol::series::{CatalogEntry, LeadWindow};
use serde_json::json;

struct MapSource {
    latest: RunMeta,
    metas: HashMap<String, RunMeta>,
}

impl MetaSource for MapSource {
    fn latest(&self) -> Result<RunMeta, RunError> {
        Ok(self.latest.clone())
    }

    fn meta(&self, reference_time_utc: &str) -> Result<RunMeta, RunError> {
        self.metas
            .get(reference_time_utc)
            .cloned()
            .ok_or_else(|| RunError::Io(format!("no meta for {reference_time_utc}")))
    }
}

#[test]
fn resolver_skips_a_partial_latest_and_keeps_the_previous_complete_cycle() {
    let entry = entry();
    let partial = meta("2026-11-23T00:00:00Z", 72, true);
    assert_eq!(partial.valid_times.len(), 73);
    let complete_18 = meta("2026-11-22T18:00:00Z", 144, true);
    assert_eq!(complete_18.valid_times.len(), 109);
    let complete_00 = meta("2026-11-22T00:00:00Z", 360, true);
    assert_eq!(complete_00.valid_times.len(), 145);
    let source = MapSource {
        latest: partial.clone(),
        metas: HashMap::from([
            (partial.reference_time_utc.clone(), partial),
            (complete_18.reference_time_utc.clone(), complete_18.clone()),
            (complete_00.reference_time_utc.clone(), complete_00.clone()),
        ]),
    };
    let resolved = resolve(&entry, &source, RunRef::LatestComplete).unwrap();
    assert_eq!(resolved.reference_time_utc, "2026-11-22T18:00:00Z");
    assert_eq!(resolved.cycle, "18");
    assert_eq!(resolved.horizon_h, 144);

    let incomplete = resolve(
        &entry,
        &source,
        RunRef::Explicit("2026-11-23T00:00:00Z".into()),
    )
    .unwrap_err();
    assert!(matches!(incomplete, RunError::RunIncomplete { .. }));

    let window = effective_window(LeadWindow::new(0, 168), &resolved);
    assert_eq!(window.effective, [0, 144]);
    assert!(window.truncated);
    assert_eq!(time_index_table(&resolved, &window).unwrap().len(), 109);

    let full = resolve(
        &entry,
        &source,
        RunRef::Explicit("2026-11-22T00:00:00Z".into()),
    )
    .unwrap();
    let full_window = effective_window(LeadWindow::new(0, 168), &full);
    assert!(!full_window.truncated);
    let table = time_index_table(&full, &full_window).unwrap();
    assert_eq!(table.len(), 113);
    assert_eq!(table[0].1, "2026-11-22T00:00:00Z");
    assert_eq!(table[90].1, "2026-11-25T18:00:00Z");
    assert_eq!(table[91].1, "2026-11-25T21:00:00Z");
    assert_eq!(table.last().unwrap().1, "2026-11-29T00:00:00Z");
}

#[test]
fn eight_incomplete_candidates_and_a_missing_variable_are_distinct_failures() {
    let entry = entry();
    let mut metas = HashMap::new();
    let mut cursor = "2026-11-23T00:00:00Z".to_owned();
    for _ in 0..8 {
        let partial = meta(&cursor, 72, true);
        let next = step_back(&partial.reference_time_utc);
        metas.insert(partial.reference_time_utc.clone(), partial);
        cursor = next;
    }
    let latest = metas.get("2026-11-23T00:00:00Z").unwrap().clone();
    let source = MapSource { latest, metas };
    assert_eq!(
        resolve(&entry, &source, RunRef::LatestComplete).unwrap_err(),
        RunError::NoCompleteRun
    );

    let mut missing = meta("2026-11-22T00:00:00Z", 360, false);
    missing.variables.clear();
    let source = MapSource {
        latest: missing.clone(),
        metas: HashMap::from([(missing.reference_time_utc.clone(), missing)]),
    };
    let error = resolve(
        &entry,
        &source,
        RunRef::Explicit("2026-11-22T00:00:00Z".into()),
    )
    .unwrap_err();
    assert!(
        matches!(error, RunError::VariableMissing { .. }),
        "{error:?}"
    );
}

fn entry() -> CatalogEntry {
    serde_json::from_value(json!({
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
        "attribution": []
    }))
    .unwrap()
}

fn meta(reference: &str, until_h: u32, with_variable: bool) -> RunMeta {
    RunMeta {
        reference_time_utc: reference.to_owned(),
        valid_times: synthetic_valid_times(reference, until_h).unwrap(),
        variables: if with_variable {
            vec!["temperature_2m".to_owned()]
        } else {
            Vec::new()
        },
        meta_etag: None,
    }
}

fn step_back(reference: &str) -> String {
    let unix = omo_engine::catalog::run::parse_utc(reference).unwrap() - 6 * 3600;
    omo_engine::catalog::run::format_utc(unix)
}
