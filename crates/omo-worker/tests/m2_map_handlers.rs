//! Integration tests for the M2 map request kinds on the worker stdio server (`T023`):
//! dispatch, `request_id` injection, `busy` rejection and interruption. The real engine
//! handlers (field load / render / inspect) are covered live by `scripts/ci-verify-map.py`.

use std::{
    io::{Cursor, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use omo_protocol::{
    frame::{decode_frame, encode_frame},
    messages::{Envelope, parse_fail_kind},
    state::{CancelLevel, FailKind},
};
use omo_worker::server::{BackendError, StageReporter, WorkerBackend, WorkerExit, serve};
use serde_json::{Value, json};

#[derive(Default)]
struct MapMock {
    stages: Option<StageReporter>,
    cancelled: Arc<AtomicBool>,
}

impl MapMock {
    fn block_until_cancelled(&self) -> Result<Value, BackendError> {
        for _ in 0..400 {
            if self.cancelled.load(Ordering::SeqCst) {
                return Err(BackendError::new("map field load interrupted"));
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(json!({"timed_out": true}))
    }
}

impl WorkerBackend for MapMock {
    fn set_stage_reporter(&mut self, reporter: Option<StageReporter>) {
        self.stages = reporter;
    }
    fn cancellation(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
        self.cancelled.store(false, Ordering::SeqCst);
        let flag = Arc::clone(&self.cancelled);
        Some(Box::new(move || flag.store(true, Ordering::SeqCst)))
    }
    fn bootstrap(
        &mut self,
        _config: &Value,
        _diagnostics: &mut dyn Write,
    ) -> Result<Value, String> {
        Ok(json!({"channel": "mock"}))
    }
    fn probe_task(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn query(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Ok(json!({}))
    }
    fn map_load_field(&mut self, body: &Value) -> Result<Value, BackendError> {
        if let Some(stages) = &self.stages {
            stages.set_stage("planning");
            stages.set_stage("reading");
            stages.set_stage("writing");
        }
        if body["block"].as_bool() == Some(true) {
            return self.block_until_cancelled();
        }
        Ok(json!({"field_id": body["field_id"], "source_layout": "data_spatial"}))
    }
    fn map_render_tiles(&mut self, body: &Value) -> Result<Value, BackendError> {
        Ok(json!({
            "result_id": body["result_id"],
            "revision": body["revision"],
            "tiles": [],
            "layer_errors": []
        }))
    }
    fn map_inspect(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Ok(json!({"sample": {"state": "not_loaded"}}))
    }
    fn map_layer_probe(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Err(BackendError::typed(
            FailKind::InvalidLayer,
            "not a PMTiles v3 archive",
        ))
    }
    fn interrupt(&mut self, _request_id: u64) -> Result<(), String> {
        Ok(())
    }
    fn ensure_spatial(&mut self) -> Result<Value, String> {
        Ok(json!({}))
    }
}

fn encode(env: Envelope<Value>) -> Vec<u8> {
    encode_frame(&serde_json::to_vec(&env).unwrap()).unwrap()
}

fn decode_all(buf: &[u8]) -> Vec<Envelope<Value>> {
    let mut cursor = Cursor::new(buf);
    let mut events = Vec::new();
    while (cursor.position() as usize) < buf.len() {
        let frame = decode_frame(&mut cursor).unwrap();
        events.push(serde_json::from_slice::<Envelope<Value>>(&frame).unwrap());
    }
    events
}

struct Session {
    writer: std::io::PipeWriter,
    stdout: Arc<Mutex<Vec<u8>>>,
    handle: thread::JoinHandle<WorkerExit>,
}

impl Session {
    fn start() -> Self {
        let (reader, mut writer) = std::io::pipe().unwrap();
        let stdout = Arc::new(Mutex::new(Vec::<u8>::new()));
        let stdout_clone = Arc::clone(&stdout);
        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            serve(reader, stdout_clone, &mut stderr, MapMock::default()).unwrap()
        });
        writer
            .write_all(&encode(Envelope::new(
                1,
                0,
                1,
                "hello",
                json!({"bootstrap_config": {}}),
            )))
            .unwrap();
        let session = Self {
            writer,
            stdout,
            handle,
        };
        session.wait_for(1, |_| true);
        session
    }

    fn send(&mut self, request_id: u64, kind: &str, body: Value) {
        self.writer
            .write_all(&encode(Envelope::new(1, 0, request_id, kind, body)))
            .unwrap();
    }

    /// Wait for a non-progress event of `request_id` matching `accept`.
    fn wait_for(
        &self,
        request_id: u64,
        accept: impl Fn(&Envelope<Value>) -> bool,
    ) -> Envelope<Value> {
        for _ in 0..600 {
            let events = decode_all(&self.stdout.lock().unwrap());
            if let Some(event) = events
                .into_iter()
                .find(|e| e.request_id == request_id && e.kind != "progress" && accept(e))
            {
                return event;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for request {request_id}");
    }

    fn finish(mut self) -> Vec<Envelope<Value>> {
        self.send(99, "shutdown", json!({}));
        assert_eq!(self.handle.join().unwrap(), WorkerExit::Shutdown);
        decode_all(&self.stdout.lock().unwrap())
    }
}

#[test]
fn map_requests_dispatch_and_results_carry_the_request_id() {
    let mut session = Session::start();
    session.send(
        2,
        "map_load_field",
        json!({"field_id": "f1", "progress_hz": 0}),
    );
    let load = session.wait_for(2, |_| true);
    assert_eq!(load.kind, "result");
    assert_eq!(load.body["request_id"], 2);
    assert_eq!(load.body["field_id"], "f1");

    session.send(
        3,
        "map_render_tiles",
        json!({"result_id": "r1", "revision": 7, "progress_hz": 0}),
    );
    let render = session.wait_for(3, |_| true);
    assert_eq!(render.kind, "result");
    assert_eq!(render.body["request_id"], 3);
    assert_eq!(render.body["revision"], 7);

    session.send(
        4,
        "map_inspect",
        json!({"field_id": "f1", "progress_hz": 0}),
    );
    let inspect = session.wait_for(4, |_| true);
    assert_eq!(inspect.kind, "result");
    assert_eq!(inspect.body["sample"]["state"], "not_loaded");

    session.send(5, "map_layer_probe", json!({"progress_hz": 0}));
    let probe = session.wait_for(5, |_| true);
    assert_eq!(probe.kind, "failed");
    assert_eq!(parse_fail_kind(&probe.body), Some(FailKind::InvalidLayer));

    let events = session.finish();
    let stages: Vec<_> = events
        .iter()
        .filter(|e| e.request_id == 2 && e.kind == "progress")
        .filter_map(|e| e.body.get("stage").and_then(Value::as_str))
        .collect();
    assert!(
        stages.contains(&"planning") && stages.contains(&"reading"),
        "{stages:?}"
    );
}

#[test]
fn a_second_map_request_is_rejected_as_busy_and_does_not_disturb_the_first() {
    let mut session = Session::start();
    session.send(
        2,
        "map_load_field",
        json!({"block": true, "progress_hz": 0}),
    );
    thread::sleep(Duration::from_millis(60));
    session.send(
        3,
        "map_render_tiles",
        json!({"result_id": "r", "revision": 1}),
    );
    let busy = session.wait_for(3, |_| true);
    assert_eq!(busy.kind, "failed");
    match parse_fail_kind(&busy.body) {
        Some(FailKind::Query(message)) => assert!(message.starts_with("busy"), "{message}"),
        other => panic!("expected busy, got {other:?}"),
    }
    // Free the worker so shutdown is prompt.
    session.send(4, "interrupt", json!({"target_request_id": 2}));
    session.wait_for(2, |_| true);
    session.finish();
}

#[test]
fn interrupting_a_field_load_reports_cancelled_and_frees_the_worker() {
    let mut session = Session::start();
    session.send(
        2,
        "map_load_field",
        json!({"block": true, "progress_hz": 0}),
    );
    thread::sleep(Duration::from_millis(60));
    session.send(3, "interrupt", json!({"target_request_id": 2}));
    let cancelled = session.wait_for(2, |_| true);
    assert_eq!(cancelled.kind, "failed");
    assert_eq!(
        parse_fail_kind(&cancelled.body),
        Some(FailKind::Cancelled(CancelLevel::Interrupt))
    );
    // The worker accepts a new map request afterwards.
    session.send(
        4,
        "map_inspect",
        json!({"field_id": "f1", "progress_hz": 0}),
    );
    assert_eq!(session.wait_for(4, |_| true).kind, "result");
    session.finish();
}
