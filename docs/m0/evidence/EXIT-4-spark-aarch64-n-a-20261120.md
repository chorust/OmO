# EXIT-4 harness follow-up on spark-aarch64/n-a — 2026-11-20

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Toolchain: Rust `1.100.0`; current workspace source; harness built with `--locked -p omo-app --no-default-features --features in-process-engine --bin omo-harness` after the system GTK/GStreamer install recorded in [N1](N1-spark-aarch64-n_a-20261119.md).
- Commands: `omo-harness crash-inject --iterations 20` and `omo-harness cancel --levels 1,2,3`.
- Result: crash injection passed 20/20 fake-worker recoveries, advancing generation from 1 to 21. Level 1 cancellation was acknowledged by the fake worker; levels 2 and 3 exercised the host drain and kill state machine.
- Stale result scope: 0 stale results accepted is covered by the current supervisor integration test in the workspace test log linked from N1; it is not reported as a metric emitted by `crash-inject` itself. Live worker restart and interrupt are recorded separately in [N7 v2](N7-v2-spark-aarch64-x11-20261118.md) and [T034](T034-worker-live-spark-aarch64-20261116.md).
- Outputs: [crash JSON](EXIT-4-spark-aarch64-n-a-20261120-crash.json), [cancel JSON](EXIT-4-spark-aarch64-n-a-20261120-cancel.json).
- Scope: only Spark arm64 was available. Linux x86_64 and macOS arm64 remain not-run. This follow-up does not close T040.
