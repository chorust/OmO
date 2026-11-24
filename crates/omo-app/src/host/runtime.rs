//! Out-of-process engine owner and event bridge for the NSH UI.
//!
//! Configuration, subprocess control, protocol polling, and export recovery all run off the UI
//! engine thread. The UI talks to this bridge through bounded event delivery and a small command
//! channel.

use std::{
    collections::HashMap,
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread,
    time::Duration,
};

use omo_protocol::{
    messages::{Envelope, failed_body, parse_fail_kind},
    state::{CancelLevel, FailKind},
};
use serde_json::{Value, json};

use super::{
    export_scan::{ExportRecovery, ExportVerdict, scan_export_dir},
    supervisor::{EngineSupervisor, WorkerEvent},
};

const EVENT_CAPACITY: usize = 256;
const RECENT_EXPORT_DIRS: &str = "recent-export-dirs.json";

struct ActiveExport {
    target: PathBuf,
    failure_emitted: bool,
}
const MAX_RECENT_EXPORT_DIRS: usize = 64;

#[derive(Debug)]
pub enum RuntimeEvent {
    Worker(WorkerEvent),
    StartupFailed(String),
    RecoveryScanFailed(String),
    Recovery(ExportRecovery),
}

#[derive(Clone)]
pub struct EngineHandle {
    commands: mpsc::Sender<RuntimeCommand>,
    generation: Arc<AtomicU64>,
}

impl EngineHandle {
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn send_request(
        &self,
        job_id: u64,
        request_id: u64,
        kind: impl Into<String>,
        body: Value,
    ) -> Result<(), String> {
        self.commands
            .send(RuntimeCommand::Request {
                job_id,
                request_id,
                kind: kind.into(),
                body,
            })
            .map_err(|error| error.to_string())
    }

    pub fn cancel(&self, job_id: u64, target_request_id: u64) -> Result<(), String> {
        self.commands
            .send(RuntimeCommand::Cancel {
                job_id,
                target_request_id,
            })
            .map_err(|error| error.to_string())
    }
}

enum RuntimeCommand {
    Request {
        job_id: u64,
        request_id: u64,
        kind: String,
        body: Value,
    },
    Cancel {
        job_id: u64,
        target_request_id: u64,
    },
}

struct RuntimeConfig {
    base_data_dir: PathBuf,
    session_data_dir: PathBuf,
    worker_executable: PathBuf,
    bootstrap_config: Value,
}

#[derive(Clone)]
struct PreviewIntent {
    request: Envelope<Value>,
    sent_generation: Option<u64>,
}

impl PreviewIntent {
    fn for_generation(&mut self, generation: u64) -> Option<Envelope<Value>> {
        if self.sent_generation == Some(generation) {
            return None;
        }
        self.request.worker_generation = generation;
        self.sent_generation = Some(generation);
        Some(self.request.clone())
    }
}

/// Start the runtime using an explicit JSON config path from `OMOBENCH_BOOTSTRAP_CONFIG`.
/// The file and the sibling worker are resolved on the bridge thread, never on the UI thread.
pub fn start_from_env() -> (EngineHandle, Receiver<RuntimeEvent>) {
    let (command_tx, command_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::sync_channel(EVENT_CAPACITY);
    let generation = Arc::new(AtomicU64::new(0));
    let thread_generation = Arc::clone(&generation);
    let thread_events = event_tx.clone();

    let spawn = thread::Builder::new()
        .name("omobench-engine-bridge".into())
        .spawn(move || run_bridge(command_rx, thread_events, thread_generation));
    if let Err(error) = spawn {
        publish_terminal(
            &event_tx,
            RuntimeEvent::StartupFailed(format!("could not start engine bridge thread: {error}")),
        );
    }

    (
        EngineHandle {
            commands: command_tx,
            generation,
        },
        event_rx,
    )
}

fn run_bridge(
    commands: Receiver<RuntimeCommand>,
    events: SyncSender<RuntimeEvent>,
    generation: Arc<AtomicU64>,
) {
    let config_path = match env::var_os("OMOBENCH_BOOTSTRAP_CONFIG") {
        Some(path) => PathBuf::from(path),
        None => {
            publish_terminal(
                &events,
                RuntimeEvent::StartupFailed(
                    "OMOBENCH_BOOTSTRAP_CONFIG must name a JSON bootstrap config".into(),
                ),
            );
            return;
        }
    };
    let worker_override = env::var_os("OMOBENCH_WORKER").map(PathBuf::from);
    let config = match load_runtime_config(&config_path, worker_override) {
        Ok(config) => config,
        Err(error) => {
            publish_terminal(&events, RuntimeEvent::StartupFailed(error));
            return;
        }
    };

    if let Err(error) = scan_recent_export_dirs(&config.base_data_dir, &events) {
        publish_terminal(&events, RuntimeEvent::RecoveryScanFailed(error.to_string()));
    }

    let result_root = config.session_data_dir.join("results");
    let mut supervisor = match EngineSupervisor::start_child(
        &config.worker_executable,
        config.bootstrap_config,
        &result_root,
    ) {
        Ok(supervisor) => supervisor,
        Err(error) => {
            let _ = fs::remove_dir_all(&config.session_data_dir);
            publish_terminal(
                &events,
                RuntimeEvent::StartupFailed(format!("could not start engine worker: {error}")),
            );
            return;
        }
    };
    generation.store(supervisor.generation(), Ordering::Release);

    let mut ready_generation = 0u64;
    let mut observed_generation = supervisor.generation();
    let mut restart_count = 0u64;
    let mut preview: Option<PreviewIntent> = None;
    let mut active_exports: HashMap<(u64, u64), ActiveExport> = HashMap::new();
    let mut shutting_down = false;

    while !shutting_down {
        match commands.recv_timeout(Duration::from_millis(20)) {
            Ok(RuntimeCommand::Request {
                job_id,
                request_id,
                kind,
                body,
            }) => {
                let request = Envelope::new(
                    supervisor.generation(),
                    job_id,
                    request_id,
                    kind.clone(),
                    body,
                );
                match kind.as_str() {
                    "query" => {
                        preview = Some(PreviewIntent {
                            request,
                            sent_generation: None,
                        });
                        if ready_generation == supervisor.generation() {
                            dispatch_preview(
                                &mut supervisor,
                                &mut preview,
                                ready_generation,
                                &events,
                            );
                        }
                    }
                    "export" => {
                        if let Err(error) =
                            persist_export_directory(&config.base_data_dir, &request.body)
                        {
                            let message = error.to_string();
                            publish_worker_failure(
                                &events,
                                &request,
                                FailKind::Export(message.clone()),
                                &message,
                            );
                            continue;
                        }
                        let target = request
                            .body
                            .get("target")
                            .and_then(Value::as_str)
                            .map(PathBuf::from);
                        if ready_generation != supervisor.generation() {
                            let message = "worker is not ready for export";
                            publish_worker_failure(
                                &events,
                                &request,
                                FailKind::Bootstrap(message.into()),
                                message,
                            );
                        } else if let Err(error) = supervisor.send_request(request.clone()) {
                            publish_worker_failure(
                                &events,
                                &request,
                                FailKind::WorkerCrashed,
                                &error.to_string(),
                            );
                        } else if let Some(target) = target {
                            active_exports.insert(
                                (job_id, request_id),
                                ActiveExport {
                                    target,
                                    failure_emitted: false,
                                },
                            );
                        }
                    }
                    _ => {
                        if ready_generation != supervisor.generation() && kind != "shutdown" {
                            let message = "worker is not ready";
                            publish_worker_failure(
                                &events,
                                &request,
                                FailKind::Bootstrap(message.into()),
                                message,
                            );
                        } else if let Err(error) = supervisor.send_request(request.clone()) {
                            publish_worker_failure(
                                &events,
                                &request,
                                FailKind::WorkerCrashed,
                                &error.to_string(),
                            );
                        }
                    }
                }
            }
            Ok(RuntimeCommand::Cancel {
                job_id,
                target_request_id,
            }) => {
                if preview
                    .as_ref()
                    .is_some_and(|intent| intent.request.request_id == target_request_id)
                {
                    preview = None;
                }
                if let Err(error) = supervisor.cancel(job_id, target_request_id) {
                    publish_terminal(
                        &events,
                        RuntimeEvent::StartupFailed(format!(
                            "could not send cancellation: {error}"
                        )),
                    );
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => shutting_down = true,
        }

        let next_generation = supervisor.generation();
        generation.store(next_generation, Ordering::Release);
        if next_generation > observed_generation {
            restart_count = restart_count.saturating_add(next_generation - observed_generation);
            observed_generation = next_generation;
            if restart_count > 3 {
                publish_terminal(&events, RuntimeEvent::StartupFailed(
                    "worker restarted repeatedly without becoming ready; check bootstrap configuration and worker diagnostics".into(),
                ));
                shutting_down = true;
            }
        }
        loop {
            match supervisor.try_recv() {
                Ok(Some(event)) => {
                    let envelope = &event.0;
                    if envelope.kind == "ready" {
                        let new_generation = envelope.worker_generation;
                        if ready_generation != 0 && ready_generation != new_generation {
                            recover_interrupted_exports(
                                &mut active_exports,
                                new_generation,
                                &config.base_data_dir,
                                &events,
                            );
                        }
                        ready_generation = new_generation;
                        observed_generation = new_generation;
                        restart_count = 0;
                        dispatch_preview(&mut supervisor, &mut preview, ready_generation, &events);
                    } else if is_terminal_event(&envelope.kind) {
                        if envelope.kind == "result" || envelope.kind == "failed" {
                            if preview.as_ref().is_some_and(|intent| {
                                intent.request.request_id == envelope.request_id
                            }) {
                                preview = None;
                            }
                        }
                        if envelope.kind == "export_committed" {
                            active_exports.remove(&(envelope.job_id, envelope.request_id));
                        } else if envelope.kind == "failed" {
                            let worker_died = matches!(
                                parse_fail_kind(&envelope.body),
                                Some(
                                    FailKind::WorkerCrashed
                                        | FailKind::Timeout
                                        | FailKind::Cancelled(CancelLevel::Kill)
                                )
                            );
                            if worker_died {
                                if let Some(export) =
                                    active_exports.get_mut(&(envelope.job_id, envelope.request_id))
                                {
                                    export.failure_emitted = true;
                                }
                            } else {
                                active_exports.remove(&(envelope.job_id, envelope.request_id));
                            }
                        }
                    }
                    publish_worker_event(&events, event);
                }
                Ok(None) => break,
                Err(error) => {
                    publish_terminal(
                        &events,
                        RuntimeEvent::StartupFailed(format!("engine supervisor stopped: {error}")),
                    );
                    shutting_down = true;
                    break;
                }
            }
        }
    }

    supervisor.shutdown();
    let _ = fs::remove_dir_all(&config.session_data_dir);
}

fn dispatch_preview(
    supervisor: &mut EngineSupervisor,
    preview: &mut Option<PreviewIntent>,
    generation: u64,
    events: &SyncSender<RuntimeEvent>,
) {
    let Some(intent) = preview.as_mut() else {
        return;
    };
    let Some(request) = intent.for_generation(generation) else {
        return;
    };
    if let Err(error) = supervisor.send_request(request.clone()) {
        publish_worker_failure(
            events,
            &request,
            FailKind::WorkerCrashed,
            &error.to_string(),
        );
    }
}

fn is_terminal_event(kind: &str) -> bool {
    matches!(
        kind,
        "result" | "failed" | "export_committed" | "bootstrap_failed"
    )
}

fn publish_worker_event(events: &SyncSender<RuntimeEvent>, event: WorkerEvent) {
    if matches!(event.0.kind.as_str(), "heartbeat" | "progress") {
        let _ = events.try_send(RuntimeEvent::Worker(event));
    } else {
        publish_terminal(events, RuntimeEvent::Worker(event));
    }
}

fn publish_terminal(events: &SyncSender<RuntimeEvent>, event: RuntimeEvent) {
    let _ = events.send(event);
}

fn publish_worker_failure(
    events: &SyncSender<RuntimeEvent>,
    request: &Envelope<Value>,
    kind: FailKind,
    message: &str,
) {
    let event = Envelope::new(
        request.worker_generation,
        request.job_id,
        request.request_id,
        "failed",
        failed_body(request.request_id, &kind, message),
    );
    publish_terminal(events, RuntimeEvent::Worker(WorkerEvent(event)));
}

fn load_runtime_config(
    path: &Path,
    worker_override: Option<PathBuf>,
) -> Result<RuntimeConfig, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("cannot read bootstrap config {}: {error}", path.display()))?;
    let mut bootstrap_config: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid bootstrap config {}: {error}", path.display()))?;
    let base_data_dir = bootstrap_config
        .get("data_dir")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or("bootstrap config must contain data_dir")?;
    if !base_data_dir.is_absolute() {
        return Err("bootstrap config data_dir must be absolute".into());
    }
    fs::create_dir_all(&base_data_dir).map_err(|error| {
        format!(
            "cannot create configured data_dir {}: {error}",
            base_data_dir.display()
        )
    })?;
    let session_data_dir = base_data_dir
        .join("sessions")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&session_data_dir).map_err(|error| {
        format!(
            "cannot create session data_dir {}: {error}",
            session_data_dir.display()
        )
    })?;
    bootstrap_config["data_dir"] = json!(session_data_dir);

    let worker_executable = match worker_override {
        Some(path) => path,
        None => {
            let executable =
                env::current_exe().map_err(|error| format!("cannot locate omo-app: {error}"))?;
            let parent = executable
                .parent()
                .ok_or("omo-app executable has no parent directory")?;
            parent.join(if cfg!(windows) {
                "omo-worker.exe"
            } else {
                "omo-worker"
            })
        }
    };
    if !worker_executable.is_file() {
        let _ = fs::remove_dir_all(&session_data_dir);
        return Err(format!(
            "engine worker executable does not exist: {}",
            worker_executable.display()
        ));
    }
    Ok(RuntimeConfig {
        base_data_dir,
        session_data_dir,
        worker_executable,
        bootstrap_config,
    })
}

fn recent_export_state_path(base_data_dir: &Path) -> PathBuf {
    base_data_dir.join(".omobench").join(RECENT_EXPORT_DIRS)
}

fn read_recent_export_dirs(base_data_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let path = recent_export_state_path(base_data_dir);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let dirs = serde_json::from_slice::<Vec<PathBuf>>(&bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid recent export directory state: {error}"),
        )
    })?;
    Ok(dirs
        .into_iter()
        .filter(|path| path.is_absolute())
        .take(MAX_RECENT_EXPORT_DIRS)
        .collect())
}

fn persist_export_directory(base_data_dir: &Path, body: &Value) -> io::Result<()> {
    let Some(target) = body
        .get("target")
        .and_then(Value::as_str)
        .map(PathBuf::from)
    else {
        return Ok(());
    };
    let Some(parent) = target.parent().filter(|path| path.is_absolute()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "export target must have an absolute parent directory",
        ));
    };
    let mut dirs = read_recent_export_dirs(base_data_dir)?;
    dirs.retain(|path| path != parent);
    dirs.insert(0, parent.to_owned());
    dirs.truncate(MAX_RECENT_EXPORT_DIRS);
    let state_path = recent_export_state_path(base_data_dir);
    let state_dir = state_path.parent().expect("state path has parent");
    fs::create_dir_all(state_dir)?;
    let temp = state_dir.join(format!(".recent-export-dirs-{}.tmp", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(&dirs).map_err(io::Error::other)?;
    let mut file = fs::File::create(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temp, &state_path)?;
    fs::File::open(state_dir)?.sync_all()?;
    Ok(())
}

fn scan_recent_export_dirs(
    base_data_dir: &Path,
    events: &SyncSender<RuntimeEvent>,
) -> io::Result<()> {
    for dir in read_recent_export_dirs(base_data_dir)? {
        match scan_export_dir(&dir) {
            Ok(recoveries) => {
                for recovery in recoveries {
                    publish_terminal(events, RuntimeEvent::Recovery(recovery));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn recover_interrupted_exports(
    active_exports: &mut HashMap<(u64, u64), ActiveExport>,
    new_generation: u64,
    base_data_dir: &Path,
    events: &SyncSender<RuntimeEvent>,
) {
    for ((job_id, request_id), export) in active_exports.drain() {
        let target = export.target;
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let recovered = scan_export_dir(parent)
            .unwrap_or_default()
            .into_iter()
            .find(|entry| entry.target == target)
            .unwrap_or_else(|| ExportRecovery {
                target: target.clone(),
                verdict: if crate::host::export_scan::is_saved(&target).unwrap_or(false) {
                    ExportVerdict::Finished
                } else {
                    ExportVerdict::Incomplete
                },
            });
        publish_terminal(events, RuntimeEvent::Recovery(recovered));
        if !export.failure_emitted {
            let message = "worker restarted during export; recovery was inspected";
            let failure = Envelope::new(
                new_generation,
                job_id,
                request_id,
                "failed",
                failed_body(request_id, &FailKind::Export(message.into()), message),
            );
            publish_terminal(events, RuntimeEvent::Worker(WorkerEvent(failure)));
        }
    }
    let _ = scan_recent_export_dirs(base_data_dir, events);
}

#[cfg(test)]
mod tests {
    use super::PreviewIntent;
    use omo_protocol::messages::Envelope;
    use serde_json::json;

    #[test]
    fn the_latest_uncancelled_preview_is_the_only_replay_intent() {
        let first = Envelope::new(4, 1, 10, "query", json!({"sql":"SELECT 1"}));
        let latest = Envelope::new(4, 1, 12, "query", json!({"sql":"SELECT 3"}));
        let mut replay = Some(PreviewIntent {
            request: first,
            sent_generation: Some(4),
        });
        let superseded = replay
            .replace(PreviewIntent {
                request: latest,
                sent_generation: None,
            })
            .unwrap();
        assert_eq!(superseded.request.request_id, 10);
        assert_eq!(replay.as_ref().unwrap().request.request_id, 12);
        let cancelled = replay.take().unwrap();
        assert_eq!(cancelled.request.request_id, 12);
        assert!(replay.is_none());
    }

    #[test]
    fn generation_change_updates_preview_before_dispatch() {
        // The bridge calls this only after the replacement worker reports ready.
        let mut replay = PreviewIntent {
            request: Envelope::new(4, 1, 12, "query", json!({"sql":"SELECT 3"})),
            sent_generation: Some(4),
        };
        let request = replay.for_generation(5).unwrap();
        assert_eq!(request.worker_generation, 5);
        assert!(replay.for_generation(5).is_none());
    }
}
