# GTK main-thread probe validation — spark-aarch64/X11 — 2026-11-17

- Host: lizhan-spark, Ubuntu 24.04, Linux aarch64; isolated Xvfb display `:104`.
- Purpose: validate the optional GLib timeout probe on GTK's native main context. This is instrumentation validation only, not an N2 or N7 acceptance workload.
- Build: Rust `1.100.0`; `PKG_CONFIG_PATH=/tmp/gtkdev/pc RUSTUP_TOOLCHAIN=1.100.0 cargo build --locked --release -p omo-app --bin omo-app --features main-thread-probe-test`.
- Run configuration: `OMOBENCH_BOOTSTRAP_CONFIG=/tmp/omo-ui-run/bootstrap.json`, `OMOBENCH_MAIN_THREAD_LOG=/tmp/omo-ui-loop-probe/main-controlled3.jsonl`, and `OMOBENCH_MAIN_THREAD_TEST_STALL_MS=250`. The test-only feature injects a single 250 ms sleep into the GLib main-context callback; File → Quit closed the app cleanly and flushed the terminal snapshot.
- Validation command: `python3 scripts/summarize-main-thread-probe.py docs/m0/evidence/main-thread-probe-validation-spark-aarch64-x11-20261117/controlled.jsonl --require-stall-ms 200` — pass.
- Result: 1,784 callbacks over 45.425 seconds; callback ran on `ThreadId(1)`, the thread that installed the GTK main loop; maximum callback gap 252.260 ms; maximum overrun beyond the nominal 25 ms interval 227.260 ms. The controlled delay was detected by the main-context probe. The terminal record reports `stopped`; its ongoing-gap field covers the tail after the last callback.
- Instrumented app SHA-256: `ebbe28b266cfe0ac76be2c819adc0048e5c5e0b79125396192bc867b389fdc94`.
- Raw JSONL SHA-256: `264fe3667944c9c882a3ae0ceb53dbdc59672f626f3d7b83f8f36fba2ab7c9be`.
- App log SHA-256: `cbfadad01f2fb2b13af4ab1924e1ff5de8305ba669de384b9ab12953b87a0e0d`.
- Raw artifacts: [controlled JSONL](main-thread-probe-validation-spark-aarch64-x11-20261117/controlled.jsonl) and [app log](main-thread-probe-validation-spark-aarch64-x11-20261117/app.log).

The GLib timeout uses default priority 0. The callback gap includes GTK main-context work, the nominal 25 ms timeout interval, and scheduler delay; main-context priority starvation can also affect the result. It confirms a main-context delay is observable, but does not isolate CPU-only blocking or input-to-render latency. The acceptance workloads use a normal release build without the delay-injection feature.
