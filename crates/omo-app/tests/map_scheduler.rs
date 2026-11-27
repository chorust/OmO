//! `MapRequestScheduler` tests (`T024`): the worker allows one active request, so map work is
//! serialized (`map_load_field` > `map_layer_probe` > `map_render_tiles` > `map_inspect`), pan/zoom batches and hover
//! points coalesce to the latest, a newer field revision interrupts the running load, and a
//! failed load keeps the last good field visible as `Stale`.

use omo_app::host::map_host::{
    ExportIntent, FieldIntent, FieldState, InspectIntent, MapAction, MapCompletion, MapOutcome,
    MapRequestKind, MapRequestScheduler, ProbeIntent, TilesIntent,
};
use serde_json::{Value, json};

fn field(revision: u64) -> FieldIntent {
    FieldIntent {
        revision,
        body: json!({"field_id": format!("f{revision}")}),
    }
}

fn tiles(field_revision: u64, tag: &str) -> TilesIntent {
    TilesIntent {
        field_revision,
        body: json!({"result_id": tag}),
    }
}

fn inspect(field_revision: u64, tag: &str) -> InspectIntent {
    InspectIntent {
        field_revision,
        body: json!({"tag": tag}),
    }
}

fn sent(actions: &[MapAction]) -> Vec<(u64, MapRequestKind)> {
    actions
        .iter()
        .filter_map(|action| match action {
            MapAction::Send {
                request_id, kind, ..
            } => Some((*request_id, *kind)),
            _ => None,
        })
        .collect()
}

fn ok(value: Value) -> MapOutcome {
    MapOutcome::Success(value)
}

fn load_result(id: &str) -> Value {
    json!({"field_id": id, "stats": {"valid_count": 5, "min": 0.0, "max": 1.0, "unit": "°C"}})
}

#[test]
fn only_one_request_is_ever_in_flight() {
    let mut s = MapRequestScheduler::new(100);
    let a = s.submit_field(field(1));
    assert_eq!(sent(&a), vec![(100, MapRequestKind::LoadField)]);
    // Tiles for the loading revision wait; nothing else is sent.
    assert!(s.submit_tiles(tiles(1, "t1")).is_empty());
    assert!(s.submit_inspect(inspect(1, "i1")).is_empty());
    assert!(s.has_running());

    let (completion, next) = s.complete(100, ok(load_result("f1")));
    assert!(matches!(
        completion,
        MapCompletion::FieldLoaded { revision: 1, .. }
    ));
    // Tiles outrank inspect once the field is ready.
    assert_eq!(sent(&next), vec![(101, MapRequestKind::RenderTiles)]);
    let (_, next) = s.complete(101, ok(json!({"tiles": []})));
    assert_eq!(sent(&next), vec![(102, MapRequestKind::Inspect)]);
    let (_, next) = s.complete(102, ok(json!({})));
    assert!(next.is_empty());
    assert!(!s.has_running());
}

#[test]
fn rapid_pan_and_zoom_coalesce_to_the_latest_batch() {
    let mut s = MapRequestScheduler::new(1);
    s.submit_field(field(1));
    s.complete(1, ok(load_result("f1")));
    // Field is ready and idle: first batch goes out immediately.
    let first = s.submit_tiles(tiles(1, "t1"));
    assert_eq!(sent(&first), vec![(2, MapRequestKind::RenderTiles)]);
    // Five more viewports arrive while it renders; they never interrupt it.
    for n in 2..=6 {
        assert!(s.submit_tiles(tiles(1, &format!("t{n}"))).is_empty());
    }
    let (_, next) = s.complete(2, ok(json!({})));
    let sends: Vec<_> = next
        .iter()
        .filter_map(|a| match a {
            MapAction::Send {
                body,
                kind: MapRequestKind::RenderTiles,
                ..
            } => Some(body["result_id"].clone()),
            _ => None,
        })
        .collect();
    assert_eq!(sends, vec![json!("t6")], "only the latest batch survives");
}

#[test]
fn hover_points_coalesce_and_never_preempt_rendering() {
    let mut s = MapRequestScheduler::new(1);
    s.submit_field(field(1));
    s.complete(1, ok(load_result("f1")));
    s.submit_tiles(tiles(1, "t"));
    for n in 0..20 {
        assert!(s.submit_inspect(inspect(1, &format!("i{n}"))).is_empty());
    }
    let (_, next) = s.complete(2, ok(json!({})));
    match &next[..] {
        [
            MapAction::Send {
                kind: MapRequestKind::Inspect,
                body,
                ..
            },
        ] => assert_eq!(body["tag"], "i19"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_newer_field_revision_interrupts_the_running_load_once_and_runs_after_it_ends() {
    let mut s = MapRequestScheduler::new(10);
    s.submit_field(field(1));
    let interrupt = s.submit_field(field(2));
    assert_eq!(interrupt, vec![MapAction::Interrupt { request_id: 10 }]);
    // Further superseding intents queue silently; the interrupt is not repeated.
    assert!(s.submit_field(field(3)).is_empty());
    // Queued tiles/inspect for superseded revisions are dropped when the load ends.
    s.submit_tiles(tiles(1, "old"));

    let (completion, next) = s.complete(
        10,
        MapOutcome::Failure {
            message: "cancelled".into(),
            cancelled: true,
        },
    );
    assert_eq!(completion, MapCompletion::DiscardedSuperseded);
    assert_eq!(sent(&next), vec![(11, MapRequestKind::LoadField)]);
    match &next[0] {
        MapAction::Send { body, .. } => {
            assert_eq!(body["field_id"], "f3", "only the latest revision runs")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_finished_superseded_load_is_discarded_not_published() {
    let mut s = MapRequestScheduler::new(10);
    s.submit_field(field(1));
    s.submit_field(field(2));
    let (completion, next) = s.complete(10, ok(load_result("f1")));
    assert_eq!(completion, MapCompletion::DiscardedSuperseded);
    assert!(matches!(
        s.field_state(),
        FieldState::Loading { revision: 2 }
    ));
    assert_eq!(sent(&next), vec![(11, MapRequestKind::LoadField)]);
}

#[test]
fn tiles_for_an_old_revision_are_dropped() {
    let mut s = MapRequestScheduler::new(1);
    s.submit_field(field(1));
    s.complete(1, ok(load_result("f1")));
    s.submit_tiles(tiles(1, "t"));
    s.submit_field(field(2));
    let (completion, next) = s.complete(2, ok(json!({})));
    assert!(matches!(
        completion,
        MapCompletion::Completed {
            kind: MapRequestKind::RenderTiles,
            ..
        }
    ));
    // The render finished for revision 1, but the new load goes first and old tiles are gone.
    assert_eq!(sent(&next), vec![(3, MapRequestKind::LoadField)]);
    s.complete(3, ok(load_result("f2")));
    assert!(!s.has_running());
}

#[test]
fn a_failed_reload_keeps_the_last_good_field_as_stale() {
    let mut s = MapRequestScheduler::new(1);
    s.submit_field(field(1));
    s.complete(1, ok(load_result("f1")));
    assert!(matches!(
        s.field_state(),
        FieldState::Ready { revision: 1, .. }
    ));

    s.submit_field(field(2));
    let (completion, _) = s.complete(
        2,
        MapOutcome::Failure {
            message: "network".into(),
            cancelled: false,
        },
    );
    assert_eq!(
        completion,
        MapCompletion::FieldFailedStale("network".into())
    );
    match s.field_state() {
        FieldState::Stale { loaded, error, .. } => {
            assert_eq!(loaded["field_id"], "f1");
            assert_eq!(error, "network");
        }
        other => panic!("{other:?}"),
    }
    // Rendering and inspecting still target the retained field.
    assert_eq!(s.active_field_revision(), Some(1));
}

#[test]
fn a_first_load_failure_without_a_previous_field_is_failed() {
    let mut s = MapRequestScheduler::new(1);
    s.submit_field(field(1));
    let (completion, next) = s.complete(
        1,
        MapOutcome::Failure {
            message: "no run".into(),
            cancelled: false,
        },
    );
    assert_eq!(completion, MapCompletion::FieldFailed("no run".into()));
    assert!(matches!(s.field_state(), FieldState::Failed(message) if message == "no run"));
    assert!(next.is_empty());
    assert_eq!(s.active_field_revision(), None);
    // Tiles without a field still render the non-field layers (field_id absent).
    let tile_actions = s.submit_tiles(tiles(0, "base"));
    assert_eq!(sent(&tile_actions), vec![(2, MapRequestKind::RenderTiles)]);
}

#[test]
fn unknown_completions_are_ignored() {
    let mut s = MapRequestScheduler::new(1);
    let (completion, next) = s.complete(999, ok(json!({})));
    assert_eq!(completion, MapCompletion::DiscardedUnknown);
    assert!(next.is_empty());
}

#[test]
fn a_worker_crash_resets_in_flight_state_and_retains_the_last_good_field() {
    let mut s = MapRequestScheduler::new(1);
    s.submit_field(field(1));
    s.complete(1, ok(load_result("f1")));
    s.submit_tiles(tiles(1, "t"));
    s.submit_inspect(inspect(1, "i"));
    s.worker_restarted();
    assert!(!s.has_running());
    // The worker lost its cache, so the retained field is only a stale picture.
    assert!(matches!(s.field_state(), FieldState::Stale { .. }));
    // Nothing queued survives the restart.
    let (_, next) = s.complete(2, ok(json!({})));
    assert!(next.is_empty());
}

fn probe(layer: &str) -> ProbeIntent {
    ProbeIntent {
        layer_id: layer.into(),
        body: json!({"layer_id": layer}),
    }
}

#[test]
fn layer_probes_queue_in_order_and_run_before_tiles() {
    let mut s = MapRequestScheduler::new(1);
    // No field yet: tiles with revision 0 render the archive layers only.
    let first = s.submit_tiles(tiles(0, "t0"));
    assert_eq!(sent(&first), vec![(1, MapRequestKind::RenderTiles)]);
    assert!(s.submit_probe(probe("a")).is_empty());
    assert!(s.submit_probe(probe("b")).is_empty());
    assert!(s.submit_tiles(tiles(0, "t1")).is_empty());
    let (_, next) = s.complete(1, ok(json!({})));
    assert_eq!(sent(&next), vec![(2, MapRequestKind::LayerProbe)]);
    let (_, next) = s.complete(2, ok(json!({"layer_id": "a"})));
    assert_eq!(sent(&next), vec![(3, MapRequestKind::LayerProbe)]);
    let (completion, next) = s.complete(3, ok(json!({"layer_id": "b"})));
    assert!(matches!(
        completion,
        MapCompletion::Completed {
            kind: MapRequestKind::LayerProbe,
            ..
        }
    ));
    assert_eq!(sent(&next), vec![(4, MapRequestKind::RenderTiles)]);
}

#[test]
fn a_queued_probe_of_the_same_layer_is_replaced_and_a_field_load_goes_first() {
    let mut s = MapRequestScheduler::new(1);
    s.submit_tiles(tiles(0, "t0"));
    s.submit_probe(probe("a"));
    s.submit_probe(probe("a"));
    s.submit_field(field(1));
    let (_, next) = s.complete(1, ok(json!({})));
    assert_eq!(sent(&next), vec![(2, MapRequestKind::LoadField)]);
    let (_, next) = s.complete(2, ok(load_result("f1")));
    assert_eq!(sent(&next), vec![(3, MapRequestKind::LayerProbe)]);
    let (_, next) = s.complete(3, ok(json!({})));
    assert!(next.is_empty(), "one probe of layer a, not two: {next:?}");
}

#[test]
fn a_failed_probe_is_a_request_failure_not_a_field_failure() {
    let mut s = MapRequestScheduler::new(1);
    s.submit_field(field(1));
    s.complete(1, ok(load_result("f1")));
    s.submit_probe(probe("a"));
    let (completion, _) = s.complete(
        2,
        MapOutcome::Failure {
            message: "invalid_layer: not a pmtiles archive".into(),
            cancelled: false,
        },
    );
    assert!(matches!(
        completion,
        MapCompletion::RequestFailed {
            kind: MapRequestKind::LayerProbe,
            ..
        }
    ));
    assert!(matches!(s.field_state(), FieldState::Ready { .. }));
}

fn export(field_revision: u64) -> ExportIntent {
    ExportIntent {
        field_revision,
        body: json!({"target": "/tmp/m.png", "format": "png"}),
    }
}

#[test]
fn an_export_waits_for_the_running_request_and_goes_before_tiles() {
    let mut s = MapRequestScheduler::new(1);
    s.submit_field(field(1));
    s.complete(1, ok(load_result("f1")));
    let first = s.submit_tiles(tiles(1, "t0"));
    assert_eq!(sent(&first), vec![(2, MapRequestKind::RenderTiles)]);
    assert!(s.submit_tiles(tiles(1, "t1")).is_empty());
    assert!(s.submit_export(export(1)).is_empty());
    let (_, next) = s.complete(2, ok(json!({})));
    assert_eq!(sent(&next), vec![(3, MapRequestKind::Export)]);
    assert_eq!(MapRequestKind::Export.wire_kind(), "export");
    assert_eq!(
        MapRequestKind::from_wire("export"),
        Some(MapRequestKind::Export)
    );
    let (completion, next) = s.complete(3, ok(json!({"target": "/tmp/m.png"})));
    assert!(matches!(
        completion,
        MapCompletion::Completed {
            kind: MapRequestKind::Export,
            ..
        }
    ));
    assert_eq!(sent(&next), vec![(4, MapRequestKind::RenderTiles)]);
}

#[test]
fn a_queued_export_is_dropped_when_a_newer_field_is_requested() {
    let mut s = MapRequestScheduler::new(1);
    s.submit_field(field(1));
    s.complete(1, ok(load_result("f1")));
    s.submit_tiles(tiles(1, "t0"));
    s.submit_export(export(1));
    s.submit_field(field(2));
    let (_, next) = s.complete(2, ok(json!({})));
    assert_eq!(sent(&next), vec![(3, MapRequestKind::LoadField)]);
    s.complete(3, ok(load_result("f2")));
    // No export request ever started: the snapshot belonged to field 1.
    assert!(s.running_request().is_none());
}

#[test]
fn an_export_without_a_forecast_field_is_never_dropped_by_a_field_load() {
    let mut s = MapRequestScheduler::new(1);
    s.submit_tiles(tiles(0, "t0"));
    s.submit_export(export(0));
    s.submit_field(field(1));
    let (_, next) = s.complete(1, ok(json!({})));
    assert_eq!(sent(&next), vec![(2, MapRequestKind::LoadField)]);
}

#[test]
fn user_cancel_clears_queued_work_and_retains_last_good_field_as_stale() {
    let mut s = MapRequestScheduler::new(1);
    s.submit_field(field(1));
    // Initial load cancelled before any field existed -> clean Idle, no error banner.
    assert_eq!(s.cancel_running(), Some((1, MapRequestKind::LoadField)));
    assert_eq!(
        s.cancel_running(),
        None,
        "duplicate cancel does not spam interrupts"
    );
    let (completion, next) = s.complete(
        1,
        MapOutcome::Failure {
            message: "cancelled".into(),
            cancelled: true,
        },
    );
    assert_eq!(completion, MapCompletion::FieldCancelled);
    assert!(matches!(s.field_state(), FieldState::Idle));
    assert!(next.is_empty());

    // Load a good field, then start reload + queue tiles/export, and cancel.
    s.submit_field(field(2));
    s.complete(2, ok(load_result("f2")));
    s.submit_field(field(3));
    s.submit_tiles(tiles(3, "t3"));
    assert_eq!(s.cancel_running(), Some((3, MapRequestKind::LoadField)));
    let (completion, next) = s.complete(
        3,
        MapOutcome::Failure {
            message: "cancelled".into(),
            cancelled: true,
        },
    );
    assert!(matches!(completion, MapCompletion::FieldFailedStale(_)));
    assert!(matches!(
        s.field_state(),
        FieldState::Stale { revision: 2, .. }
    ));
    assert!(next.is_empty(), "queued work was cleared on cancel");
}
