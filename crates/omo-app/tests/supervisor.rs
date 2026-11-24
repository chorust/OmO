use std::{
    collections::VecDeque,
    fs, thread,
    time::{Duration, Instant},
};

use omo_app::host::{
    cancel::{CancelAction, CancelController},
    supervisor::{
        EngineSupervisor, EventDisposition, SupervisorCore, WorkerEvent, WorkerFactory, WorkerPoll,
        WorkerProcess,
    },
};
use omo_protocol::{
    messages::{Envelope, failed_body, parse_fail_kind},
    state::{CancelLevel, FailKind, JobState},
};
use serde_json::{Value, json};

struct FakeWorker {
    generation: u64,
}

impl FakeWorker {
    fn emit(&self, kind: &str, body: serde_json::Value) -> WorkerEvent {
        WorkerEvent(Envelope::new(self.generation, 11, 22, kind, body))
    }

    fn crash(&self, core: &mut SupervisorCore, now: Instant) {
        core.worker_crashed(now);
    }
}

#[test]
fn stale_generation_frames_and_referenced_files_are_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let result_path = dir.path().join("old.arrow");
    let plot_path = dir.path().join("old-plot.arrow");
    fs::write(&result_path, b"stale result").unwrap();
    fs::write(&plot_path, b"stale plot sample").unwrap();
    let current_worker = FakeWorker { generation: 2 };
    let old_worker = FakeWorker { generation: 1 };
    let mut core = SupervisorCore::with_result_root(2, Instant::now(), dir.path());
    let stale_result = old_worker.emit(
        "result",
        json!({
            "result_ref": {
                "id":"old",
                "generation":1,
                "path":result_path,
                "rows":1,
                "schema_hash":"a".repeat(64),
                "truncated":false,
                "limits":{"rows":10,"bytes":100}
            },
            "plot_ref": {
                "id":"old-plot",
                "generation":1,
                "path":plot_path,
                "rows":1,
                "schema_hash":"b".repeat(64),
                "truncated":false,
                "limits":{"rows":10,"bytes":100}
            },
            "matches_ref": {
                "id":"old-matches",
                "generation":1,
                "path":dir.path().join("old-matches.arrow"),
                "rows":1,
                "schema_hash":"c".repeat(64),
                "truncated":false,
                "limits":{"rows":10,"bytes":100}
            },
            "parquet_path": dir.path().join("old.parquet")
        }),
    );
    fs::write(dir.path().join("old-matches.arrow"), b"stale matches").unwrap();
    fs::write(dir.path().join("old.parquet"), b"stale parquet").unwrap();

    assert!(matches!(
        core.accept_event(stale_result),
        EventDisposition::DiscardedStale {
            result_files_deleted: 4
        }
    ));
    assert!(!result_path.exists());
    assert!(!plot_path.exists());
    assert!(!dir.path().join("old-matches.arrow").exists());
    assert!(!dir.path().join("old.parquet").exists());
    assert_eq!(
        core.accept_event(current_worker.emit("heartbeat", json!({}))),
        EventDisposition::Accepted
    );
}

#[test]
fn crash_fails_active_jobs_and_advances_generation() {
    let now = Instant::now();
    let worker = FakeWorker { generation: 4 };
    let mut core = SupervisorCore::new(4, now);
    core.start_job(11);
    core.start_job(12);
    worker.crash(&mut core, now + Duration::from_millis(1));

    assert_eq!(core.generation(), 5);
    assert_eq!(
        core.job_state(11),
        Some(&JobState::Failed(FailKind::WorkerCrashed))
    );
    assert_eq!(
        core.job_state(12),
        Some(&JobState::Failed(FailKind::WorkerCrashed))
    );
}

#[test]
fn missed_heartbeat_triggers_kill() {
    let now = Instant::now();
    let core = SupervisorCore::new(8, now);
    assert!(!core.heartbeat_expired(now + Duration::from_secs(4), Duration::from_secs(5)));
    assert!(core.heartbeat_expired(now + Duration::from_secs(5), Duration::from_secs(5)));
}

#[test]
fn cancellation_escalates_interrupt_drain_then_kill_within_ten_seconds() {
    let now = Instant::now();
    let mut cancel = CancelController::new(Duration::from_secs(10));
    assert_eq!(
        cancel.request(44, false, now),
        CancelAction::SendInterrupt {
            target_request_id: 44
        }
    );
    let deadline = match cancel.begin_drain(now) {
        CancelAction::DrainUntil(deadline) => deadline,
        action => panic!("expected drain, got {action:?}"),
    };
    assert_eq!(
        cancel.poll(deadline - Duration::from_millis(1)),
        CancelAction::DrainUntil(deadline)
    );
    assert_eq!(cancel.poll(deadline), CancelAction::KillWorker);
}

#[test]
fn application_exit_kills_without_waiting_for_drain() {
    let mut cancel = CancelController::new(Duration::from_secs(10));
    assert_eq!(
        cancel.request(1, true, Instant::now()),
        CancelAction::KillWorker
    );
}

#[derive(Clone, Copy)]
enum ScriptedMode {
    CrashOnQuery,
    AckCancelThenExit,
    ResultThenExit,
}

struct ScriptedFactory {
    mode: ScriptedMode,
}

impl WorkerFactory for ScriptedFactory {
    fn spawn(&mut self, generation: u64) -> Result<Box<dyn WorkerProcess>, String> {
        Ok(Box::new(ScriptedWorker {
            generation,
            mode: self.mode,
            events: VecDeque::new(),
            exit_next_poll: false,
            last_heartbeat: Instant::now(),
        }))
    }
}

struct ScriptedWorker {
    generation: u64,
    mode: ScriptedMode,
    events: VecDeque<WorkerEvent>,
    exit_next_poll: bool,
    last_heartbeat: Instant,
}

impl WorkerProcess for ScriptedWorker {
    fn send(&mut self, request: Envelope<Value>) -> Result<(), String> {
        match (self.mode, request.kind.as_str()) {
            (ScriptedMode::CrashOnQuery, "query" | "series") => {
                self.exit_next_poll = true;
            }
            (ScriptedMode::AckCancelThenExit, "interrupt") => {
                let target_id = request
                    .body
                    .get("target_request_id")
                    .and_then(Value::as_u64)
                    .unwrap_or(request.request_id);
                self.events.push_back(WorkerEvent(Envelope::new(
                    self.generation,
                    request.job_id,
                    target_id,
                    "failed",
                    failed_body(
                        target_id,
                        &FailKind::Cancelled(CancelLevel::Interrupt),
                        "interrupted",
                    ),
                )));
                self.exit_next_poll = true;
            }
            (ScriptedMode::ResultThenExit, "query") => {
                self.events.push_back(WorkerEvent(Envelope::new(
                    self.generation,
                    request.job_id,
                    request.request_id,
                    "result",
                    json!({"request_id": request.request_id, "result_ref": {"id": "ok", "rows": 1}}),
                )));
                self.exit_next_poll = true;
            }
            _ => {}
        }
        Ok(())
    }

    fn poll(&mut self) -> Result<WorkerPoll, String> {
        let mut events: Vec<WorkerEvent> = self.events.drain(..).collect();
        if self.exit_next_poll {
            self.exit_next_poll = false;
            return Ok(WorkerPoll {
                events,
                exited: true,
                ..WorkerPoll::default()
            });
        }
        if self.last_heartbeat.elapsed() >= Duration::from_millis(500) {
            self.last_heartbeat = Instant::now();
            events.push(WorkerEvent(Envelope::new(
                self.generation,
                0,
                0,
                "heartbeat",
                json!({"t_ms": 0}),
            )));
        }
        Ok(WorkerPoll {
            events,
            ..WorkerPoll::default()
        })
    }

    fn kill(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn supervisor_emits_worker_crashed_failure_once_for_active_job() {
    let dir = tempfile::tempdir().unwrap();
    let mut supervisor = EngineSupervisor::start(
        ScriptedFactory {
            mode: ScriptedMode::CrashOnQuery,
        },
        json!({}),
        dir.path(),
    )
    .unwrap();
    let generation = supervisor.generation();
    supervisor
        .send_request(Envelope::new(
            generation,
            15,
            99,
            "query",
            json!({"sql": "SELECT 1"}),
        ))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut failures = Vec::new();
    while Instant::now() < deadline {
        if let Some(event) = supervisor.try_recv().unwrap()
            && event.0.kind == "failed"
        {
            failures.push(event);
        }
        if supervisor.generation() > generation && !failures.is_empty() {
            thread::sleep(Duration::from_millis(120));
            while let Some(event) = supervisor.try_recv().unwrap() {
                if event.0.kind == "failed" {
                    failures.push(event);
                }
            }
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        failures.len(),
        1,
        "expected exactly one crash failure event"
    );
    let failed = &failures[0].0;
    assert_eq!(failed.worker_generation, generation);
    assert_eq!(failed.job_id, 15);
    assert_eq!(failed.request_id, 99);
    assert_eq!(parse_fail_kind(&failed.body), Some(FailKind::WorkerCrashed));
    assert!(supervisor.generation() > generation);
    supervisor.shutdown();
}

#[test]
fn supervisor_cancel_ack_does_not_duplicate_failure_if_worker_then_exits() {
    let dir = tempfile::tempdir().unwrap();
    let mut supervisor = EngineSupervisor::start(
        ScriptedFactory {
            mode: ScriptedMode::AckCancelThenExit,
        },
        json!({}),
        dir.path(),
    )
    .unwrap();
    let generation = supervisor.generation();
    supervisor
        .send_request(Envelope::new(
            generation,
            21,
            77,
            "query",
            json!({"sql": "SELECT 1"}),
        ))
        .unwrap();
    supervisor.cancel(21, 77).unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut failures = Vec::new();
    while Instant::now() < deadline {
        if let Some(event) = supervisor.try_recv().unwrap()
            && event.0.kind == "failed"
        {
            failures.push(event);
        }
        if supervisor.generation() > generation && !failures.is_empty() {
            thread::sleep(Duration::from_millis(120));
            while let Some(event) = supervisor.try_recv().unwrap() {
                if event.0.kind == "failed" {
                    failures.push(event);
                }
            }
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        failures.len(),
        1,
        "cancel ack followed by worker exit must emit only one terminal failure"
    );
    assert_eq!(
        parse_fail_kind(&failures[0].0.body),
        Some(FailKind::Cancelled(CancelLevel::Interrupt))
    );
    supervisor.shutdown();
}

#[test]
fn supervisor_result_then_exit_does_not_emit_spurious_crash_failure() {
    let dir = tempfile::tempdir().unwrap();
    let mut supervisor = EngineSupervisor::start(
        ScriptedFactory {
            mode: ScriptedMode::ResultThenExit,
        },
        json!({}),
        dir.path(),
    )
    .unwrap();
    let generation = supervisor.generation();
    supervisor
        .send_request(Envelope::new(
            generation,
            31,
            88,
            "query",
            json!({"sql": "SELECT 1"}),
        ))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut results = 0usize;
    let mut failures = 0usize;
    while Instant::now() < deadline {
        if let Some(event) = supervisor.try_recv().unwrap() {
            match event.0.kind.as_str() {
                "result" => results += 1,
                "failed" => failures += 1,
                _ => {}
            }
        }
        if supervisor.generation() > generation && results > 0 {
            thread::sleep(Duration::from_millis(120));
            while let Some(event) = supervisor.try_recv().unwrap() {
                match event.0.kind.as_str() {
                    "result" => results += 1,
                    "failed" => failures += 1,
                    _ => {}
                }
            }
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(results, 1, "completed result must be delivered once");
    assert_eq!(
        failures, 0,
        "subsequent worker exit must not emit a crash failure for an already-completed request"
    );
    assert!(supervisor.generation() > generation);
    supervisor.shutdown();
}

#[test]
fn series_crash_emits_worker_crashed_and_interrupt_cancel_emits_cancelled_interrupt() {
    let dir = tempfile::tempdir().unwrap();
    let mut supervisor = EngineSupervisor::start(
        ScriptedFactory {
            mode: ScriptedMode::CrashOnQuery,
        },
        json!({}),
        dir.path(),
    )
    .unwrap();
    let generation = supervisor.generation();
    supervisor
        .send_request(Envelope::new(
            generation,
            41,
            101,
            "series",
            json!({"result_id": "s1"}),
        ))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut crash_failure = None;
    while Instant::now() < deadline {
        if let Some(event) = supervisor.try_recv().unwrap()
            && event.0.kind == "failed"
            && event.0.request_id == 101
        {
            crash_failure = Some(event);
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let crash_failure = crash_failure.expect("series crash must emit failed");
    assert_eq!(
        parse_fail_kind(&crash_failure.0.body),
        Some(FailKind::WorkerCrashed)
    );
    assert!(supervisor.generation() > generation);
    supervisor.shutdown();

    let mut cancel_sup = EngineSupervisor::start(
        ScriptedFactory {
            mode: ScriptedMode::AckCancelThenExit,
        },
        json!({}),
        dir.path(),
    )
    .unwrap();
    let cancel_gen = cancel_sup.generation();
    cancel_sup
        .send_request(Envelope::new(
            cancel_gen,
            42,
            102,
            "series",
            json!({"result_id": "s2"}),
        ))
        .unwrap();
    cancel_sup.cancel(42, 102).unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut cancel_failure = None;
    while Instant::now() < deadline {
        if let Some(event) = cancel_sup.try_recv().unwrap()
            && event.0.kind == "failed"
            && event.0.request_id == 102
        {
            cancel_failure = Some(event);
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let cancel_failure = cancel_failure.expect("series cancel must emit failed");
    assert_eq!(
        parse_fail_kind(&cancel_failure.0.body),
        Some(FailKind::Cancelled(CancelLevel::Interrupt))
    );
    cancel_sup.shutdown();
}
