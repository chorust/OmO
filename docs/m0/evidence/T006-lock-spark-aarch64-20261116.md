# T006 workspace lock and build — spark-aarch64 — 2026-11-16

- Host / OS / kernel / desktop: `lizhan-spark`, Ubuntu 24.04.4 LTS, aarch64, kernel `6.17.0-1029-nvidia`, X11.
- Versions: installed rustc/cargo 1.96.0; `duckdb` 1.20000.1; `arrow` 60.0.0; `arrow-ipc` 60.0.0; `nsh-*` 0.5.1. `Cargo.lock` resolves 506 packages.
- Commands:
  - `RUSTUP_TOOLCHAIN=1.96.0 cargo generate-lockfile` — passed; lock regenerated against the current workspace manifests and registry metadata.
  - `RUSTUP_TOOLCHAIN=1.96.0 cargo build --workspace --locked` — stopped before compilation because Rust 1.96.0 is below the workspace and NSH 0.5.1 minimum of 1.97.
- Result: lock generation passed; the required locked workspace build did not pass. T006 remains incomplete pending the approved toolchain install/build gate.
- Measurements: resolved direct pins are DuckDB 1.20000.1, Arrow 60.0.0, Arrow IPC 60.0.0, and NSH 0.5.1.
- Output / artifacts: root `Cargo.lock`.
- Notes / deviations: at the time of this record no host packages were installed, and the pinned Rust 1.100.0 toolchain had been removed. GTK4/GStreamer development packages remained unverified. The failure below was superseded by the 2026-11-17 follow-up.

## Follow-up — 2026-11-17

- Installed and selected Rust `1.100.0 (0ef1bc386 2026-11-09)`.
- `PKG_CONFIG_PATH=/tmp/gtkdev/pc RUSTUP_TOOLCHAIN=1.100.0 cargo build --workspace --locked` passed; complete output: [build log](N1-spark-aarch64-n_a-20261117-build.log).
- `PKG_CONFIG_PATH=/tmp/gtkdev/pc RUSTUP_TOOLCHAIN=1.100.0 cargo test --workspace --locked` passed; complete output: [test log](N1-spark-aarch64-n_a-20261117-test.log).
- Exact direct pins remain DuckDB `1.20000.1`, Arrow / Arrow IPC `60.0.0`, and NSH `0.5.1`.
- This completes T006's lock generation and locked build requirement using the documented `/tmp/gtkdev` extraction workaround. It does not claim system-wide GTK4/GStreamer package installation; that prerequisite is separately tracked under T019.
