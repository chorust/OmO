use omo_app::host::preview_queue::{
    Completion, PreviewOutcome, PreviewQueue, PreviewRequest, QueueAction,
};
use omo_project::PreviewKey;
use serde_json::json;

fn key(revision: u64) -> PreviewKey {
    PreviewKey {
        doc_id: "doc".into(),
        revision,
        view_id: "sql".into(),
        query_hash: format!("q{revision}"),
        stations_hash: None,
        catalog_version: "catalog-0".into(),
        run_ref: "latest_complete".into(),
        window: omo_project::preview_key::PreviewWindow::default(),
        max_distance_m: 7200,
        worker_generation: 1,
    }
}

#[test]
fn one_running_preview_coalesces_updates_and_interrupts_superseded_work() {
    let mut queue = PreviewQueue::default();
    assert_eq!(
        queue.submit(PreviewRequest::new(key(1), 10)),
        QueueAction::Start(10)
    );
    assert_eq!(
        queue.submit(PreviewRequest::new(key(2), 11)),
        QueueAction::Interrupt { request_id: 10 }
    );
    assert_eq!(
        queue.submit(PreviewRequest::new(key(3), 12)),
        QueueAction::Queued
    );

    let completion = queue.complete(10, PreviewOutcome::Success(json!({"revision":1})));
    assert_eq!(completion, Completion::DiscardedSuperseded);
    assert_eq!(queue.running_request("doc"), Some(12));
}

#[test]
fn restart_after_supersede_promotes_only_the_latest_request_to_the_new_generation() {
    let mut queue = PreviewQueue::default();
    assert_eq!(
        queue.submit(PreviewRequest::new(key(1), 10)),
        QueueAction::Start(10)
    );
    assert_eq!(
        queue.submit(PreviewRequest::new(key(2), 11)),
        QueueAction::Interrupt { request_id: 10 }
    );
    assert_eq!(
        queue.submit(PreviewRequest::new(key(3), 12)),
        QueueAction::Queued
    );

    assert_eq!(
        queue.complete(
            10,
            PreviewOutcome::Failure("worker restarted during cancellation".into())
        ),
        Completion::DiscardedSuperseded
    );
    let latest = queue.take_dispatch("doc").unwrap();
    assert_eq!(latest.request_id, 12);
    assert!(queue.update_running_generation("doc", latest.request_id, 2));
    assert_eq!(
        queue.complete(12, PreviewOutcome::Success(json!({"generation":2}))),
        Completion::Accepted
    );
    assert_eq!(queue.last_good("doc").unwrap().key.revision, 3);
    assert_eq!(queue.last_good("doc").unwrap().key.worker_generation, 2);
}

#[test]
fn replayed_preview_tracks_the_replacement_worker_generation() {
    let mut queue = PreviewQueue::default();
    queue.submit(PreviewRequest::new(key(1), 10));

    assert!(queue.update_running_generation("doc", 10, 2));
    assert!(!queue.update_running_generation("doc", 99, 3));
    assert_eq!(
        queue.complete(10, PreviewOutcome::Success(json!({"generation":2}))),
        Completion::Accepted
    );
    assert_eq!(queue.last_good("doc").unwrap().key.worker_generation, 2);
}

#[test]
fn failed_preview_retains_last_good_result_and_marks_it_stale() {
    let mut queue = PreviewQueue::default();
    queue.submit(PreviewRequest::new(key(1), 10));
    assert_eq!(
        queue.complete(10, PreviewOutcome::Success(json!({"ok":true}))),
        Completion::Accepted
    );
    queue.submit(PreviewRequest::new(key(2), 11));

    assert_eq!(
        queue.complete(11, PreviewOutcome::Failure("query failed".into())),
        Completion::RetainedStale
    );
    let retained = queue.last_good("doc").unwrap();
    assert_eq!(retained.value, json!({"ok":true}));
    assert_eq!(queue.stale_error("doc"), Some("query failed"));
}

#[test]
fn superseded_window_change_drops_late_result_and_marks_previous_stale() {
    let mut queue = PreviewQueue::default();
    let k1 = key(1);
    queue.submit(PreviewRequest::new(k1.clone(), 10));
    assert_eq!(
        queue.complete(10, PreviewOutcome::Success(json!({"window":"0-168"}))),
        Completion::Accepted
    );

    let mut k2 = key(2);
    k2.window = omo_project::preview_key::PreviewWindow {
        start_h: 0,
        end_h: 72,
    };
    assert_eq!(
        queue.submit(PreviewRequest::new(k2.clone(), 11)),
        QueueAction::Start(11)
    );
    let mut k3 = key(3);
    k3.window = omo_project::preview_key::PreviewWindow {
        start_h: 0,
        end_h: 48,
    };
    assert_eq!(
        queue.submit(PreviewRequest::new(k3, 12)),
        QueueAction::Interrupt { request_id: 11 }
    );
    // Result for superseded window k2 arrives late -> dropped, previous good k1 stays retained.
    assert_eq!(
        queue.complete(11, PreviewOutcome::Success(json!({"window":"0-72"}))),
        Completion::DiscardedSuperseded
    );
    assert_eq!(queue.last_good("doc").unwrap().key, k1);
    assert_ne!(queue.last_good("doc").unwrap().key, k2);
}
