# N7 threading rerun on spark-aarch64/X11 — 2026-11-18

- Host: lizhan-spark, Ubuntu 24.04, Linux aarch64, X11 (`DISPLAY=:104`, isolated Xvfb).
- Toolchain and runtime: Rust `1.100.0`; NSH `0.5.1`; DuckDB `v2.0.0`; duckomo `v0.4.0`, GitHub-release-unsigned, `linux_arm64`.
- App SHA-256: `4a3ccab70609f29c6200f1e0b651b44baec55b44d6ae79e31e25193caef80751` (`target/release/omo-app`). Worker SHA-256: `f98819131ae5f984dde4aef0b5f2febb4427b3ae03f0a4e4a472942a32385e1b` (`target/release/omo-worker`).
- Launch workload: `OMOBENCH_PROGRESS_HZ=50`, `OMOBENCH_INITIAL_SQL='SELECT sum(hash(a * 2000000 + b)) FROM range(2000000) t1(a) CROSS JOIN range(2000000) t2(b)'`, `NSH_FPS=1`, and `OMOBENCH_MAIN_THREAD_LOG` set to the raw JSONL path. The query was submitted with the UI Run preview action.

## Duration and UI interaction

The app was started before the explicit start-time marker. The marker at `01:09:01+08:00` was written after the initial active-query screenshot showed progress sequence 773. At `01:19:28+08:00`, the ten-minute screenshot showed sequence 32,090. The displayed sequence advanced by 31,317 over at least 627 seconds between those markers, equivalent to about 49.95 displayed increments/s at the configured 50 Hz. As with the earlier N7 run, this is a difference between displayed sequence values, not an independent count of progress events delivered to the UI.

While progress continued, clicking the Results tab changed the selected view; see [interaction](n7-main-thread-spark-aarch64-x11-20261118/interaction.png). At the ten-minute point the UI showed the active preview and sequence 32,090; see [ten-minute capture](n7-main-thread-spark-aarch64-x11-20261118/ten-minute.png).

After the ten-minute interval, worker PID `3665959` was killed with `SIGKILL`. The app remained open, the worker restarted as generation 2, and progress resumed at sequence 3037; see [restart capture](n7-main-thread-spark-aarch64-x11-20261118/restarted.png). A subsequent click returned to the SQL editor while generation 2 continued; see [post-restart interaction](n7-main-thread-spark-aarch64-x11-20261118/post-restart-interaction.png). File → Quit closed the UI and flushed the terminal probe sample.

## Main-thread callback gaps

The opt-in GLib timeout probe ran on GTK's `ThreadId(1)` at default source priority 0 and sampled every 25 ms. Its JSONL has 30,761 callback samples over 784.234 seconds according to Rust's monotonic clock, a maximum callback interval of **61.976 ms**, and a maximum interval over the nominal 25 ms of **36.976 ms**. The largest sampled ongoing gap was 28.015 ms; the terminal record reports `stopped`. Validate with:

```text
python3 scripts/summarize-main-thread-probe.py docs/m0/evidence/n7-main-thread-spark-aarch64-x11-20261118/main-thread.jsonl --min-coverage-seconds 600
```

The host UTC markers span 627 seconds while the probe's monotonic elapsed field spans 784.234 seconds. These cover different intervals: the probe began before the first marker and continued after the second through restart and shutdown. Their difference alone does not establish a clock discrepancy. The wall-clock markers support the ten-minute interval; the displayed progress-counter delta corroborates activity but is not an independent duration measurement. The callback interval includes main-context work and scheduling delay and may be affected by GLib source starvation; it is an observed main-thread service interval, not CPU-only blocking or input-to-render latency. The probe is calibrated separately in [main-thread probe validation](MAIN-THREAD-PROBE-spark-aarch64-x11-20261117.md).

The 782-line [NSH performance log](n7-main-thread-spark-aarch64-x11-20261118/app.log) contains 771 `nsh::fps` samples. Across its rolling samples, the frame p50 field ranged 4.55–8.36 ms, p95 ranged 6.10–18.99 ms, frame-work maximum ranged 7.06–35.21 ms, and app RSS ranged 232.1–252.0 MB. These are rolling values, not input-to-response timings.

## Artifacts and scope

- Probe JSONL SHA-256: `00f17ab215f3754cfce4f623b6ecc7f5e4e4a5a42aba1f8157e92a7c5ad7ab77`.
- App log SHA-256: `b6ff163702268926d30f2d54ca7f5686b506312aab9d3d6ea383eb97311db4d8`.
- [Main-thread JSONL](n7-main-thread-spark-aarch64-x11-20261118/main-thread.jsonl), [app log](n7-main-thread-spark-aarch64-x11-20261118/app.log), [start capture](n7-main-thread-spark-aarch64-x11-20261118/start.png), [ready state](n7-main-thread-spark-aarch64-x11-20261118/ready.png), [ten-minute state](n7-main-thread-spark-aarch64-x11-20261118/ten-minute.png), [restart state](n7-main-thread-spark-aarch64-x11-20261118/restarted.png), and [post-restart interaction](n7-main-thread-spark-aarch64-x11-20261118/post-restart-interaction.png).
- Start and end marker files are [start-time.txt](n7-main-thread-spark-aarch64-x11-20261118/start-time.txt) and [end-time.txt](n7-main-thread-spark-aarch64-x11-20261118/end-time.txt).

Result: the ten-minute progress exercise, UI interaction, worker restart, and bounded main-thread callback-gap measurement pass for this spark X11 run. Linux x86_64, macOS, and the full spark Wayland N7 cell remain not-run.
