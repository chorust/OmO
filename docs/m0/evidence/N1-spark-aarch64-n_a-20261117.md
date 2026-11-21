# N1 on spark-aarch64/n/a — 2026-11-17

- Host / OS / kernel / desktop: lizhan-spark, Ubuntu 24.04, aarch64, kernel `6.17.0-1029-nvidia`; build is session-independent.
- Versions: Rust `1.100.0 (0ef1bc386 2026-11-09)`; `duckdb` 1.20000.1; Arrow and Arrow IPC 60.0.0; NSH 0.5.1. Extracted build metadata reports GTK4 4.14.5, Pango/PangoCairo 1.52.1, and GStreamer 1.24.2.
- Commands: `PKG_CONFIG_PATH=/tmp/gtkdev/pc RUSTUP_TOOLCHAIN=1.100.0 cargo build --workspace --locked` and `PKG_CONFIG_PATH=/tmp/gtkdev/pc RUSTUP_TOOLCHAIN=1.100.0 cargo test --workspace --locked` — passed; final rerun logs are linked below.
- Result: **pass for the locked workspace build and tests using the extracted-package workaround.** The system package installation part of T019 remains unverified: `libgtk-4-dev`, `libgstreamer1.0-dev`, and `libgstreamer-plugins-base1.0-dev` are not installed system-wide; `sudo -n` reports that a password is required. No package installation is claimed.
- Measurements: the locked workspace build compiled the debug `omo-app` target and all workspace crates; workspace tests completed successfully with the same `PKG_CONFIG_PATH`. The release `omo-app` binary used for N2/N3/N7/N8 is identified separately in [N7 evidence](N7-spark-aarch64-x11-20261117.md).
- Output / artifacts: [build log](N1-spark-aarch64-n_a-20261117-build.log), [test log](N1-spark-aarch64-n_a-20261117-test.log), [Linux prerequisites and workaround](../linux-prereqs.md).
- Notes / deviations: `/tmp/gtkdev` contains extracted Ubuntu arm64 `.deb` files under `/tmp/gtkdev/root`; their pkg-config files were copied to `/tmp/gtkdev/pc` and rewritten to point into that extraction root. This workaround is separate from system package installation. System-installed prerequisites observed: `build-essential 12.10ubuntu1`, `pkg-config 1.8.1-2build1`, and `libpango1.0-dev 1.52.1+ds-1build1`.

## Current-tree rerun — 2026-11-18

After removing unused manifest dependencies, the locked workspace build and tests were rerun against the current source tree with Rust `1.100.0` and the same `/tmp/gtkdev` pkg-config workaround:

- `PKG_CONFIG_PATH=/tmp/gtkdev/pc RUSTUP_TOOLCHAIN=1.100.0 cargo build --workspace --locked` — passed; [build log](N1-spark-aarch64-n_a-20261118-build.txt).
- `PKG_CONFIG_PATH=/tmp/gtkdev/pc RUSTUP_TOOLCHAIN=1.100.0 cargo test --workspace --locked` — 42 tests passed, 0 failed; [test log](N1-spark-aarch64-n_a-20261118-test.txt).
- `PKG_CONFIG_PATH=/tmp/gtkdev/pc RUSTUP_TOOLCHAIN=1.100.0 cargo build --release --workspace --locked` — passed; [release build log](N1-spark-aarch64-n_a-20261118-release.txt).

The rerun emitted no unused-dependency warnings. This updates build/test evidence only; the system GTK4/GStreamer development packages remain absent and T019 is still open.
