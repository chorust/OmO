//! Development harness utilities used by the M0 workbench and stress runs.

use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::{self, Write},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use arrow::{
    array::{ArrayRef, Int64Array, RecordBatch},
    datatypes::{DataType, Field, Schema},
};
use arrow_ipc::writer::FileWriter;
use clap::{Args, Parser, Subcommand, ValueEnum};
use omo_app::host::{
    cancel::{CancelAction, CancelController},
    supervisor::{EngineSupervisor, WorkerEvent, WorkerFactory, WorkerPoll, WorkerProcess},
};
use omo_protocol::{
    messages::{Envelope, failed_body, parse_fail_kind},
    state::{CancelLevel, FailKind},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug, Parser)]
#[command(name = "omo-harness", about = "M0 development and acceptance harness")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Write a synthetic Arrow IPC file for table or plot paging.
    GenResult(GenResultArgs),
    /// Emit a timed progress stream to stdout as newline-delimited JSON.
    Progress(ProgressArgs),
    /// Run worker crash recovery against a deterministic fake process.
    CrashInject(CrashInjectArgs),
    /// Exercise worker cancellation levels against host state and a fake process.
    Cancel(CancelArgs),
    /// Inject a process failure at one of the atomic export commit points.
    ExportKill(ExportKillArgs),
}

#[derive(Debug, Args)]
struct GenResultArgs {
    /// Number of table rows; incompatible with --series/--points.
    #[arg(long)]
    rows: Option<u64>,
    /// Number of table columns; incompatible with --series/--points.
    #[arg(long)]
    cols: Option<u32>,
    /// Number of numeric plot series; use with --points.
    #[arg(long, requires = "points", conflicts_with_all = ["rows", "cols"])]
    series: Option<u32>,
    /// Number of points per numeric series; use with --series.
    #[arg(long, requires = "series", conflicts_with_all = ["rows", "cols"])]
    points: Option<u64>,
    /// Output path. Defaults to a unique file in the system temporary directory.
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ProgressArgs {
    /// Progress events per second (1..=1000).
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=1000))]
    hz: u32,
    /// Duration in minutes (1..=1440).
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..=1440))]
    minutes: u32,
}

#[derive(Debug, Args)]
struct CrashInjectArgs {
    /// Number of crash and restart cycles (1..=1000).
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
    iterations: u32,
}

#[derive(Debug, Args)]
struct ExportKillArgs {
    /// Commit boundary after which to inject the failure.
    #[arg(long, value_enum)]
    at: ExportKillPoint,
    /// Keep the recovery files in this directory instead of a temporary directory.
    #[arg(long)]
    directory: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ExportKillPoint {
    Step1,
    Step2,
    Step3,
}

#[derive(Debug, Args)]
struct CancelArgs {
    /// Cancellation levels to exercise (comma-separated subset of 1,2,3).
    #[arg(long, default_value = "1,2,3")]
    levels: String,
}

#[derive(Serialize)]
struct ProgressEvent {
    kind: &'static str,
    sequence: u64,
    elapsed_ms: u128,
}

#[derive(Serialize)]
struct HarnessReport {
    status: &'static str,
    scenario: &'static str,
    iterations: u32,
    recoveries: u32,
    generation: u64,
    details: Vec<String>,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::GenResult(args) => gen_result(args),
        Command::Progress(args) => emit_progress(args),
        Command::CrashInject(args) => crash_inject(args),
        Command::Cancel(args) => cancel(args),
        Command::ExportKill(args) => export_kill(args),
    };
    if let Err(error) = result {
        eprintln!("omo-harness: {error:#}");
        std::process::exit(2);
    }
}

fn gen_result(args: GenResultArgs) -> anyhow::Result<()> {
    let (rows, columns) = match (args.series, args.points) {
        (Some(series), Some(points)) => (points, series),
        (None, None) => (args.rows.unwrap_or(1_000), args.cols.unwrap_or(4)),
        _ => anyhow::bail!("--series and --points must be supplied together"),
    };
    anyhow::ensure!(rows > 0, "--rows/--points must be greater than zero");
    anyhow::ensure!(columns > 0, "--cols/--series must be greater than zero");
    anyhow::ensure!(
        rows <= 10_000_000,
        "row count exceeds the 10,000,000 fixture limit"
    );
    anyhow::ensure!(columns <= 256, "column count exceeds the 256 fixture limit");

    let path = args.output.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("omo-harness-{}.arrow", uuid::Uuid::new_v4()))
    });
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let schema = Arc::new(Schema::new(
        (0..columns)
            .map(|column| Field::new(format!("series_{column:02}"), DataType::Int64, false))
            .collect::<Vec<_>>(),
    ));
    let mut writer = FileWriter::try_new(file, &schema)?;
    const BATCH_ROWS: u64 = 8_192;
    let mut start = 0u64;
    while start < rows {
        let batch_rows = (rows - start).min(BATCH_ROWS) as usize;
        let arrays: Vec<ArrayRef> = (0..columns)
            .map(|column| {
                let offset = i64::from(column);
                let values = (0..batch_rows)
                    .map(|row| (start + row as u64) as i64 + offset)
                    .collect::<Vec<_>>();
                Arc::new(Int64Array::from(values)) as ArrayRef
            })
            .collect();
        let batch = RecordBatch::try_new(Arc::clone(&schema), arrays)?;
        writer.write(&batch)?;
        start += batch_rows as u64;
    }
    writer.finish()?;
    println!("{}", path.display());
    Ok(())
}

fn emit_progress(args: ProgressArgs) -> anyhow::Result<()> {
    let period = Duration::from_nanos(1_000_000_000 / u64::from(args.hz));
    let duration = Duration::from_secs(u64::from(args.minutes) * 60);
    let started = Instant::now();
    let mut sequence = 0u64;
    let stdout = io::stdout();
    let mut output = stdout.lock();

    while started.elapsed() < duration {
        let event = ProgressEvent {
            kind: "progress",
            sequence,
            elapsed_ms: started.elapsed().as_millis(),
        };
        serde_json::to_writer(&mut output, &event)?;
        output.write_all(b"\n")?;
        output.flush()?;
        sequence = sequence.saturating_add(1);
        let next_tick = period.saturating_mul(u32::try_from(sequence).unwrap_or(u32::MAX));
        if let Some(wait) = next_tick.checked_sub(started.elapsed()) {
            thread::sleep(wait);
        }
    }
    Ok(())
}

fn crash_inject(args: CrashInjectArgs) -> anyhow::Result<()> {
    let root = temporary_result_root()?;
    let mut supervisor = EngineSupervisor::start(
        FakeWorkerFactory::new(FakeMode::CrashOnQuery),
        json!({"mode":"deterministic-fake"}),
        &root,
    )?;
    let mut details = Vec::with_capacity(args.iterations as usize);

    for iteration in 0..args.iterations {
        let generation = supervisor.generation();
        supervisor.send_request(Envelope::new(
            generation,
            1,
            u64::from(iteration) + 1,
            "query",
            json!({"inject_crash":true}),
        ))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while supervisor.generation() == generation && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let next_generation = supervisor.generation();
        anyhow::ensure!(
            next_generation > generation,
            "fake worker did not restart after crash iteration {}",
            iteration + 1
        );
        details.push(format!("generation {generation} -> {next_generation}"));
    }

    let report = HarnessReport {
        status: "passed",
        scenario: "fake_worker_crash_recovery",
        iterations: args.iterations,
        recoveries: details.len() as u32,
        generation: supervisor.generation(),
        details,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    supervisor.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

fn cancel(args: CancelArgs) -> anyhow::Result<()> {
    let levels = parse_cancel_levels(&args.levels)?;
    let root = temporary_result_root()?;
    let mut supervisor = EngineSupervisor::start(
        FakeWorkerFactory::new(FakeMode::AcknowledgeInterrupt),
        json!({"mode":"deterministic-fake"}),
        &root,
    )?;
    let generation = supervisor.generation();
    let job_id = 7u64;
    let request_id = 11u64;
    supervisor.send_request(Envelope::new(
        generation,
        job_id,
        request_id,
        "query",
        json!({"hold_open":true}),
    ))?;

    let mut details = Vec::with_capacity(levels.len());
    if levels.contains(&1) {
        supervisor.cancel(job_id, request_id)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut acknowledged = false;
        while Instant::now() < deadline {
            if let Some(event) = supervisor.try_recv()?
                && event.0.kind == "failed"
                && matches!(
                    parse_fail_kind(&event.0.body),
                    Some(FailKind::Cancelled(CancelLevel::Interrupt))
                )
            {
                acknowledged = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        anyhow::ensure!(
            acknowledged,
            "fake worker did not acknowledge interrupt cancellation"
        );
        details.push("level 1: fake worker acknowledged interrupt".to_owned());
    }
    for level in levels.iter().copied().filter(|level| *level != 1) {
        let now = Instant::now();
        let mut controller = CancelController::new(Duration::from_secs(10));
        anyhow::ensure!(
            matches!(
                controller.request(request_id, false, now),
                CancelAction::SendInterrupt { .. }
            ),
            "cancel controller did not request interrupt"
        );
        let deadline = match controller.begin_drain(now) {
            CancelAction::DrainUntil(deadline) => deadline,
            action => anyhow::bail!("unexpected cancel drain action: {action:?}"),
        };
        let outcome = if level == 2 {
            controller.completed(CancelLevel::Drain { timeout_ms: 1_000 });
            controller
                .phase_level()
                .expect("completed drain has a level")
        } else {
            anyhow::ensure!(
                matches!(controller.poll(deadline), CancelAction::KillWorker),
                "cancel controller did not escalate an expired drain to kill"
            );
            controller
                .phase_level()
                .expect("killed controller has a level")
        };
        details.push(format!(
            "level {level}: cancel state-machine outcome {outcome:?}"
        ));
    }

    let report = HarnessReport {
        status: "passed",
        scenario: "fake_worker_cancel_and_host_state_machine",
        iterations: levels.len() as u32,
        recoveries: 0,
        generation: supervisor.generation(),
        details,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    supervisor.shutdown();
    std::fs::remove_dir_all(root)?;
    Ok(())
}

fn export_kill(args: ExportKillArgs) -> anyhow::Result<()> {
    #[cfg(feature = "in-process-engine")]
    {
        export_kill_enabled(args)
    }
    #[cfg(not(feature = "in-process-engine"))]
    {
        let _ = args;
        anyhow::bail!("export-kill requires omo-app feature `in-process-engine`")
    }
}

#[cfg(feature = "in-process-engine")]
fn export_kill_enabled(args: ExportKillArgs) -> anyhow::Result<()> {
    use omo_app::host::export_scan::{ExportVerdict, is_saved, scan_export_dir};
    use omo_engine::export::{
        ExportStep, SIDECAR_SCHEMA, STAGING_OWNER_MARKER, commit_staged, sidecar_path,
    };
    use sha2::{Digest, Sha256};

    let (root, temporary) = match args.directory {
        Some(path) => {
            std::fs::create_dir_all(&path)?;
            (path, false)
        }
        None => {
            let path =
                std::env::temp_dir().join(format!("omo-export-kill-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path)?;
            (path, true)
        }
    };
    let target = root.join("report.csv");
    let old_data = b"old data";
    std::fs::write(&target, old_data)?;
    let old_hash = format!("{:x}", Sha256::digest(old_data));
    std::fs::write(
        sidecar_path(&target),
        serde_json::to_vec(&json!({
            "schema": SIDECAR_SCHEMA, "data":{"sha256":old_hash}
        }))?,
    )?;
    let suffix = uuid::Uuid::new_v4();
    let staging = root.join(format!(".report.csv.omo-tmp-{suffix}"));
    std::fs::create_dir(&staging)?;
    std::fs::write(staging.join(".omo-owner"), STAGING_OWNER_MARKER)?;
    let new_data = b"new data";
    std::fs::write(staging.join("data"), new_data)?;
    let new_hash = format!("{:x}", Sha256::digest(new_data));
    std::fs::write(
        staging.join("sidecar"),
        serde_json::to_vec(&json!({
            "schema":"omo-export-sidecar/1", "data":{"sha256":new_hash}
        }))?,
    )?;

    let injected_at = match args.at {
        ExportKillPoint::Step1 => ExportStep::OldSidecarMoved,
        ExportKillPoint::Step2 => ExportStep::DataMoved,
        ExportKillPoint::Step3 => ExportStep::SidecarMoved,
    };
    let commit = commit_staged(&target, &staging, true, |step| {
        if step == injected_at {
            Err(io::Error::other("injected process termination"))
        } else {
            Ok(())
        }
    });
    anyhow::ensure!(
        commit.is_err(),
        "fault injection did not interrupt the commit"
    );
    let entries = scan_export_dir(&root)?;
    anyhow::ensure!(
        entries.len() == 1,
        "recovery did not report exactly one export"
    );
    let expected = if matches!(args.at, ExportKillPoint::Step3) {
        ExportVerdict::Finished
    } else {
        ExportVerdict::Incomplete
    };
    anyhow::ensure!(
        entries[0].verdict == expected,
        "unexpected recovery verdict: {:?}",
        entries[0].verdict
    );
    let saved = is_saved(&target)?;
    anyhow::ensure!(
        saved == matches!(expected, ExportVerdict::Finished),
        "saved predicate disagrees with recovery verdict"
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status":"passed", "scenario":"export_kill_injection", "at":format!("{:?}", args.at).to_lowercase(),
            "recovery":format!("{:?}", expected).to_lowercase(), "target":target,
            "saved":saved, "staging_preserved":staging.exists()
        }))?
    );
    if temporary {
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

fn parse_cancel_levels(raw: &str) -> anyhow::Result<Vec<u8>> {
    let mut levels = Vec::new();
    for part in raw.split(',') {
        let level: u8 = part.trim().parse()?;
        anyhow::ensure!((1..=3).contains(&level), "cancel levels must be 1, 2, or 3");
        if !levels.contains(&level) {
            levels.push(level);
        }
    }
    anyhow::ensure!(!levels.is_empty(), "at least one cancel level is required");
    Ok(levels)
}

fn temporary_result_root() -> anyhow::Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("omo-harness-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

#[derive(Clone, Copy)]
enum FakeMode {
    CrashOnQuery,
    AcknowledgeInterrupt,
}

struct FakeWorkerFactory {
    mode: FakeMode,
}

impl FakeWorkerFactory {
    fn new(mode: FakeMode) -> Self {
        Self { mode }
    }
}

impl WorkerFactory for FakeWorkerFactory {
    fn spawn(&mut self, generation: u64) -> Result<Box<dyn WorkerProcess>, String> {
        Ok(Box::new(FakeWorker {
            generation,
            mode: self.mode,
            events: VecDeque::new(),
            crash_pending: false,
            last_heartbeat: Instant::now(),
        }))
    }
}

struct FakeWorker {
    generation: u64,
    mode: FakeMode,
    events: VecDeque<WorkerEvent>,
    crash_pending: bool,
    last_heartbeat: Instant,
}

impl WorkerProcess for FakeWorker {
    fn send(&mut self, request: Envelope<Value>) -> Result<(), String> {
        match request.kind.as_str() {
            "query" if matches!(self.mode, FakeMode::CrashOnQuery) => {
                self.crash_pending = true;
            }
            "interrupt" if matches!(self.mode, FakeMode::AcknowledgeInterrupt) => {
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
                        "fake interrupt acknowledged",
                    ),
                )));
            }
            _ => {}
        }
        Ok(())
    }

    fn poll(&mut self) -> Result<WorkerPoll, String> {
        if self.crash_pending {
            self.crash_pending = false;
            return Ok(WorkerPoll {
                exited: true,
                ..WorkerPoll::default()
            });
        }
        let mut events = self.events.drain(..).collect::<Vec<_>>();
        if self.last_heartbeat.elapsed() >= Duration::from_secs(1) {
            self.last_heartbeat = Instant::now();
            events.push(WorkerEvent(Envelope::new(
                self.generation,
                0,
                0,
                "heartbeat",
                json!({"t_ms":0}),
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
