//! Integration tests for M1 worker handlers (`catalog_check`, `resolve_run`, `sql_session`).

use std::{
    io::{Cursor, Write},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use omo_protocol::{
    frame::{decode_frame, encode_frame},
    messages::{Envelope, parse_fail_kind},
    state::FailKind,
};
use omo_worker::server::{BackendError, StageReporter, WorkerBackend, WorkerExit, serve};
use serde_json::{Value, json};

#[derive(Default)]
struct M1MockBackend {
    stages: Option<StageReporter>,
}

impl WorkerBackend for M1MockBackend {
    fn set_stage_reporter(&mut self, reporter: Option<StageReporter>) {
        self.stages = reporter;
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
    fn query(&mut self, body: &Value) -> Result<Value, BackendError> {
        Ok(json!({"sql": body["sql"]}))
    }
    fn catalog_check(&mut self, body: &Value) -> Result<Value, BackendError> {
        Ok(json!({
            "entry_id": body["entry"]["id"],
            "readiness": { "status": "not_ready", "reason": "NoCompleteRun", "detail": "none" }
        }))
    }
    fn resolve_run(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Err(BackendError::typed(
            FailKind::NoCompleteRun,
            "no complete run found",
        ))
    }
    fn series(&mut self, _body: &Value) -> Result<Value, BackendError> {
        if let Some(stages) = &self.stages {
            stages.set_stage("planning");
            stages.set_stage("reading");
            stages.set_stage("writing");
        }
        Ok(json!({
            "result_ref": {"id": "series-1", "rows": 2},
            "scan_metrics": {"bytes_read": 2048, "duration_ms": 42}
        }))
    }
    fn sql_session(&mut self, body: &Value) -> Result<Value, BackendError> {
        let mut views = vec!["stations"];
        if body.get("series_parquet").and_then(Value::as_str).is_some() {
            views.push("station_series");
        }
        Ok(json!({ "views": views }))
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

#[test]
fn m1_worker_dispatches_catalog_check_resolve_run_and_sql_session() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    let stdout = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stdout_clone = Arc::clone(&stdout);
    let handle = thread::spawn(move || {
        let mut stderr = Vec::new();
        serve(reader, stdout_clone, &mut stderr, M1MockBackend::default()).unwrap()
    });

    let wait_for = |req_id: u64| {
        for _ in 0..200 {
            let events = decode_all(&stdout.lock().unwrap());
            if events
                .iter()
                .any(|e| e.request_id == req_id && e.kind != "progress")
            {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for request {req_id}");
    };

    writer
        .write_all(&encode(Envelope::new(
            1,
            0,
            1,
            "hello",
            json!({"bootstrap_config": {}}),
        )))
        .unwrap();
    wait_for(1);

    writer
        .write_all(&encode(Envelope::new(
            1,
            0,
            2,
            "catalog_check",
            json!({"entry": {"id": "ecmwf_ifs.temperature_2m"}, "progress_hz": 0}),
        )))
        .unwrap();
    wait_for(2);

    writer
        .write_all(&encode(Envelope::new(
            1,
            0,
            3,
            "sql_session",
            json!({"stations_ref": "/tmp/stations.csv", "series_parquet": "/tmp/s.parquet", "progress_hz": 0}),
        )))
        .unwrap();
    wait_for(3);

    writer
        .write_all(&encode(Envelope::new(
            1,
            0,
            4,
            "resolve_run",
            json!({"entry": {"id": "ecmwf_ifs.temperature_2m"}, "run": {"kind": "latest_complete"}, "progress_hz": 0}),
        )))
        .unwrap();
    wait_for(4);

    writer
        .write_all(&encode(Envelope::new(
            1,
            0,
            5,
            "series",
            json!({"progress_hz": 0}),
        )))
        .unwrap();
    wait_for(5);

    writer
        .write_all(&encode(Envelope::new(1, 0, 6, "shutdown", json!({}))))
        .unwrap();
    assert_eq!(handle.join().unwrap(), WorkerExit::Shutdown);

    let events = decode_all(&stdout.lock().unwrap());
    let non_progress: Vec<_> = events.iter().filter(|e| e.kind != "progress").collect();
    assert_eq!(non_progress[0].kind, "ready");
    assert_eq!(non_progress[1].kind, "result");
    assert_eq!(non_progress[1].body["entry_id"], "ecmwf_ifs.temperature_2m");
    assert_eq!(non_progress[2].kind, "result");
    assert_eq!(
        non_progress[2].body["views"],
        json!(["stations", "station_series"])
    );
    assert_eq!(non_progress[3].kind, "failed");
    assert_eq!(
        parse_fail_kind(&non_progress[3].body),
        Some(FailKind::NoCompleteRun)
    );
    assert_eq!(non_progress[4].kind, "result");
    assert_eq!(non_progress[4].body["scan_metrics"]["bytes_read"], 2048);
    assert_eq!(non_progress[4].body["scan_metrics"]["duration_ms"], 42);
    let series_stages: Vec<_> = events
        .iter()
        .filter(|e| e.request_id == 5 && e.kind == "progress")
        .filter_map(|e| e.body.get("stage").and_then(Value::as_str))
        .collect();
    assert_eq!(series_stages, vec!["planning", "reading", "writing"]);
}
