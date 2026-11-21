# SPIKE-5 on macos14-arm64/n/a — 2026-11-17

- Host / OS / kernel: dependency tree was resolved on lizhan-spark; no macOS 14 arm64 host was available.
- Target: `aarch64-apple-darwin`.
- Command: locked target-specific `cargo tree` outputs for `omo-app` and the four core crates, plus the boundary-policy inspection.
- Result: **pass for the macOS arm64 target dependency graph only.** `omo-app` has no DuckDB or forbidden UI runtime, core crates have no NSH/GPUI/Tauri dependency, and the workspace resolves one Arrow major.
- Measurements: tree evidence is linked from [N9 macOS arm64](N9-macos14-arm64-n_a-20261116.md), with dated artifacts `N9-tree-aarch64_apple_darwin-*-20261117.txt`.
- Notes: this is target metadata evidence, not a macOS build or runtime test.
