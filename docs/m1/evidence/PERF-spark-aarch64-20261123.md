# PERF — SC-002 (100 stations) — 2026-11-23

- **Host**: `lizhan-spark` (`aarch64-unknown-linux-gnu`), 2026-11-23 UTC, release `omo-worker`
- **Versions**: Rust `1.100.0`, DuckDB `v2.0.0`, duckomo `v0.4.0` (`github-release-unsigned`)
- **Command**: `python3 scripts/perf-series.py --runs 5` (fresh worker, data dir, and extension cache per run; no connection reuse)
- **Input**: `fixtures/stations/m1-100.csv` (100 stations), default window 0–168 h, latest complete run `2026-11-23T06:00:00Z` (06Z, truncated at 144 h), Plan G with 20 parallel point readers, `max_distance_m = 7200`
- **Network**: the host's normal internet path to `openmeteo.s3.amazonaws.com`; latency and bandwidth were not recorded separately.

## Result (wall time of the `series` request, from send to `result`)

| run | seconds |
|---|---|
| 1 | 77.99 |
| 2 | 86.55 |
| 3 | 84.43 |
| 4 | 73.26 |
| 5 | 74.54 |

**Median 77.99 s** (target < 120 s) with 10,900 rows (100 stations × 109 valid times). This is slower than the 59.0 s S1 spike figure because the request also resolves the run, writes the Arrow, matches, and Parquet files, and runs the object-version check.

## Not measured

- SC-001 (time to first interaction in the GUI) was not measured: it needs a GUI session that this run did not drive.
- A 00Z/12Z run (113 valid times) was not available; the 06Z run has fewer time steps, so a 00Z/12Z run may be somewhat slower.
