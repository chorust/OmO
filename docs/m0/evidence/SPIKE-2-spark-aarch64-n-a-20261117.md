# SPIKE-2 on spark-aarch64/n/a — 2026-11-17

- Host: lizhan-spark, Ubuntu 24.04, Linux aarch64; Rust 1.100.0; DuckDB v2.0.0; duckomo v0.4.0, GitHub-release-unsigned.
- Command: `omo-series-spike --plan A` and `omo-series-spike --plan B` on the same fixture, stations, and time window; exact invocations and artifacts are in [EXIT-2 detail](EXIT-2-spark-aarch64-20261117.md).
- Result: **pass for the station-series comparison on spark.** Both plans produced 27 rows, passed all 10 independent semantic oracle checks, and had no mismatches. Plan B was selected for eligible regular spherical grids; Plan A remains the full-grid fallback.
- Measurements: Plan A 49 ms, 131 reported physical read bytes, incomplete scan, no selective slicing. Plan B 120 ms, 27 candidates, complete scan, 624 reported physical read bytes.
- Notes: this is Linux arm64 evidence only; it does not establish results on the other target platforms.
- Output / artifacts: [full report](EXIT-2-spark-aarch64-20261117.md), [Plan A CSV](EXIT-2-spark-aarch64-20261117-plan-a.csv), [Plan B CSV](EXIT-2-spark-aarch64-20261117-plan-b.csv), [comparison JSON](EXIT-2-spark-aarch64-20261117-comparison.json).
