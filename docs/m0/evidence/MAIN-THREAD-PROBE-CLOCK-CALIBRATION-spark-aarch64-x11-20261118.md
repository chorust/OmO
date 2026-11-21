# Main-thread probe clock calibration — spark-aarch64/X11 — 2026-11-18

- Host: lizhan-spark, Ubuntu 24.04, Linux aarch64; isolated Xvfb `:105`.
- Build: Rust `1.100.0`; `PKG_CONFIG_PATH=/tmp/gtkdev/pc RUSTUP_TOOLCHAIN=1.100.0 cargo build -p omo-app --release --locked --features main-thread-probe-test` passed.
- Instrumented app SHA-256: `7977738da5710600f824210996faefc9469d2c21c2bab94a5fab6f53ada1b72f`.
- Run: `OMOBENCH_MAIN_THREAD_TEST_STALL_MS=250`, `OMOBENCH_MAIN_THREAD_LOG=/tmp/omo-ui-loop-probe/main-clock-v2.jsonl`, `NSH_FPS=1`; File → Quit closed the app and flushed the terminal sample.
- Validation: `python3 scripts/summarize-main-thread-probe.py docs/m0/evidence/main-thread-probe-validation-spark-aarch64-x11-20261118/controlled-clock-v2.jsonl --require-stall-ms 200` passed.

The version-2 capture recorded both Rust monotonic and wall-clock callback gaps. It contained 1,289 callbacks over 32.852 seconds by each clock, with a maximum gap of 250.175 ms from both clocks and no sampled wall-clock regression. The controlled 250 ms main-context delay was detected consistently by both measurements.

Artifacts: [JSONL](main-thread-probe-validation-spark-aarch64-x11-20261118/controlled-clock-v2.jsonl), SHA-256 `41800ad96d8ee3fe164061b4cee377973b510f4521b0954d326d319120a8f4aa`; [app log](main-thread-probe-validation-spark-aarch64-x11-20261118/app.log), SHA-256 `48cc9d47ac171e7bf9f476d20499aef0a5dd8c96675b158d6228219af28bda70`.

The callback records clock readings and atomic counters only; a background writer handles JSONL I/O. This validates the added wall-clock path under a controlled delay. Long-run measurements still report both elapsed clocks and their rate ratio, since system clock behavior can vary during a longer capture.
