use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use omo_protocol::{
    messages::{Envelope, failed_body, parse_fail_kind},
    state::{CancelLevel, FailKind, JobState},
};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::{runtime::Builder, sync::mpsc as async_mpsc, time};

#[derive(Clone, Debug)]
pub struct WorkerEvent(pub Envelope<Value>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventDisposition {
    Accepted,
    DiscardedStale { result_files_deleted: usize },
}

pub struct SupervisorCore {
    generation: u64,
    last_heartbeat: Instant,
    jobs: HashMap<u64, JobState>,
    result_root: Option<PathBuf>,
}

impl SupervisorCore {
    pub fn new(generation: u64, now: Instant) -> Self {
        Self {
            generation,
            last_heartbeat: now,
            jobs: HashMap::new(),
            result_root: None,
        }
    }

    pub fn with_result_root(
        generation: u64,
        now: Instant,
        result_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            generation,
            last_heartbeat: now,
            jobs: HashMap::new(),
            result_root: Some(result_root.into()),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn start_job(&mut self, job_id: u64) {
        self.jobs.insert(job_id, JobState::Queued);
    }

    pub fn set_job_state(&mut self, job_id: u64, state: JobState) {
        self.jobs.insert(job_id, state);
    }

    pub fn job_state(&self, job_id: u64) -> Option<&JobState> {
        self.jobs.get(&job_id)
    }

    pub fn accept_event(&mut self, event: WorkerEvent) -> EventDisposition {
        let envelope = event.0;
        match omo_protocol::generation::classify_generation(
            envelope.worker_generation,
            self.generation,
        ) {
            omo_protocol::generation::GenerationDisposition::Current => {
                if envelope.kind == "heartbeat" {
                    self.last_heartbeat = Instant::now();
                }
                EventDisposition::Accepted
            }
            omo_protocol::generation::GenerationDisposition::Stale => {
                let mut result_files_deleted = ["result_ref", "plot_ref", "matches_ref"]
                    .into_iter()
                    .filter_map(|key| envelope.body.get(key)?.get("path")?.as_str())
                    .filter(|path| self.remove_result_if_owned(Path::new(path)))
                    .count();
                if let Some(parquet) = envelope.body.get("parquet_path").and_then(Value::as_str) {
                    if self.remove_result_if_owned(Path::new(parquet)) {
                        result_files_deleted += 1;
                    }
                }
                EventDisposition::DiscardedStale {
                    result_files_deleted,
                }
            }
        }
    }

    pub fn heartbeat_expired(&self, now: Instant, timeout: Duration) -> bool {
        now.checked_duration_since(self.last_heartbeat)
            .is_some_and(|elapsed| elapsed >= timeout)
    }

    pub fn worker_crashed(&mut self, at: Instant) {
        for state in self.jobs.values_mut() {
            if !matches!(
                state,
                JobState::Saved | JobState::Failed(_) | JobState::Cancelled(_)
            ) {
                *state = JobState::Failed(omo_protocol::state::FailKind::WorkerCrashed);
            }
        }
        self.generation = self.generation.saturating_add(1);
        self.last_heartbeat = at;
    }

    fn remove_result_if_owned(&self, path: &Path) -> bool {
        let Some(root) = &self.result_root else {
            return false;
        };
        let Ok(canonical_root) = std::fs::canonicalize(root) else {
            return false;
        };
        let Ok(canonical_path) = std::fs::canonicalize(path) else {
            return false;
        };
        let ext = canonical_path.extension().and_then(|ext| ext.to_str());
        if !canonical_path.starts_with(&canonical_root) || !matches!(ext, Some("arrow" | "parquet"))
        {
            return false;
        }
        std::fs::remove_file(canonical_path).is_ok()
    }
}

#[derive(Debug, Default)]
pub struct WorkerPoll {
    pub events: Vec<WorkerEvent>,
    pub stderr: Vec<u8>,
    pub exited: bool,
}

pub trait WorkerProcess: Send {
    fn send(&mut self, request: Envelope<Value>) -> Result<(), String>;
    fn poll(&mut self) -> Result<WorkerPoll, String>;
    fn kill(&mut self) -> Result<(), String>;
}

pub trait WorkerFactory: Send + 'static {
    fn spawn(&mut self, generation: u64) -> Result<Box<dyn WorkerProcess>, String>;
}

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error("supervisor thread failed: {0}")]
    Thread(#[from] std::io::Error),
    #[error("supervisor startup channel closed")]
    StartupClosed,
    #[error("worker startup failed: {0}")]
    WorkerStartup(String),
    #[error("supervisor command channel is closed")]
    CommandClosed,
    #[error("stderr ring buffer lock is poisoned")]
    StderrPoisoned,
}

enum SupervisorCommand {
    Request(Envelope<Value>),
    Cancel { job_id: u64, target_request_id: u64 },
    Shutdown,
}

struct ActiveCancel {
    target_request_id: u64,
    interrupt_request_id: u64,
    controller: crate::host::cancel::CancelController,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveRequest {
    job_id: u64,
    request_id: u64,
}

const STDERR_RING_CAPACITY: usize = 64 * 1024;
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct EngineSupervisor {
    commands: async_mpsc::UnboundedSender<SupervisorCommand>,
    events: Receiver<WorkerEvent>,
    generation: Arc<AtomicU64>,
    stderr_ring: Arc<Mutex<VecDeque<u8>>>,
    thread: Option<JoinHandle<()>>,
}

impl EngineSupervisor {
    pub fn start_child(
        worker_executable: impl Into<PathBuf>,
        bootstrap_config: Value,
        result_root: impl Into<PathBuf>,
    ) -> Result<Self, SupervisorError> {
        Self::start(
            crate::host::process::SubprocessWorkerFactory::new(worker_executable),
            bootstrap_config,
            result_root,
        )
    }

    pub fn start<F: WorkerFactory>(
        mut factory: F,
        bootstrap_config: Value,
        result_root: impl Into<PathBuf>,
    ) -> Result<Self, SupervisorError> {
        let (commands, command_rx) = async_mpsc::unbounded_channel();
        let (event_tx, events) = mpsc::channel();
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);
        let generation = Arc::new(AtomicU64::new(1));
        let stderr_ring = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_RING_CAPACITY)));
        let thread_generation = Arc::clone(&generation);
        let thread_stderr = Arc::clone(&stderr_ring);
        let result_root = result_root.into();

        let thread = thread::Builder::new()
            .name("omo-engine-supervisor".into())
            .spawn(move || {
                let runtime = match Builder::new_current_thread().enable_all().build() {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = startup_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                runtime.block_on(supervisor_loop(
                    &mut factory,
                    command_rx,
                    event_tx,
                    bootstrap_config,
                    result_root,
                    thread_generation,
                    thread_stderr,
                    startup_tx,
                ));
            })?;

        match startup_rx
            .recv()
            .map_err(|_| SupervisorError::StartupClosed)?
        {
            Ok(()) => Ok(Self {
                commands,
                events,
                generation,
                stderr_ring,
                thread: Some(thread),
            }),
            Err(message) => {
                let _ = thread.join();
                Err(SupervisorError::WorkerStartup(message))
            }
        }
    }

    pub fn send_request(&self, request: Envelope<Value>) -> Result<(), SupervisorError> {
        self.commands
            .send(SupervisorCommand::Request(request))
            .map_err(|_| SupervisorError::CommandClosed)
    }

    pub fn cancel(&self, job_id: u64, target_request_id: u64) -> Result<(), SupervisorError> {
        self.commands
            .send(SupervisorCommand::Cancel {
                job_id,
                target_request_id,
            })
            .map_err(|_| SupervisorError::CommandClosed)
    }

    pub fn try_recv(&self) -> Result<Option<WorkerEvent>, SupervisorError> {
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(SupervisorError::CommandClosed),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn stderr_snapshot(&self) -> Result<Vec<u8>, SupervisorError> {
        let ring = self
            .stderr_ring
            .lock()
            .map_err(|_| SupervisorError::StderrPoisoned)?;
        Ok(ring.iter().copied().collect())
    }

    pub fn shutdown(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = self.commands.send(SupervisorCommand::Shutdown);
            let _ = thread.join();
        }
    }
}

impl Drop for EngineSupervisor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[allow(clippy::too_many_arguments)]
async fn supervisor_loop<F: WorkerFactory>(
    factory: &mut F,
    mut commands: async_mpsc::UnboundedReceiver<SupervisorCommand>,
    events: mpsc::Sender<WorkerEvent>,
    bootstrap_config: Value,
    result_root: PathBuf,
    generation: Arc<AtomicU64>,
    stderr_ring: Arc<Mutex<VecDeque<u8>>>,
    startup: mpsc::SyncSender<Result<(), String>>,
) {
    let mut current_generation = 1u64;
    let now = Instant::now();
    let mut core = SupervisorCore::with_result_root(current_generation, now, result_root);
    let mut worker = match spawn_and_bootstrap(factory, current_generation, &bootstrap_config) {
        Ok(worker) => worker,
        Err(error) => {
            let _ = startup.send(Err(error));
            return;
        }
    };
    generation.store(current_generation, Ordering::Release);
    let _ = startup.send(Ok(()));

    let mut tick = time::interval(Duration::from_millis(50));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let mut next_request_id = 1u64;
    let mut cancellations: HashMap<u64, ActiveCancel> = HashMap::new();
    let mut active_requests: Vec<ActiveRequest> = Vec::new();

    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(SupervisorCommand::Request(request)) => {
                        if request.worker_generation == current_generation {
                            let tracked = is_tracked_request(&request.kind);
                            let entry = ActiveRequest {
                                job_id: request.job_id,
                                request_id: request.request_id,
                            };
                            if tracked {
                                core.start_job(request.job_id);
                                if !active_requests.contains(&entry) {
                                    active_requests.push(entry);
                                }
                            }
                            if let Err(error) = worker.send(request) {
                                append_stderr(&stderr_ring, error.as_bytes());
                                fail_active_and_restart(
                                    factory,
                                    &mut worker,
                                    &mut core,
                                    &mut current_generation,
                                    &generation,
                                    &bootstrap_config,
                                    &stderr_ring,
                                    &mut active_requests,
                                    &mut cancellations,
                                    &events,
                                    &[],
                                    FailKind::WorkerCrashed,
                                    &format!("worker stdin send failed: {error}"),
                                );
                            }
                        }
                    }
                    Some(SupervisorCommand::Cancel { job_id, target_request_id }) => {
                        let mut controller = crate::host::cancel::CancelController::new(Duration::from_secs(10));
                        let interrupt_id = next_request_id;
                        next_request_id = next_request_id.saturating_add(1);
                        let request = Envelope::new(
                            current_generation,
                            job_id,
                            interrupt_id,
                            "interrupt",
                            json!({"target_request_id":target_request_id}),
                        );
                        let action = controller.request(target_request_id, false, Instant::now());
                        if matches!(action, crate::host::cancel::CancelAction::SendInterrupt { .. }) {
                            match worker.send(request) {
                                Ok(()) => {
                                    let _ = controller.begin_drain(Instant::now());
                                    cancellations.insert(job_id, ActiveCancel {
                                        target_request_id,
                                        interrupt_request_id: interrupt_id,
                                        controller,
                                    });
                                }
                                Err(error) => {
                                    append_stderr(&stderr_ring, error.as_bytes());
                                    fail_active_and_restart(
                                        factory,
                                        &mut worker,
                                        &mut core,
                                        &mut current_generation,
                                        &generation,
                                        &bootstrap_config,
                                        &stderr_ring,
                                        &mut active_requests,
                                        &mut cancellations,
                                        &events,
                                        &[],
                                        FailKind::WorkerCrashed,
                                        &format!("worker interrupt send failed: {error}"),
                                    );
                                }
                            }
                        }
                    }
                    Some(SupervisorCommand::Shutdown) | None => {
                        let _ = worker.kill();
                        return;
                    }
                }
            }
            _ = tick.tick() => {
                match worker.poll() {
                    Ok(mut poll) => {
                        append_stderr(&stderr_ring, &poll.stderr);
                        for event in poll.events.drain(..) {
                            let disposition = core.accept_event(event.clone());
                            if disposition == EventDisposition::Accepted {
                                let fail_kind = (event.0.kind == "failed")
                                    .then(|| parse_fail_kind(&event.0.body))
                                    .flatten();
                                let cancel_ack = matches!(fail_kind, Some(FailKind::Cancelled(_)));
                                let matching_cancel = cancellations
                                    .get(&event.0.job_id)
                                    .is_some_and(|active| {
                                        event.0.request_id == active.target_request_id
                                            || event.0.request_id == active.interrupt_request_id
                                    });
                                if cancel_ack && matching_cancel {
                                    if let Some(active) = cancellations.remove(&event.0.job_id) {
                                        let _ = active.controller.phase_level();
                                        active_requests.retain(|req| {
                                            !(req.job_id == event.0.job_id
                                                && (req.request_id == active.target_request_id
                                                    || req.request_id == event.0.request_id))
                                        });
                                    }
                                    core.set_job_state(
                                        event.0.job_id,
                                        JobState::Cancelled(CancelLevel::Interrupt),
                                    );
                                } else if event.0.kind == "failed" {
                                    active_requests.retain(|req| {
                                        !(req.job_id == event.0.job_id
                                            && req.request_id == event.0.request_id)
                                    });
                                    let kind = fail_kind.unwrap_or_else(|| {
                                        FailKind::Query(
                                            event
                                                .0
                                                .body
                                                .get("message")
                                                .and_then(Value::as_str)
                                                .unwrap_or("worker request failed")
                                                .to_owned(),
                                        )
                                    });
                                    core.set_job_state(event.0.job_id, JobState::Failed(kind));
                                } else if matches!(
                                    event.0.kind.as_str(),
                                    "result" | "export_committed"
                                ) {
                                    active_requests.retain(|req| {
                                        !(req.job_id == event.0.job_id
                                            && req.request_id == event.0.request_id)
                                    });
                                    if matching_cancel {
                                        cancellations.remove(&event.0.job_id);
                                    }
                                    core.set_job_state(event.0.job_id, JobState::Saved);
                                }
                                let _ = events.send(event);
                            }
                        }
                        if poll.exited {
                            fail_active_and_restart(
                                factory,
                                &mut worker,
                                &mut core,
                                &mut current_generation,
                                &generation,
                                &bootstrap_config,
                                &stderr_ring,
                                &mut active_requests,
                                &mut cancellations,
                                &events,
                                &[],
                                FailKind::WorkerCrashed,
                                "worker exited unexpectedly",
                            );
                        }
                    }
                    Err(error) => {
                        append_stderr(&stderr_ring, error.as_bytes());
                        fail_active_and_restart(
                            factory,
                            &mut worker,
                            &mut core,
                            &mut current_generation,
                            &generation,
                            &bootstrap_config,
                            &stderr_ring,
                            &mut active_requests,
                            &mut cancellations,
                            &events,
                            &[],
                            FailKind::WorkerCrashed,
                            &format!("worker poll failed: {error}"),
                        );
                    }
                }

                let now = Instant::now();
                let expired_jobs: Vec<u64> = cancellations
                    .iter_mut()
                    .filter_map(|(job_id, active)| {
                        matches!(active.controller.poll(now), crate::host::cancel::CancelAction::KillWorker)
                            .then_some(*job_id)
                    })
                    .collect();
                if !expired_jobs.is_empty() {
                    fail_active_and_restart(
                        factory,
                        &mut worker,
                        &mut core,
                        &mut current_generation,
                        &generation,
                        &bootstrap_config,
                        &stderr_ring,
                        &mut active_requests,
                        &mut cancellations,
                        &events,
                        &expired_jobs,
                        FailKind::WorkerCrashed,
                        "worker killed after cancellation drain timeout",
                    );
                } else if core.heartbeat_expired(now, HEARTBEAT_TIMEOUT) {
                    append_stderr(&stderr_ring, b"worker heartbeat timeout; restarting generation\n");
                    fail_active_and_restart(
                        factory,
                        &mut worker,
                        &mut core,
                        &mut current_generation,
                        &generation,
                        &bootstrap_config,
                        &stderr_ring,
                        &mut active_requests,
                        &mut cancellations,
                        &events,
                        &[],
                        FailKind::Timeout,
                        "worker heartbeat timeout",
                    );
                }
            }
        }
    }
}

fn is_tracked_request(kind: &str) -> bool {
    matches!(
        kind,
        "query"
            | "export"
            | "series_plan"
            | "ensure_spatial"
            | "catalog_check"
            | "resolve_run"
            | "series"
            | "sql_session"
    )
}

#[allow(clippy::too_many_arguments)]
fn fail_active_and_restart<F: WorkerFactory>(
    factory: &mut F,
    worker: &mut Box<dyn WorkerProcess>,
    core: &mut SupervisorCore,
    generation: &mut u64,
    generation_shared: &AtomicU64,
    bootstrap_config: &Value,
    stderr_ring: &Arc<Mutex<VecDeque<u8>>>,
    active_requests: &mut Vec<ActiveRequest>,
    cancellations: &mut HashMap<u64, ActiveCancel>,
    events: &mpsc::Sender<WorkerEvent>,
    killed_cancel_jobs: &[u64],
    default_kind: FailKind,
    message: &str,
) {
    let failed_generation = *generation;
    let drained: Vec<ActiveRequest> = active_requests.drain(..).collect();
    let drained_jobs: Vec<u64> = drained.iter().map(|req| req.job_id).collect();
    for req in drained {
        let kind = if killed_cancel_jobs.contains(&req.job_id) {
            FailKind::Cancelled(CancelLevel::Kill)
        } else {
            default_kind.clone()
        };
        core.set_job_state(req.job_id, JobState::Failed(kind.clone()));
        let event = WorkerEvent(Envelope::new(
            failed_generation,
            req.job_id,
            req.request_id,
            "failed",
            failed_body(req.request_id, &kind, message),
        ));
        let _ = events.send(event);
    }
    for &job_id in killed_cancel_jobs {
        if !drained_jobs.contains(&job_id)
            && let Some(active) = cancellations.get(&job_id)
        {
            let kind = FailKind::Cancelled(CancelLevel::Kill);
            core.set_job_state(job_id, JobState::Failed(kind.clone()));
            let event = WorkerEvent(Envelope::new(
                failed_generation,
                job_id,
                active.target_request_id,
                "failed",
                failed_body(active.target_request_id, &kind, message),
            ));
            let _ = events.send(event);
        }
    }
    cancellations.clear();
    restart_worker(
        factory,
        worker,
        core,
        generation,
        generation_shared,
        bootstrap_config,
        stderr_ring,
    );
}

fn spawn_and_bootstrap<F: WorkerFactory>(
    factory: &mut F,
    generation: u64,
    bootstrap_config: &Value,
) -> Result<Box<dyn WorkerProcess>, String> {
    let mut worker = factory.spawn(generation)?;
    worker.send(Envelope::new(
        generation,
        0,
        0,
        "hello",
        json!({"bootstrap_config": bootstrap_config}),
    ))?;
    Ok(worker)
}

fn restart_worker<F: WorkerFactory>(
    factory: &mut F,
    worker: &mut Box<dyn WorkerProcess>,
    core: &mut SupervisorCore,
    generation: &mut u64,
    generation_shared: &AtomicU64,
    bootstrap_config: &Value,
    stderr_ring: &Arc<Mutex<VecDeque<u8>>>,
) {
    let _ = worker.kill();
    core.worker_crashed(Instant::now());
    *generation = core.generation();
    generation_shared.store(*generation, Ordering::Release);
    match spawn_and_bootstrap(factory, *generation, bootstrap_config) {
        Ok(next) => *worker = next,
        Err(error) => {
            append_stderr(
                stderr_ring,
                format!("worker restart failed: {error}\n").as_bytes(),
            );
        }
    }
}

fn append_stderr(ring: &Arc<Mutex<VecDeque<u8>>>, bytes: &[u8]) {
    if let Ok(mut ring) = ring.lock() {
        for byte in bytes {
            if ring.len() == STDERR_RING_CAPACITY {
                ring.pop_front();
            }
            ring.push_back(*byte);
        }
    }
}
