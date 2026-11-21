# EXIT-6 export recovery on spark-aarch64/n-a — 2026-11-17

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`; terminal session.
- Versions: Rust `1.100.0`; current workspace source.
- Commands: `RUSTUP_TOOLCHAIN=1.100.0 target/debug/omo-harness export-kill --at step1`; repeat with `step2` and `step3`.
- Result: **pass on spark.**
- Measurements: step 1 → incomplete / saved=false / staging preserved; step 2 → incomplete / saved=false / staging preserved; step 3 → finished / saved=true / staging removed.
- Output / artifacts: [step 1 JSON](EXIT-6-spark-aarch64-n-a-20261117-step1.json), [step 2 JSON](EXIT-6-spark-aarch64-n-a-20261117-step2.json), [step 3 JSON](EXIT-6-spark-aarch64-n-a-20261117-step3.json); test suites and EXDEV coverage are summarized in [prior EXIT-6 evidence](EXIT-6-spark-aarch64-n-a-20261116.md).
- Notes: GTK/GStreamer system package installation is not established by this headless harness run. Linux x86_64 and macOS arm64 are not-run for lack of hosts.
