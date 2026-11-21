# SPIKE-5 on spark-aarch64/n/a — 2026-11-17

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel 6.17.0-1029-nvidia.
- Versions: Rust 1.100.0 for the 2026-11-17 target-tree capture; DuckDB Rust binding 1.20000.1; Arrow/Arrow IPC 60.0.0; NSH 0.5.1.
- Command: `scripts/check-boundaries.sh`; `scripts/check-boundaries.sh --self-test`; `cargo tree --locked` for `omo-app` and each core crate, including target-specific trees.
- Result: **pass for dependency coexistence and the boundary self-test.** `omo-app` excludes DuckDB and forbidden UI runtimes; core crates exclude UI frameworks; the workspace resolves one Arrow major. All 10 injected forbidden dependency cases were detected.
- Measurements: target tree outputs are linked in [N9](N9-spark-aarch64-n_a-20261116.md) and [the aarch64 target app tree](N9-tree-aarch64_unknown_linux_gnu-omo-app-20261117.txt).
- Notes: target dependency-tree resolution is not a cross-platform build or runtime check. Earlier cargo-tree inventory is summarized in [SPIKE-5 background](SPIKE-5-2026-11-16.md).
