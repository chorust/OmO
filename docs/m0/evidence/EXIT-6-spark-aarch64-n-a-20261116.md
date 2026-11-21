# EXIT-6 export recovery on spark-aarch64/n-a — 2026-11-16

- Host / OS / kernel / desktop: spark-aarch64, Ubuntu 24.04.4, Linux 6.17.0-1029-nvidia; terminal session, no desktop.
- Versions: Rust 1.100.0; NSH core 0.5.1 in the lockfile; export fault injection exercised the shared engine commit code without loading DuckDB.
- Commands:
  - `RUSTUP_TOOLCHAIN=1.100.0 cargo test --locked -p omo-engine --features test-support --lib`
  - `RUSTUP_TOOLCHAIN=1.100.0 cargo test --locked -p omo-engine --features test-support --test export`
  - `RUSTUP_TOOLCHAIN=1.100.0 cargo test --locked -p omo-app --no-default-features --features in-process-engine --test export_scan`
  - `target/debug/omo-harness export-kill --at step1`
  - `target/debug/omo-harness export-kill --at step2`
  - `target/debug/omo-harness export-kill --at step3`
- Result: pass on this host. The kernel EXDEV mapping test passed. The engine export suite passed 3/3 tests; the app recovery suite passed 3/3 tests. Fault injection returned `incomplete`, `incomplete`, and `finished` for steps 1, 2, and 3 respectively. The saved predicate was false for steps 1 and 2 and true after step 3; recovery preserved incomplete staging and removed completed leftovers.
- GTK4 and GStreamer development packages are not installed (`pkg-config` could not find `gtk4` or `gstreamer-1.0`), so this result does not establish GTK UI behavior.
- Output: T034's live worker/export transaction is separately recorded in [T034 live evidence](T034-worker-live-spark-aarch64-20261116.md).
- Scope: this evidence covers only spark Linux arm64. Linux x86_64 and macOS arm64 acceptance remain unrun.
