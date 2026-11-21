# EXIT-2: station-series A/B spike

**Result:** PASS for Plan A and Plan B on Linux aarch64, using the locked GitHub-release-unsigned Duckomo artifact. Both plans emitted the same 27-row `station_series` result and matched the independent oracle on all listed semantic fields.

## Environment and inputs

- Date: 2026-11-17 UTC.
- Host: `lizhan-spark`, Linux `aarch64`, kernel `6.17.0-1029-nvidia`.
- Rust: `rustc 1.100.0 (0ef1bc386 2026-11-09)`.
- DuckDB: `v2.0.0`; Duckomo: `v0.4.0`; channel: `github-release-unsigned`.
- Locked compressed artifact: 8,783,732 bytes, SHA-256 `8fc579a83eccd25729c7f345e70295e3b466d81a2aaeb34d3d2a59d277cb6c2e`. Loaded extension SHA-256: `2189a68bebc26f2eb7fa7b7b0f1fb2e5db9bd4cea543d5941ec165f05046a83e`.
- R4 Plan B bootstrap: spatial preinstall `ok`, post-lock `ensure_spatial` `ok`; locked golden query also passed.
- Source OM bytes: 488, SHA-256 `264fe0905d0681d0fedd08d60a43bf7974cbd161319df77e919f08b4cb582e36`.
- Official OM C API value CSV: 1,121 bytes, SHA-256 `98b8349ae035ef9fc14e4e7f37caf5daa2d44f7de2cad5fe3201534cf4be1b32`.
- Station CSV SHA-256: `508f99b6bc8ce97672df8e85c8b2f5e0788a4b65e429e3e17bb8ac476578e62c`.
- Generated station reference SHA-256: `984137dba38e07fbb874380e5f8bed3cfa6e4dfb194c65be3dc9dac895d75ff0`.
- The two source files came from tracked Duckomo v0.4.0 files at commit `bcdb8c0739c55217abd7d342f47004164644ec0d`. Their selected source paths were clean when acquired. Duckling was not used for this spike; its pre-existing modified and untracked files were left untouched.

## Commands

```sh
python3 scripts/generate-series-reference.py
RUSTUP_TOOLCHAIN=1.100.0 cargo test --locked -p omo-engine --features test-support
RUSTUP_TOOLCHAIN=1.100.0 cargo run --locked -p omo-worker --bin omo-series-spike -- --plan A --artifact /tmp/omobench-duckomo-artifacts/duckomo-v0.4.0-duckdb-v2.0.0-linux_arm64.duckdb_extension.gz --output docs/m0/evidence/EXIT-2-spark-aarch64-20261117-plan-a.csv
RUSTUP_TOOLCHAIN=1.100.0 cargo run --locked -p omo-worker --bin omo-series-spike -- --plan B --artifact /tmp/omobench-duckomo-artifacts/duckomo-v0.4.0-duckdb-v2.0.0-linux_arm64.duckdb_extension.gz --output docs/m0/evidence/EXIT-2-spark-aarch64-20261117-plan-b.csv
python3 scripts/compare-series-plans.py
```

The schema test suite passed all six checks, including stable column order, key-based de-duplication, conflicting duplicate rejection, longitude normalization, UTC validation, and null-value acceptance.

## Semantic comparison

The independent oracle contains nine stations and three hourly times. Both output CSVs have 27 rows and 27 unique `(station_id, variable, valid_time)` keys. The Plan A and Plan B output files are byte-identical, SHA-256 `e0daeebb7b99809964020142fcd5c2c0a853a6a110dfba383cad410fb9e7f60a`.

| Check | Result |
|---|---|
| Station, variable, UTC time key | 27/27 match |
| Value, including null positions | 27/27 match; 19 null rows in the station oracle |
| Nearest grid latitude/longitude | 27/27 match; tolerance `1e-10` degrees |
| Spherical distance | 27/27 match; tolerance `1e-4` m |
| Nearest point index and row-major tie break | 27/27 match; tie station selects index 5 |
| Longitude inputs `462` and `-258` | Both normalize to `102`; their sampled series match |
| Out-of-domain latitude/longitude | Six rows retained with null sample fields |
| Grid CRS | Plans agree: regular spherical lon/lat, axis order longitude/latitude, radius 6,371,229 m |
| Source identity/version | One source object for all in-domain rows; the version is null; outside-domain rows have no source identity |

Plan B builds a 0.0001-degree square around each station and uses Duckomo's `point_on_surface_v1` representative. The resulting source point and distance matched the independent center, off-grid, tie, wrapping, null, and out-of-domain cases.

## Cost observations

These are single runs on a 5×5×3×2 synthetic fixture; they are diagnostic, not a production benchmark.

| Plan | Query time | Value data bytes | Metadata bytes | Index bytes | Physical read bytes | Scan complete | Source candidates |
|---|---:|---:|---:|---:|---:|---|---:|
| A | 49 ms | 0 reported | 131 | 0 | 131 | no | 150 |
| B | 120 ms | 206 | 393 | 25 | 624 | yes | 27 |

Plan A selected five unique grid points out of 25, but the locked `read_om_series` API has no selected-point argument. Its SQL coordinate join is downstream of the table-function scan; Duckomo reported fallback selection and 150 candidates. Its metrics also report `scan_complete=false`; the zero value-byte count is therefore not a complete data-read measurement and is not compared as though it were.

Plan B's metrics were complete for this run. Its 27 candidates are nine regions × three times; 21 source rows were requested for the seven in-domain station IDs. The reported 624 physical bytes include 206 value bytes, 393 metadata bytes, and 25 index bytes. The timings and byte totals should not be extrapolated to larger grids.

## Evidence files

- Per-plan CSVs: [Plan A](EXIT-2-spark-aarch64-20261117-plan-a.csv), [Plan B](EXIT-2-spark-aarch64-20261117-plan-b.csv).
- Full bootstrap, query, and raw metrics reports: [Plan A report](EXIT-2-spark-aarch64-20261117-plan-a.report.json), [Plan B report](EXIT-2-spark-aarch64-20261117-plan-b.report.json).
- Field-by-field comparison: [comparison JSON](EXIT-2-spark-aarch64-20261117-comparison.json).
- Fixture and reference provenance: [PROVENANCE.md](../../../fixtures/PROVENANCE.md), [station-series manifest](../../../fixtures/reference/station_series_manifest.json).
