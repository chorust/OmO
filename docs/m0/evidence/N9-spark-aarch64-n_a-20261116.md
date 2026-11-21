# N9 dependencies on spark-aarch64/n/a — 2026-11-16

- Host / OS / kernel / desktop: `lizhan-spark`, Ubuntu 24.04.4 LTS, aarch64, kernel `6.17.0-1029-nvidia`, X11. Cargo tree target: `aarch64-unknown-linux-gnu`.
- Versions: rustc 1.96.0 used for metadata commands; `nsh-* =0.5.1`; DuckDB Rust binding `1.20000.1`; `arrow` and `arrow-ipc` `60.0.0` (Arrow subcrates resolve within major 60).
- Commands: for each package, `RUSTUP_TOOLCHAIN=1.96.0 cargo tree -p <package> -e <normal[,build]> --target aarch64-unknown-linux-gnu --locked`.
- Result: pass for the N9 dependency boundary inspection. `omo-app` has no `duckdb`, `libduckdb-sys`, GPUI, Tauri, QuickJS, or `nsh-js`; each core crate has no NSH, GPUI, or Tauri dependency. This is tree evidence only; it does not establish a successful build.
- Measurements: 949 package occurrences in the `omo-app` tree; 266–306 occurrences across core trees.
- Output / artifacts: [omo-app tree](N9-tree-linux-aarch64-omo-app.txt), [omo-engine tree](N9-tree-linux-aarch64-omo-engine.txt), [omo-protocol tree](N9-tree-linux-aarch64-omo-protocol.txt), [omo-worker tree](N9-tree-linux-aarch64-omo-worker.txt), [omo-project tree](N9-tree-linux-aarch64-omo-project.txt).
- Notes / deviations: this is the confirmed spark host. The 2026-11-16 dependency tree used Rust 1.96.0 and predates the later toolchain/build work. Its note that N1 was blocked is superseded by the current-tree Rust 1.100.0 build/test/release run documented in [N1 follow-up](N1-spark-aarch64-n_a-20261117.md); the system GTK/GStreamer package installation remains unverified and T019 remains open.

## Current-tree N9 refresh — 2026-11-19

After the manifest cleanup and current-tree build, all five locked target dependency trees were regenerated with Rust `1.100.0` using `cargo tree -p <crate> -e normal,build --target aarch64-unknown-linux-gnu --locked`. They show no DuckDB or forbidden UI runtime in `omo-app`, and no NSH/GPUI/Tauri dependency in a core crate. This is dependency metadata, not a Linux aarch64 host build.

- [omo-app](N9-tree-aarch64_unknown_linux_gnu-omo-app-20261119.txt)
- [omo-engine](N9-tree-aarch64_unknown_linux_gnu-omo-engine-20261119.txt)
- [omo-protocol](N9-tree-aarch64_unknown_linux_gnu-omo-protocol-20261119.txt)
- [omo-worker](N9-tree-aarch64_unknown_linux_gnu-omo-worker-20261119.txt)
- [omo-project](N9-tree-aarch64_unknown_linux_gnu-omo-project-20261119.txt)
