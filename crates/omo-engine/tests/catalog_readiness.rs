//! Catalog readiness checks (T037).

use std::path::PathBuf;

use omo_engine::{
    catalog::{
        Catalog, GridInfoSource, GridProbe, check_readiness_with, parse_grid_point_count,
        run::{MetaSource, RunError, RunMeta, synthetic_valid_times},
    },
    grid::reduced_gaussian::O1280,
};
use omo_protocol::series::{NotReadyReason, Readiness};

struct FakeMeta {
    latest: RunMeta,
}

impl MetaSource for FakeMeta {
    fn latest(&self) -> Result<RunMeta, RunError> {
        Ok(self.latest.clone())
    }
    fn meta(&self, _reference_time_utc: &str) -> Result<RunMeta, RunError> {
        Err(RunError::NoCompleteRun)
    }
}

struct FakeGrid {
    probe: Result<GridProbe, String>,
}

impl GridInfoSource for FakeGrid {
    fn grid_info(&self, _object_url: &str, _domain: &str) -> Result<GridProbe, String> {
        self.probe.clone()
    }
}

fn default_entry() -> omo_protocol::series::CatalogEntry {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog/products.json");
    Catalog::load(&path).unwrap().default_entry().clone()
}

fn complete_00z() -> RunMeta {
    RunMeta {
        reference_time_utc: "2026-11-23T00:00:00Z".into(),
        valid_times: synthetic_valid_times("2026-11-23T00:00:00Z", 360).unwrap(),
        variables: vec!["temperature_2m".into()],
        meta_etag: Some("\"etag\"".into()),
    }
}

fn valid_grid() -> FakeGrid {
    let (lat, lon) = O1280::point(0);
    FakeGrid {
        probe: Ok(GridProbe {
            point_count: 6_599_680,
            samples: vec![(0, lat, lon)],
            unit: Some("°C".into()),
        }),
    }
}

#[test]
fn readiness_covers_ready_remote_unavailable_variable_missing_no_run_and_grid_mismatch() {
    let entry = default_entry();
    let ok_meta = FakeMeta {
        latest: complete_00z(),
    };

    assert!(matches!(
        check_readiness_with(&entry, true, &ok_meta, &valid_grid()),
        Readiness::Ready { .. }
    ));

    assert!(matches!(
        check_readiness_with(&entry, false, &ok_meta, &valid_grid()),
        Readiness::NotReady {
            reason: NotReadyReason::RemoteUnavailable,
            ..
        }
    ));

    let mut missing_var = complete_00z();
    missing_var.variables = vec!["wind_speed_10m".into()];
    let var_meta = FakeMeta {
        latest: missing_var,
    };
    assert!(matches!(
        check_readiness_with(&entry, true, &var_meta, &valid_grid()),
        Readiness::NotReady {
            reason: NotReadyReason::VariableMissing,
            ..
        }
    ));

    let mut partial = complete_00z();
    partial.valid_times = synthetic_valid_times("2026-11-23T00:00:00Z", 72).unwrap();
    let no_run_meta = FakeMeta { latest: partial };
    assert!(matches!(
        check_readiness_with(&entry, true, &no_run_meta, &valid_grid()),
        Readiness::NotReady {
            reason: NotReadyReason::NoCompleteRun,
            ..
        }
    ));

    let bad_count = FakeGrid {
        probe: Ok(GridProbe {
            point_count: 100,
            samples: vec![],
            unit: Some("°C".into()),
        }),
    };
    assert!(matches!(
        check_readiness_with(&entry, true, &ok_meta, &bad_count),
        Readiness::NotReady {
            reason: NotReadyReason::GridMismatch,
            ..
        }
    ));

    let empty_samples = FakeGrid {
        probe: Ok(GridProbe {
            point_count: 6_599_680,
            samples: vec![],
            unit: Some("°C".into()),
        }),
    };
    assert!(matches!(
        check_readiness_with(&entry, true, &ok_meta, &empty_samples),
        Readiness::NotReady {
            reason: NotReadyReason::GridMismatch,
            ..
        }
    ));

    let (lat, lon) = O1280::point(0);
    let bad_coord = FakeGrid {
        probe: Ok(GridProbe {
            point_count: 6_599_680,
            samples: vec![(0, lat + 0.5, lon)],
            unit: Some("°C".into()),
        }),
    };
    assert!(matches!(
        check_readiness_with(&entry, true, &ok_meta, &bad_coord),
        Readiness::NotReady {
            reason: NotReadyReason::GridMismatch,
            ..
        }
    ));

    let bad_unit = FakeGrid {
        probe: Ok(GridProbe {
            point_count: 6_599_680,
            samples: vec![(0, lat, lon)],
            unit: Some("K".into()),
        }),
    };
    assert!(matches!(
        check_readiness_with(&entry, true, &ok_meta, &bad_unit),
        Readiness::NotReady {
            reason: NotReadyReason::VariableMissing,
            ..
        }
    ));

    assert!(parse_grid_point_count(&serde_json::json!({}), &serde_json::json!({})).is_err());
    assert_eq!(
        parse_grid_point_count(
            &serde_json::json!({}),
            &serde_json::json!({"axes": ["location", "time"], "shape": [6_599_680, 145]})
        ),
        Ok(6_599_680)
    );
}
