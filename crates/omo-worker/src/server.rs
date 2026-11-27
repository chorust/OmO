use std::{
    io::{self, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use omo_protocol::{
    frame::{FrameError, decode_frame, write_frame},
    messages::{Envelope, MessageBody, PROTOCOL_VERSION, failed_body},
    state::{CancelLevel, FailKind},
};
use serde_json::{Value, json};
use thiserror::Error;

use crate::heartbeat;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendError {
    pub kind: Option<FailKind>,
    pub message: String,
}

impl BackendError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            kind: None,
            message: message.into(),
        }
    }

    pub fn typed(kind: FailKind, message: impl Into<String>) -> Self {
        Self {
            kind: Some(kind),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BackendError {}

impl From<String> for BackendError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for BackendError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

/// Handle passed to a running backend task so it can report stage transitions
/// (`planning`, `reading`, `writing`, `committing`) immediately on the protocol stream.
#[derive(Clone)]
pub struct StageReporter {
    stage: Arc<Mutex<&'static str>>,
    emit: Arc<dyn Fn(&'static str) + Send + Sync>,
}

impl StageReporter {
    pub fn set_stage(&self, stage: &'static str) {
        if let Ok(mut guard) = self.stage.lock() {
            *guard = stage;
        }
        (self.emit)(stage);
    }
}

pub trait WorkerBackend: Send + 'static {
    fn set_generation(&mut self, _generation: u64) {}
    fn set_stage_reporter(&mut self, _reporter: Option<StageReporter>) {}
    fn cancellation(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
        None
    }
    fn bootstrap(&mut self, config: &Value, diagnostics: &mut dyn Write) -> Result<Value, String>;
    fn probe_task(&mut self) -> Result<(), String>;
    fn query(&mut self, body: &Value) -> Result<Value, BackendError>;
    fn series_plan(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Err("series_plan is unavailable".into())
    }
    fn catalog_check(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Err("catalog_check is unavailable".into())
    }
    fn resolve_run(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Err("resolve_run is unavailable".into())
    }
    fn series(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Err("series is unavailable".into())
    }
    fn sql_session(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Err("sql_session is unavailable".into())
    }
    fn export(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Err("export is unavailable".into())
    }
    fn map_load_field(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Err("map_load_field is unavailable".into())
    }
    fn map_render_tiles(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Err("map_render_tiles is unavailable".into())
    }
    fn map_inspect(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Err("map_inspect is unavailable".into())
    }
    fn map_layer_probe(&mut self, _body: &Value) -> Result<Value, BackendError> {
        Err("map_layer_probe is unavailable".into())
    }
    fn interrupt(&mut self, request_id: u64) -> Result<(), String>;
    fn ensure_spatial(&mut self) -> Result<Value, String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerExit {
    InputClosed,
    Shutdown,
    BootstrapFailed,
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("worker frame failed: {0}")]
    Frame(#[from] FrameError),
    #[error("worker message JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("worker stdout lock is poisoned")]
    OutputPoisoned,
    #[error("worker diagnostics failed: {0}")]
    Diagnostics(#[from] io::Error),
}

pub fn run_stdio<B: WorkerBackend>(backend: B) -> Result<WorkerExit, ServerError> {
    serve(
        io::stdin(),
        heartbeat::stdout_writer(),
        &mut io::stderr(),
        backend,
    )
}

pub fn serve<R, W, L, B>(
    reader: R,
    stdout: Arc<Mutex<W>>,
    diagnostics: &mut L,
    backend: B,
) -> Result<WorkerExit, ServerError>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
    L: Write,
    B: WorkerBackend,
{
    let current_generation = Arc::new(AtomicU64::new(0));
    let heartbeat = heartbeat::start(Arc::clone(&stdout), Arc::clone(&current_generation));
    let result = serve_loop(reader, &stdout, diagnostics, backend, &current_generation);
    heartbeat.stop();
    result
}

struct ActiveRequest {
    request: Envelope<MessageBody>,
    cancel: Option<Box<dyn Fn() + Send + Sync>>,
    interrupted: bool,
}

fn serve_loop<R, W, L, B>(
    mut reader: R,
    stdout: &Arc<Mutex<W>>,
    diagnostics: &mut L,
    backend: B,
    current_generation: &AtomicU64,
) -> Result<WorkerExit, ServerError>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
    L: Write,
    B: WorkerBackend,
{
    use std::sync::mpsc;
    use std::time::Duration;
    let (input_tx, input_rx) = mpsc::channel();
    std::thread::spawn(move || {
        loop {
            let frame = decode_frame(&mut reader);
            let finished = frame.is_err();
            if input_tx.send(frame).is_err() || finished {
                break;
            }
        }
    });
    let (done_tx, done_rx) = mpsc::channel::<(B, Result<Value, BackendError>)>();
    let mut backend = Some(backend);
    let mut active: Option<ActiveRequest> = None;
    let mut ready = false;
    loop {
        if let Ok((returned_backend, outcome)) = done_rx.try_recv() {
            backend = Some(returned_backend);
            if let Some(active_request) = active.take() {
                let request = active_request.request;
                match outcome {
                    Ok(result_ref) if request.kind == "export" => {
                        send_event(stdout, &request, "export_committed", result_ref)?
                    }
                    Ok(payload)
                        if matches!(
                            request.kind.as_str(),
                            "catalog_check" | "resolve_run" | "series" | "sql_session"
                        ) || is_map_request(&request.kind) =>
                    {
                        let mut body = match payload {
                            Value::Object(map) => Value::Object(map),
                            other => json!({ "payload": other }),
                        };
                        if let Some(obj) = body.as_object_mut() {
                            obj.insert("request_id".into(), json!(request.request_id));
                        }
                        send_event(stdout, &request, "result", body)?
                    }
                    Ok(query_result) => send_event(
                        stdout,
                        &request,
                        "result",
                        query_result_event_body(request.request_id, query_result),
                    )?,
                    Err(error) if active_request.interrupted => send_failed(
                        stdout,
                        &request,
                        FailKind::Cancelled(CancelLevel::Interrupt),
                        &error.message,
                    )?,
                    Err(error) => {
                        let fail_kind = classify_task_failure(&request.kind, &error);
                        send_failed(stdout, &request, fail_kind, &error.message)?;
                    }
                }
            }
        }
        let frame = match input_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Ok(frame)) => frame,
            Ok(Err(FrameError::TruncatedLength { actual: 0 }))
            | Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(ActiveRequest {
                    cancel: Some(cancel),
                    ..
                }) = &active
                {
                    cancel();
                }
                return Ok(WorkerExit::InputClosed);
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
        };
        let request: Envelope<MessageBody> = serde_json::from_slice(&frame)?;
        if request.v != PROTOCOL_VERSION {
            send_failed(
                stdout,
                &request,
                FailKind::Query("protocol: unsupported protocol version".into()),
                "unsupported protocol version",
            )?;
            continue;
        }
        if ready && request.worker_generation != current_generation.load(Ordering::Acquire) {
            send_failed(
                stdout,
                &request,
                FailKind::Query("protocol: worker generation mismatch".into()),
                "worker generation mismatch",
            )?;
            continue;
        }
        match request.kind.as_str() {
            "hello" if !ready && active.is_none() => {
                current_generation.store(request.worker_generation, Ordering::Release);
                let engine = backend.as_mut().expect("idle backend");
                engine.set_generation(request.worker_generation);
                let config = request.body.get("bootstrap_config").unwrap_or(&Value::Null);
                writeln!(
                    diagnostics,
                    "received hello for generation {}",
                    request.worker_generation
                )?;
                match engine.bootstrap(config, diagnostics) {
                    Ok(report) => {
                        ready = true;
                        send_event(
                            stdout,
                            &request,
                            "ready",
                            json!({"bootstrap_report": report}),
                        )?;
                    }
                    Err(message) => {
                        send_event(
                            stdout,
                            &request,
                            "bootstrap_failed",
                            json!({"error": {"kind":"bootstrap", "message":message}, "report": Value::Null}),
                        )?;
                        return Ok(WorkerExit::BootstrapFailed);
                    }
                }
            }
            kind if is_task_request(kind) && ready && active.is_none() => {
                let mut engine = backend.take().expect("idle backend");
                if let Err(message) = engine.probe_task() {
                    send_failed(
                        stdout,
                        &request,
                        FailKind::Bootstrap(message.clone()),
                        &message,
                    )?;
                    return Ok(WorkerExit::BootstrapFailed);
                }
                let cancellation = engine.cancellation();
                let mut body = request.body.clone();
                if let Some(object) = body.as_object_mut() {
                    if matches!(request.kind.as_str(), "query" | "series_plan" | "series")
                        && !object.contains_key("result_id")
                    {
                        object.insert(
                            "result_id".into(),
                            json!(format!("{}-{}", request.job_id, request.request_id)),
                        );
                    }
                    if request.kind == "export" {
                        object.insert("request_id".into(), json!(request.request_id));
                    }
                }
                let tx = done_tx.clone();
                let task_kind = request.kind.clone();
                let initial_stage = match task_kind.as_str() {
                    "series" | "map_load_field" => "planning",
                    "map_render_tiles" | "map_inspect" | "map_layer_probe" => "reading",
                    "export" => "writing",
                    _ => "reading",
                };
                let (progress, reporter) = ProgressEmitter::start(
                    Arc::clone(stdout),
                    request.clone(),
                    initial_stage,
                    request
                        .body
                        .get("progress_hz")
                        .and_then(Value::as_u64)
                        .map_or(DEFAULT_PROGRESS_HZ, |hz| hz.min(MAX_PROGRESS_HZ)),
                );
                engine.set_stage_reporter(Some(reporter));
                std::thread::spawn(move || {
                    let result = match task_kind.as_str() {
                        "export" => engine.export(&body),
                        "series_plan" => engine.series_plan(&body),
                        "catalog_check" => engine.catalog_check(&body),
                        "resolve_run" => engine.resolve_run(&body),
                        "series" => engine.series(&body),
                        "sql_session" => engine.sql_session(&body),
                        "map_load_field" => engine.map_load_field(&body),
                        "map_render_tiles" => engine.map_render_tiles(&body),
                        "map_inspect" => engine.map_inspect(&body),
                        "map_layer_probe" => engine.map_layer_probe(&body),
                        _ => engine.query(&body),
                    };
                    engine.set_stage_reporter(None);
                    progress.stop();
                    let _ = tx.send((engine, result));
                });
                active = Some(ActiveRequest {
                    request,
                    cancel: cancellation,
                    interrupted: false,
                });
            }
            "interrupt" => {
                let target = request
                    .body
                    .get("target_request_id")
                    .and_then(Value::as_u64);
                match active.as_mut() {
                    Some(ActiveRequest {
                        request: running,
                        cancel: Some(cancel),
                        interrupted,
                    }) if Some(running.request_id) == target => {
                        cancel();
                        *interrupted = true;
                    }
                    _ => send_failed(
                        stdout,
                        &request,
                        FailKind::Query("cancel: no matching interruptible request".into()),
                        "no matching interruptible request",
                    )?,
                }
            }
            "ensure_spatial" if ready && active.is_none() => {
                match backend.as_mut().expect("idle backend").ensure_spatial() {
                    Ok(report) => send_event(stdout, &request, "result", report)?,
                    Err(message) => send_failed(
                        stdout,
                        &request,
                        FailKind::Capability(message.clone()),
                        &message,
                    )?,
                }
            }
            "shutdown" => {
                if let Some(ActiveRequest {
                    cancel: Some(cancel),
                    ..
                }) = &active
                {
                    cancel();
                }
                return Ok(WorkerExit::Shutdown);
            }
            kind if is_task_request(kind) && !ready => send_failed(
                stdout,
                &request,
                FailKind::Bootstrap("worker is not bootstrapped".into()),
                "worker is not bootstrapped",
            )?,
            kind if is_task_request(kind) || kind == "ensure_spatial" => send_failed(
                stdout,
                &request,
                FailKind::Query("busy: worker already has an active request".into()),
                "worker already has an active request",
            )?,
            other => {
                let message = format!("unsupported request kind `{other}`");
                send_failed(
                    stdout,
                    &request,
                    FailKind::Query(format!("protocol: {message}")),
                    &message,
                )?;
            }
        }
    }
}

/// M2 map requests; their results are plain `result` bodies like the M1 request kinds.
fn is_map_request(kind: &str) -> bool {
    matches!(
        kind,
        "map_load_field" | "map_render_tiles" | "map_inspect" | "map_layer_probe"
    )
}

/// Requests that occupy the single worker task slot.
fn is_task_request(kind: &str) -> bool {
    matches!(
        kind,
        "query"
            | "export"
            | "series_plan"
            | "catalog_check"
            | "resolve_run"
            | "series"
            | "sql_session"
    ) || is_map_request(kind)
}

fn query_result_event_body(request_id: u64, payload: Value) -> Value {
    let result_ref = payload
        .get("result_ref")
        .cloned()
        .unwrap_or_else(|| payload.clone());
    let mut body = json!({"request_id": request_id, "result_ref": result_ref});
    if let Some(plot_ref) = payload.get("plot_ref") {
        body["plot_ref"] = plot_ref.clone();
    }
    if let Some(source_rows) = payload.get("plot_source_rows").and_then(Value::as_u64) {
        body["plot_source_rows"] = json!(source_rows);
    }
    if let Some(scan_metrics) = payload.get("scan_metrics") {
        body["scan_metrics"] = scan_metrics.clone();
    }
    if let Some(plan_details) = payload.get("plan_details") {
        body["plan_details"] = plan_details.clone();
    }
    body
}

fn classify_task_failure(request_kind: &str, error: &BackendError) -> FailKind {
    if let Some(kind) = &error.kind {
        return kind.clone();
    }
    let message = error.message.as_str();
    if (message.starts_with("capability `") && message.contains("is unavailable"))
        || message.contains("during ensure_spatial")
    {
        FailKind::Capability(message.to_owned())
    } else if request_kind == "export" {
        FailKind::Export(message.to_owned())
    } else {
        FailKind::Query(message.to_owned())
    }
}

fn send_failed<W: Write>(
    stdout: &Arc<Mutex<W>>,
    request: &Envelope<MessageBody>,
    kind: FailKind,
    message: &str,
) -> Result<(), ServerError> {
    send_event(
        stdout,
        request,
        "failed",
        failed_body(request.request_id, &kind, message),
    )
}

fn send_event<W: Write>(
    stdout: &Arc<Mutex<W>>,
    request: &Envelope<MessageBody>,
    kind: &str,
    body: Value,
) -> Result<(), ServerError> {
    let event = Envelope::new(
        request.worker_generation,
        request.job_id,
        request.request_id,
        kind,
        body,
    );
    let bytes = serde_json::to_vec(&event)?;
    let mut writer = stdout.lock().map_err(|_| ServerError::OutputPoisoned)?;
    write_frame(&mut *writer, &bytes)?;
    Ok(())
}

/// Progress events per second the worker emits while a task runs when the request does not
/// ask for another rate, and the highest rate it honours.
const DEFAULT_PROGRESS_HZ: u64 = 2;
const MAX_PROGRESS_HZ: u64 = 200;

/// Emits `progress` frames for one running request until it finishes. The task thread owns
/// the emitter, so a panic in the task still stops the frames when it unwinds.
struct ProgressEmitter {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ProgressEmitter {
    fn start<W>(
        stdout: Arc<Mutex<W>>,
        request: Envelope<MessageBody>,
        initial_stage: &'static str,
        hz: u64,
    ) -> (Self, StageReporter)
    where
        W: Write + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let stage = Arc::new(Mutex::new(initial_stage));
        let sequence = Arc::new(AtomicU64::new(0));
        let started = Instant::now();

        let emit_stdout = Arc::clone(&stdout);
        let emit_request = request.clone();
        let emit_seq = Arc::clone(&sequence);
        let emit_fn: Arc<dyn Fn(&'static str) + Send + Sync> = Arc::new(move |stage_name| {
            let seq = emit_seq.fetch_add(1, Ordering::Relaxed).saturating_add(1);
            let event = Envelope::new(
                emit_request.worker_generation,
                emit_request.job_id,
                emit_request.request_id,
                "progress",
                json!({
                    "request_id": emit_request.request_id,
                    "stage": stage_name,
                    "elapsed_ms": started.elapsed().as_millis() as u64,
                    "sequence": seq,
                }),
            );
            if let Ok(bytes) = serde_json::to_vec(&event)
                && let Ok(mut writer) = emit_stdout.lock()
            {
                let _ = write_frame(&mut *writer, &bytes);
            }
        });
        let reporter = StageReporter {
            stage: Arc::clone(&stage),
            emit: Arc::clone(&emit_fn),
        };

        if hz == 0 {
            return (Self { stop, thread: None }, reporter);
        }
        let thread_stop = Arc::clone(&stop);
        let thread_stage = Arc::clone(&stage);
        let thread = thread::spawn(move || {
            let period = Duration::from_nanos(1_000_000_000 / hz);
            loop {
                thread::park_timeout(period);
                if thread_stop.load(Ordering::Acquire) {
                    break;
                }
                let current_stage = thread_stage
                    .lock()
                    .map(|guard| *guard)
                    .unwrap_or(initial_stage);
                let seq = sequence.fetch_add(1, Ordering::Relaxed).saturating_add(1);
                let event = Envelope::new(
                    request.worker_generation,
                    request.job_id,
                    request.request_id,
                    "progress",
                    json!({
                        "request_id": request.request_id,
                        "stage": current_stage,
                        "elapsed_ms": started.elapsed().as_millis() as u64,
                        "sequence": seq,
                    }),
                );
                let Ok(bytes) = serde_json::to_vec(&event) else {
                    continue;
                };
                let Ok(mut writer) = stdout.lock() else {
                    break;
                };
                if write_frame(&mut *writer, &bytes).is_err() {
                    break;
                }
            }
        });
        (
            Self {
                stop,
                thread: Some(thread),
            },
            reporter,
        )
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
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
    use serde_json::{Value, json};

    use super::{BackendError, WorkerBackend, WorkerExit, query_result_event_body, serve};

    struct TestBackend {
        cancelled: Arc<AtomicBool>,
    }

    impl Default for TestBackend {
        fn default() -> Self {
            Self {
                cancelled: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    impl WorkerBackend for TestBackend {
        fn cancellation(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
            let cancelled = Arc::clone(&self.cancelled);
            Some(Box::new(move || cancelled.store(true, Ordering::Release)))
        }

        fn bootstrap(
            &mut self,
            _config: &Value,
            _diagnostics: &mut dyn Write,
        ) -> Result<Value, String> {
            Ok(json!({"channel": "test"}))
        }

        fn probe_task(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn query(&mut self, body: &Value) -> Result<Value, BackendError> {
            if body.get("capability_error").and_then(Value::as_bool) == Some(true) {
                return Err("capability `httpfs` is unavailable: offline".into());
            }
            if body.get("wait_for_cancel").and_then(Value::as_bool) == Some(true) {
                for _ in 0..50 {
                    if self.cancelled.load(Ordering::Acquire) {
                        return Err("interrupted".into());
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                return Err("timed out waiting for cancel".into());
            }
            Ok(json!({"result_ref": {"id": body["result_id"], "rows": 1}}))
        }

        fn series_plan(&mut self, body: &Value) -> Result<Value, BackendError> {
            match body.get("plan").and_then(Value::as_str) {
                Some(plan @ ("A" | "B")) => Ok(json!({
                    "result_ref": {"id": body["result_id"], "rows": 27},
                    "scan_metrics": {"plan": plan, "scan_complete": true},
                })),
                _ => Err("series_plan plan must be \"A\" or \"B\"".into()),
            }
        }

        fn export(&mut self, _body: &Value) -> Result<Value, BackendError> {
            Err("overwrite confirmation is required".into())
        }

        fn interrupt(&mut self, _request_id: u64) -> Result<(), String> {
            Ok(())
        }

        fn ensure_spatial(&mut self) -> Result<Value, String> {
            Err("spatial extension missing".into())
        }
    }

    fn encode(env: Envelope<Value>) -> Vec<u8> {
        encode_frame(&serde_json::to_vec(&env).unwrap()).unwrap()
    }

    fn decode_all(bytes: &[u8]) -> Vec<Envelope<Value>> {
        let mut cursor = Cursor::new(bytes);
        let mut out = Vec::new();
        while (cursor.position() as usize) < bytes.len() {
            let frame = decode_frame(&mut cursor).unwrap();
            let env: Envelope<Value> = serde_json::from_slice(&frame).unwrap();
            if env.kind != "heartbeat" && env.kind != "progress" {
                out.push(env);
            }
        }
        out
    }

    #[test]
    fn result_event_keeps_full_result_and_publishes_plot_artifact() {
        let body = query_result_event_body(
            17,
            json!({
                "result_ref": {"id": "query", "rows": 20_000},
                "plot_ref": {"id": "query-plot", "rows": 10_000},
                "plot_source_rows": 20_000,
                "scan_metrics": {"scan_complete": true}
            }),
        );

        assert_eq!(body["request_id"], 17);
        assert_eq!(body["result_ref"]["id"], "query");
        assert_eq!(body["plot_ref"]["id"], "query-plot");
        assert_eq!(body["plot_source_rows"], 20_000);
        assert_eq!(body["scan_metrics"]["scan_complete"], true);
    }

    #[test]
    fn legacy_backend_result_remains_a_result_ref() {
        let body = query_result_event_body(18, json!({"id": "legacy", "rows": 1}));
        assert_eq!(body["result_ref"]["id"], "legacy");
        assert!(body.get("plot_ref").is_none());
    }

    #[test]
    fn series_plan_and_structured_fail_kinds_follow_worker_contract() {
        let mut input = Vec::new();
        input.extend(encode(Envelope::new(1, 0, 1, "hello", json!({}))));
        input.extend(encode(Envelope::new(
            1,
            2,
            2,
            "series_plan",
            json!({"plan": "A", "progress_hz": 0}),
        )));
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(&input).unwrap();
        let stdout = Arc::new(Mutex::new(Vec::<u8>::new()));
        let stdout_clone = Arc::clone(&stdout);
        let handle = thread::spawn(move || {
            let mut stderr = Vec::new();
            serve(reader, stdout_clone, &mut stderr, TestBackend::default()).unwrap()
        });

        // Wait for series_plan A to finish, then send B, malformed series_plan, export fail, capability fail, unmatched interrupt, and cancelable query.
        let wait_for_request = |req_id: u64| {
            for _ in 0..100 {
                let events = decode_all(&stdout.lock().unwrap());
                if events.iter().any(|e| e.request_id == req_id) {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
            panic!("timed out waiting for request {req_id}");
        };

        wait_for_request(2);
        writer
            .write_all(&encode(Envelope::new(
                1,
                2,
                3,
                "series_plan",
                json!({"plan": "B", "progress_hz": 0}),
            )))
            .unwrap();
        wait_for_request(3);

        writer
            .write_all(&encode(Envelope::new(
                1,
                2,
                4,
                "series_plan",
                json!({"plan": "Z", "progress_hz": 0}),
            )))
            .unwrap();
        wait_for_request(4);

        writer
            .write_all(&encode(Envelope::new(
                1,
                3,
                5,
                "export",
                json!({"progress_hz": 0}),
            )))
            .unwrap();
        wait_for_request(5);

        writer
            .write_all(&encode(Envelope::new(
                1,
                4,
                6,
                "query",
                json!({"capability_error": true, "progress_hz": 0}),
            )))
            .unwrap();
        wait_for_request(6);

        writer
            .write_all(&encode(Envelope::new(
                1,
                5,
                7,
                "interrupt",
                json!({"target_request_id": 999}),
            )))
            .unwrap();
        wait_for_request(7);

        writer
            .write_all(&encode(Envelope::new(
                1,
                6,
                8,
                "query",
                json!({"wait_for_cancel": true, "progress_hz": 0}),
            )))
            .unwrap();
        thread::sleep(Duration::from_millis(50));
        writer
            .write_all(&encode(Envelope::new(
                1,
                6,
                9,
                "interrupt",
                json!({"target_request_id": 8}),
            )))
            .unwrap();
        wait_for_request(8);

        writer
            .write_all(&encode(Envelope::new(1, 0, 10, "shutdown", json!({}))))
            .unwrap();
        drop(writer);

        assert_eq!(handle.join().unwrap(), WorkerExit::Shutdown);
        let events = decode_all(&stdout.lock().unwrap());
        let by_req = |id: u64| events.iter().find(|e| e.request_id == id).unwrap().clone();

        let plan_a = by_req(2);
        assert_eq!(plan_a.kind, "result");
        assert_eq!(plan_a.body["scan_metrics"]["plan"], "A");

        let plan_b = by_req(3);
        assert_eq!(plan_b.kind, "result");
        assert_eq!(plan_b.body["scan_metrics"]["plan"], "B");

        let bad_plan = by_req(4);
        assert_eq!(bad_plan.kind, "failed");
        assert!(matches!(
            parse_fail_kind(&bad_plan.body),
            Some(FailKind::Query(_))
        ));

        let export_fail = by_req(5);
        assert_eq!(export_fail.kind, "failed");
        assert!(matches!(
            parse_fail_kind(&export_fail.body),
            Some(FailKind::Export(_))
        ));

        let cap_fail = by_req(6);
        assert_eq!(cap_fail.kind, "failed");
        assert!(matches!(
            parse_fail_kind(&cap_fail.body),
            Some(FailKind::Capability(_))
        ));

        let unmatched_int = by_req(7);
        assert_eq!(unmatched_int.kind, "failed");
        assert!(matches!(
            parse_fail_kind(&unmatched_int.body),
            Some(FailKind::Query(_))
        ));

        let cancelled = by_req(8);
        assert_eq!(cancelled.kind, "failed");
        assert_eq!(
            parse_fail_kind(&cancelled.body),
            Some(FailKind::Cancelled(CancelLevel::Interrupt))
        );
    }
}
