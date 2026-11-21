# EXIT-4 worker recovery and cancellation on spark-aarch64/n-a — 2026-11-17

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Versions: Rust `1.100.0`; `omo-app` / `omo-worker` workspace sources.
- Commands:
  - `RUSTUP_TOOLCHAIN=1.100.0 cargo run --locked -p omo-app --no-default-features --features in-process-engine --bin omo-harness -- crash-inject --iterations 20`
  - `RUSTUP_TOOLCHAIN=1.100.0 target/debug/omo-harness cancel --levels 1,2,3`
  - `RUSTUP_TOOLCHAIN=1.100.0 cargo test --locked -p omo-app --no-default-features --features in-process-engine --test supervisor`
- Result: pass for the spark host. The crash harness recovered 20/20 injected fake-worker crashes and advanced generation from 1 to 21. The supervisor suite passed 5/5 tests, including stale-generation frame rejection with deletion of the referenced owned result file, crash failure/generation advance, heartbeat timeout, and cancellation escalation. The live Linux arm64 worker kill/restart and live query cancellation are separately demonstrated by [N7](N7-spark-aarch64-x11-20261117.md) and [T034](T034-worker-live-spark-aarch64-20261116.md).
- Cancellation scope: level 1 was acknowledged by the fake worker; levels 2 and 3 exercise the host state machine (drain completion and kill escalation), not real worker timing. The live UI query cancellation in N7 ended at the worker's interrupt path; a separate live level-3 cancellation timing run is not claimed.
- Measurements: crash harness 20 recoveries, generation 1 → 21; stale results accepted: 0 in the supervisor test; test suite 5 passed, 0 failed.
- Output / artifacts: [crash JSON](EXIT-4-spark-aarch64-n-a-20261117-crash.json), [cancel JSON](EXIT-4-spark-aarch64-n-a-20261117-cancel.json), [N7 live worker evidence](N7-spark-aarch64-x11-20261117.md).
- Notes / deviations: only spark Linux arm64 was available. Linux x86_64 and macOS arm64 runs are not available in this session and are recorded separately as not-run.
