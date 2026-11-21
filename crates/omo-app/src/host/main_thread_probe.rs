//! Opt-in measurements from the native GTK main context on Linux.
//!
//! A GLib timeout records monotonic and wall-clock callback gaps using clock
//! reads and atomic updates. A background thread writes one JSON Lines snapshot
//! per second, keeping file I/O off the GTK main thread. A callback gap includes
//! the callback's work, the nominal 25 ms interval, and scheduler delay; it is
//! not CPU-only time or input-to-render latency.

use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle, ThreadId},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

const LOG_ENV: &str = "OMOBENCH_MAIN_THREAD_LOG";
pub const LOOP_PERIOD: Duration = Duration::from_millis(25);
const LOOP_PERIOD_US: u64 = 25_000;
const SNAPSHOT_PERIOD: Duration = Duration::from_secs(1);

#[derive(Default)]
struct State {
    samples: AtomicU64,
    callback_elapsed_us: AtomicU64,
    callback_wall_elapsed_us: AtomicU64,
    last_callback_gap_us: AtomicU64,
    max_callback_gap_us: AtomicU64,
    last_callback_wall_gap_us: AtomicU64,
    max_callback_wall_gap_us: AtomicU64,
    callback_on_main_thread: AtomicBool,
}

/// Samples callback gaps on the thread that installed the GTK main loop probe.
pub struct MainThreadProbe {
    started: Instant,
    wall_started: SystemTime,
    main_thread: ThreadId,
    state: Arc<State>,
    stop: Arc<AtomicBool>,
    writer: Option<JoinHandle<io::Result<()>>>,
}

impl MainThreadProbe {
    /// Start the probe when `OMOBENCH_MAIN_THREAD_LOG` names a JSONL output path.
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os(LOG_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)?;
        let started = Instant::now();
        let wall_started = SystemTime::now();
        let main_thread = thread::current().id();
        let state = Arc::new(State::default());
        let stop = Arc::new(AtomicBool::new(false));
        let writer_state = Arc::clone(&state);
        let writer_stop = Arc::clone(&stop);
        let main_thread_label = format!("{main_thread:?}");
        let writer = match thread::Builder::new()
            .name(String::from("omo-main-thread-probe"))
            .spawn(move || {
                write_snapshots(
                    path,
                    started,
                    wall_started,
                    main_thread_label,
                    writer_state,
                    writer_stop,
                )
            }) {
            Ok(writer) => writer,
            Err(error) => {
                eprintln!("could not start main-thread probe writer: {error}");
                return None;
            }
        };

        Some(Self {
            started,
            wall_started,
            main_thread,
            state,
            stop,
            writer: Some(writer),
        })
    }

    /// Record one callback. This performs no I/O and does not wait for the writer.
    pub fn record_callback(
        &self,
        now: Instant,
        wall_elapsed: Duration,
        gap: Option<Duration>,
        wall_gap: Option<Duration>,
    ) {
        if let Some(gap) = gap {
            let gap_us = duration_us(gap);
            self.state
                .last_callback_gap_us
                .store(gap_us, Ordering::Relaxed);
            update_max(&self.state.max_callback_gap_us, gap_us);
        }
        if let Some(gap) = wall_gap {
            let gap_us = duration_us(gap);
            self.state
                .last_callback_wall_gap_us
                .store(gap_us, Ordering::Relaxed);
            update_max(&self.state.max_callback_wall_gap_us, gap_us);
        }
        self.state.callback_on_main_thread.store(
            thread::current().id() == self.main_thread,
            Ordering::Relaxed,
        );
        self.state.callback_elapsed_us.store(
            duration_us(now.saturating_duration_since(self.started)),
            Ordering::Relaxed,
        );
        self.state
            .callback_wall_elapsed_us
            .store(duration_us(wall_elapsed), Ordering::Relaxed);
        self.state.samples.fetch_add(1, Ordering::Release);
    }

    /// Wall-clock timestamp captured when the probe started.
    pub fn wall_started(&self) -> SystemTime {
        self.wall_started
    }

    /// Stop and join the background writer after GTK's event loop has returned.
    pub fn finish(&mut self) {
        self.stop_writer();
        if let Some(writer) = self.writer.take() {
            match writer.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("main-thread probe writer failed: {error}"),
                Err(_) => eprintln!("main-thread probe writer panicked"),
            }
        }
    }

    pub fn stop_writer(&self) {
        self.stop.store(true, Ordering::Release);
        if let Some(writer) = self.writer.as_ref() {
            writer.thread().unpark();
        }
    }
}

impl Drop for MainThreadProbe {
    fn drop(&mut self) {
        self.stop_writer();
        // The logger is detached; the GTK main thread never waits for file I/O.
        drop(self.writer.take());
    }
}

fn write_snapshots(
    path: PathBuf,
    started: Instant,
    wall_started: SystemTime,
    main_thread: String,
    state: Arc<State>,
    stop: Arc<AtomicBool>,
) -> io::Result<()> {
    let file = File::create(&path)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
    let mut output = BufWriter::new(file);
    let wall_start_ms = wall_started
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let header = json!({
        "type": "header",
        "schema": "omo-main-thread-probe/2",
        "expected_interval_us": LOOP_PERIOD_US,
        "glib_source_priority": "default (0)",
        "wall_start_unix_ms": wall_start_ms,
        "main_thread_id": main_thread,
        "process_id": std::process::id(),
    });
    write_json_line(&mut output, &header)?;

    let writer_started = Instant::now();
    loop {
        thread::park_timeout(SNAPSHOT_PERIOD);
        let stopping = stop.load(Ordering::Acquire);
        let snapshot = snapshot_value(&state, started, wall_started, writer_started, stopping);
        write_json_line(&mut output, &snapshot)?;
        if stopping {
            return Ok(());
        }
    }
}

fn snapshot_value(
    state: &State,
    started: Instant,
    wall_started: SystemTime,
    writer_started: Instant,
    stopping: bool,
) -> Value {
    let samples = state.samples.load(Ordering::Acquire);
    let max_callback_gap_us = state.max_callback_gap_us.load(Ordering::Relaxed);
    let callback_elapsed_us = state.callback_elapsed_us.load(Ordering::Relaxed);
    let callback_wall_elapsed_us = state.callback_wall_elapsed_us.load(Ordering::Relaxed);
    let run_elapsed_us = duration_us(started.elapsed());
    let wall_now = SystemTime::now();
    let wall_elapsed_us = duration_us(wall_now.duration_since(wall_started).unwrap_or_default());
    let ongoing_gap_us = run_elapsed_us.saturating_sub(callback_elapsed_us);
    let ongoing_wall_gap_us = wall_elapsed_us.saturating_sub(callback_wall_elapsed_us);
    let snapshot_wall_unix_ms = wall_now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    json!({
        "type": "snapshot",
        "writer_elapsed_us": duration_us(writer_started.elapsed()),
        "snapshot_wall_unix_ms": snapshot_wall_unix_ms,
        "callback_elapsed_us": callback_elapsed_us,
        "callback_wall_elapsed_us": callback_wall_elapsed_us,
        "run_elapsed_us": run_elapsed_us,
        "wall_elapsed_us": wall_elapsed_us,
        "ongoing_gap_us": ongoing_gap_us,
        "ongoing_wall_gap_us": ongoing_wall_gap_us,
        "samples": samples,
        "last_callback_gap_us": state.last_callback_gap_us.load(Ordering::Relaxed),
        "max_callback_gap_us": max_callback_gap_us,
        "max_callback_overrun_us": max_callback_gap_us.saturating_sub(LOOP_PERIOD_US),
        "last_callback_wall_gap_us": state.last_callback_wall_gap_us.load(Ordering::Relaxed),
        "max_callback_wall_gap_us": state.max_callback_wall_gap_us.load(Ordering::Relaxed),
        "max_callback_wall_overrun_us": state
            .max_callback_wall_gap_us
            .load(Ordering::Relaxed)
            .saturating_sub(LOOP_PERIOD_US),
        "callback_on_main_thread": state.callback_on_main_thread.load(Ordering::Relaxed),
        "termination": if stopping { "stopped" } else { "running" },
    })
}

fn write_json_line(output: &mut BufWriter<File>, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *output, value).map_err(io::Error::other)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn duration_us(duration: Duration) -> u64 {
    duration.as_micros().min(u64::MAX as u128) as u64
}

fn update_max(target: &AtomicU64, value: u64) {
    let mut current = target.load(Ordering::Relaxed);
    while value > current {
        match target.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::update_max;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn maximum_only_moves_upward() {
        let maximum = AtomicU64::new(0);
        update_max(&maximum, 125);
        update_max(&maximum, 80);
        update_max(&maximum, 240);
        assert_eq!(maximum.load(Ordering::Relaxed), 240);
    }
}
