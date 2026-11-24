use std::path::PathBuf;

use omo_protocol::{
    generation::{GenerationDisposition, classify_generation},
    messages::{Envelope, MessageBody, failed_body, parse_fail_kind},
    result_ref::{ResultLimits, ResultRef},
    state::{CancelLevel, FailKind, JobState},
};
use serde_json::json;

#[test]
fn envelope_and_body_round_trip_as_json() {
    let message = Envelope::new(4, 12, 30, "query", json!({"sql":"select 1"}));
    let encoded = serde_json::to_vec(&message).expect("serialize");
    let decoded: Envelope<MessageBody> = serde_json::from_slice(&encoded).expect("deserialize");
    assert_eq!(decoded, message);
    assert_eq!(decoded.v, 1);
}

#[test]
fn result_ref_and_state_are_serializable() {
    let reference = ResultRef {
        id: "r-1".into(),
        generation: 4,
        path: PathBuf::from("/tmp/g4/r-1.arrow"),
        rows: 9,
        schema_hash: "a".repeat(64),
        truncated: true,
        limits: ResultLimits {
            rows: 10,
            bytes: 1000,
        },
    };
    let value = serde_json::to_value(&reference).expect("serialize ref");
    assert_eq!(value["generation"], 4);
    assert_eq!(value["limits"]["bytes"], 1000);
    assert_eq!(
        serde_json::from_value::<ResultRef>(value).unwrap(),
        reference
    );

    let state = JobState::Failed(FailKind::WorkerCrashed);
    assert_eq!(
        serde_json::from_str::<JobState>(&serde_json::to_string(&state).unwrap()).unwrap(),
        state
    );
    assert_eq!(
        CancelLevel::Drain { timeout_ms: 10_000 }.timeout_ms(),
        Some(10_000)
    );
}

#[test]
fn stale_generation_is_classified_for_discard() {
    assert_eq!(classify_generation(7, 7), GenerationDisposition::Current);
    assert_eq!(classify_generation(6, 7), GenerationDisposition::Stale);
}

#[test]
fn failed_event_body_matches_literal_wire_contract() {
    let cancelled = failed_body(
        3,
        &FailKind::Cancelled(CancelLevel::Interrupt),
        "interrupted",
    );
    assert_eq!(
        cancelled,
        json!({
            "request_id": 3,
            "kind": {
                "kind": "cancelled",
                "detail": { "level": "interrupt" }
            },
            "message": "interrupted"
        })
    );
    assert_eq!(
        parse_fail_kind(&cancelled),
        Some(FailKind::Cancelled(CancelLevel::Interrupt))
    );

    let crashed = failed_body(9, &FailKind::WorkerCrashed, "worker exited");
    assert_eq!(
        crashed,
        json!({
            "request_id": 9,
            "kind": { "kind": "worker_crashed" },
            "message": "worker exited"
        })
    );
    assert_eq!(parse_fail_kind(&crashed), Some(FailKind::WorkerCrashed));

    let export = failed_body(12, &FailKind::Export("cross-fs".into()), "cross-fs");
    assert_eq!(
        export,
        json!({
            "request_id": 12,
            "kind": { "kind": "export", "detail": "cross-fs" },
            "message": "cross-fs"
        })
    );
    assert_eq!(
        parse_fail_kind(&export),
        Some(FailKind::Export("cross-fs".into()))
    );
}
