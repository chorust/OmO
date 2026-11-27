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
            (
                ScriptedMode::CrashOnQuery,
                "query" | "series" | "map_load_field" | "map_render_tiles" | "map_inspect"
                | "map_layer_probe",
            ) => {
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

#[test]
fn map_request_kinds_are_tracked_and_crash_as_worker_crashed() {
    for (index, kind) in [
        "map_load_field",
        "map_render_tiles",
        "map_inspect",
        "map_layer_probe",
    ]
    .into_iter()
    .enumerate()
    {
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
        let request_id = 300 + index as u64;
        supervisor
            .send_request(Envelope::new(generation, 77, request_id, kind, json!({})))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut failure = None;
        while Instant::now() < deadline {
            if let Some(event) = supervisor.try_recv().unwrap()
                && event.0.kind == "failed"
                && event.0.request_id == request_id
            {
                failure = Some(event);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let failure = failure.unwrap_or_else(|| panic!("{kind} crash must emit failed"));
        assert_eq!(
            parse_fail_kind(&failure.0.body),
            Some(FailKind::WorkerCrashed),
            "{kind}"
        );
        assert!(supervisor.generation() > generation, "{kind}");
        supervisor.shutdown();
    }
}

#[test]
fn stale_map_results_delete_rgba_png_and_f32bin_files() {
    let dir = tempfile::tempdir().unwrap();
    let gen_dir = dir.path().join("g1");
    fs::create_dir_all(&gen_dir).unwrap();
    let tile = gen_dir.join("tile_forecast_5_26_12.rgba");
    let png = gen_dir.join("export_staging.png");
    let field = gen_dir.join("field_f1.f32bin");
    let keep = gen_dir.join("notes.txt");
    for path in [&tile, &png, &field, &keep] {
        fs::write(path, b"stale").unwrap();
    }
    let mut core = SupervisorCore::with_result_root(2, Instant::now(), dir.path());
    let disposition = core.accept_event(WorkerEvent(Envelope::new(
        1,
        11,
        22,
        "result",
        json!({
            "tiles": [{"layer_id": "forecast", "rgba_path": tile}],
            "readout_path": field,
            "png_path": png,
            "other_path": keep,
        }),
    )));
    assert_eq!(
        disposition,
        EventDisposition::DiscardedStale {
            result_files_deleted: 3
        }
    );
    assert!(!tile.exists() && !png.exists() && !field.exists());
    assert!(keep.exists(), "non-whitelisted files must survive");
}

#[test]
fn crash_reaps_the_whole_stale_generation_directory_recursively() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("g1");
    fs::create_dir_all(old.join("nested")).unwrap();
    fs::write(old.join("tile_a.rgba"), b"a").unwrap();
    fs::write(old.join("nested").join("field.f32bin"), b"b").unwrap();
    fs::write(old.join("leftover.partial"), b"c").unwrap();
    let next = dir.path().join("g2");
    fs::create_dir_all(&next).unwrap();
    fs::write(next.join("tile_b.rgba"), b"keep").unwrap();
    let unrelated = dir.path().join("graphs");
    fs::create_dir_all(&unrelated).unwrap();
    fs::write(unrelated.join("x.rgba"), b"keep").unwrap();

    let mut core = SupervisorCore::with_result_root(1, Instant::now(), dir.path());
    core.worker_crashed(Instant::now());
    assert_eq!(core.generation(), 2);
    assert!(!old.exists(), "g1 must be reaped after the crash");
    assert!(
        next.join("tile_b.rgba").exists(),
        "current generation untouched"
    );
    assert!(
        unrelated.join("x.rgba").exists(),
        "only g<digits> dirs are reaped"
    );

    // Application shutdown reaps the generation that is still current.
    assert_eq!(core.reap_current_generation(), 1);
    assert!(!next.exists());
    assert!(unrelated.exists());
}
