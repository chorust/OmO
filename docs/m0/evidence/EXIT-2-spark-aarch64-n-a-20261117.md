# EXIT-2 on spark-aarch64/n/a — 2026-11-17

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel 6.17.0-1029-nvidia.
- Versions: Rust 1.100.0; DuckDB v2.0.0; duckomo v0.4.0, GitHub-release-unsigned.
- Command: `omo-series-spike --plan A` and `omo-series-spike --plan B` using the same committed fixture, station list, time window, and independent oracle.
- Result: **pass.** Both plans returned 27 rows and passed all 10 semantic oracle checks with no mismatches. Plan B was selected for eligible regular spherical grids; Plan A remains a full-grid fallback.
- Measurements: Plan A 49 ms / 131 reported physical read bytes / incomplete scan / slice feasibility `no`. Plan B 120 ms / 27 candidates / 624 reported physical read bytes / complete scan.
- Output / artifacts: [detailed EXIT-2 report](EXIT-2-spark-aarch64-20261117.md), [comparison JSON](EXIT-2-spark-aarch64-20261117-comparison.json), [Plan A CSV](EXIT-2-spark-aarch64-20261117-plan-a.csv), [Plan B CSV](EXIT-2-spark-aarch64-20261117-plan-b.csv), [plan decision](../decisions/series-plan.md).
