# Cross-platform GitHub CI and release evidence — 2026-11-27

- **Date**: 2026-11-27 UTC
- **Hosts / Targets**: GitHub Actions runners (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, and build-only `x86_64-apple-darwin`)
- **Locked versions**: `rustc 1.100.0`, `DuckDB v2.0.0` (`duckdb =1.20000.1`), `duckomo v0.4.0`, `NSH =0.5.1`
- **Commands exercised in CI**:
  - `cargo +1.100.0 build --workspace --locked`
  - `cargo +1.100.0 test --workspace --locked`
  - `scripts/check-boundaries.sh && scripts/check-boundaries.sh --self-test`
  - `python3 scripts/ci-verify-duckomo.py --repeat 3` (locked Duckomo platforms)
  - `python3 scripts/ci-verify-series.py` (locked Duckomo platforms; M1 regression)
  - `python3 scripts/ci-verify-map.py` (locked Duckomo platforms; M2 `SC-009`)

This record closes the M2 cross-platform gate (`V8` / `SC-009`) and documents closure of
`003 — M2 Regional Tile Map & Layer Stack` as `verified` under Constitution 1.3.0.

## Workflow runs

| run | workflow | trigger | head SHA | result |
|---|---|---|---|---|
| [45928536196](https://github.com/chorust/OmO/actions/runs/45928536196) | CI (`.github/workflows/ci.yml`) | push `main`, 2026-11-27T02:28:33Z | `f9ac341db62019fbc40c9c8fddf907ea218adebe` | success |
| [45933007001](https://github.com/chorust/OmO/actions/runs/45933007001) | CI (`.github/workflows/ci.yml`) | push `v0.1.0-m2`, 2026-11-27T03:14:35Z | `f9ac341db62019fbc40c9c8fddf907ea218adebe` | success |
| [45933008722](https://github.com/chorust/OmO/actions/runs/45933008722) | Release assets (`.github/workflows/release.yml`) | release `v0.1.0-m2`, 2026-11-27T03:14:36Z | `f9ac341db62019fbc40c9c8fddf907ea218adebe` | success |

## CI jobs (run 45928536196; replicated on tag run 45933007001)

| job id | runner / image | target | Duckomo platform | workspace tests | Duckomo bootstrap + live worker | M1 series smoke (`ci-verify-series.py`) | M2 map smoke (`ci-verify-map.py`) |
|---|---|---|---|---|---|---|---|
| 140275541972 | `ubuntu-24.04` (`20261122.23.1`) | `x86_64-unknown-linux-gnu` | `linux_amd64` | 263 passed, 0 failed, 4 ignored | passed | passed (`rows: 327`, `plan: "G"`) | passed (`valid_count: 33431`, `tiles_filled: 4`, `png_bytes: 188232`) |
| 140275541740 | `ubuntu-24.04-arm` (`20261116.188.1`) | `aarch64-unknown-linux-gnu` | `linux_arm64` | 263 passed, 0 failed, 4 ignored | passed | passed (`rows: 327`, `plan: "G"`) | passed (`valid_count: 33431`, `tiles_filled: 4`, `png_bytes: 188232`) |
| 140275542098 | `macos-15` (`20261123.0558.1`) | `aarch64-apple-darwin` | `osx_arm64` | 262 passed, 0 failed, 4 ignored | passed | passed (`rows: 327`, `plan: "G"`) | passed (`valid_count: 33431`, `tiles_filled: 4`, `png_bytes: 188232`) |
| 140275541957 | `macos-15-intel` (`20260824.0482.1`) | `x86_64-apple-darwin` | — (build-only) | 262 passed, 0 failed, 4 ignored | not applicable | not applicable | not applicable |

*(Note: Linux runs 263 tests vs. 262 on macOS due to the `#[cfg(all(test, target_os = "linux"))]` `/dev/shm` cross-filesystem `EXDEV` fallback test in `crates/omo-engine/src/export.rs`.)*

Recorded `scripts/ci-verify-map.py` stdout on the three locked platforms (live network run in both
`45928536196` and tag run `45933007001` jobs `140289709503`, `140289709663`, `140289709288`; never
`skipped(network)`):

```json
{"scenario": "ci_verify_map", "platform": "linux_amd64", "run": "2026-11-26T18:00:00Z", "layout": "data_spatial", "valid_count": 33431, "tiles_filled": 4, "inspect": {"value": 2.299999952316284, "unit": "\u00b0C"}, "status": "passed", "png_bytes": 188232}
{"scenario": "ci_verify_map", "platform": "linux_arm64", "run": "2026-11-26T18:00:00Z", "layout": "data_spatial", "valid_count": 33431, "tiles_filled": 4, "inspect": {"value": 2.299999952316284, "unit": "\u00b0C"}, "status": "passed", "png_bytes": 188232}
{"scenario": "ci_verify_map", "platform": "osx_arm64",   "run": "2026-11-26T18:00:00Z", "layout": "data_spatial", "valid_count": 33431, "tiles_filled": 4, "inspect": {"value": 2.299999952316284, "unit": "\u00b0C"}, "status": "passed", "png_bytes": 188232}
```

## Release jobs and artifacts (run 45933008722, `v0.1.0-m2`)

| job id | job | artifact | bytes |
|---|---|---|---|
| 140289714896 | Build linux-x86_64 | `omobench-v0.1.0-m2-linux-x86_64.tar.gz` | 42103211 |
| 140289714559 | Build linux-aarch64 | `omobench-v0.1.0-m2-linux-aarch64.tar.gz` | 40502850 |
| 140289715038 | Build macos-aarch64 | `omobench-v0.1.0-m2-macos-aarch64.tar.gz` | 35465437 |
| 140289714919 | Build macos-x86_64 | `omobench-v0.1.0-m2-macos-x86_64.tar.gz` | 38388169 |

## Recorded scope notes preserved from Spark acceptance

All prior scope boundaries recorded in [acceptance.md](../acceptance.md) remain explicit:

- **S0 (`NSH =0.7.0` gate)**: `=0.7.0` failed to map the `1440×960` main window under Spark X11/Xvfb (creating only the `1×1` `com.omobench.studio` window); `=0.5.1` was retained per `S0` protocol (`S0-NSH070-spark-aarch64-20261126.md`).
- **S1 / SC-001 (cold `data_spatial` latency)**: Measured median `24.07 s` (`19.58–38.45 s`, 5 cold trials) accepted under the user-relaxed `< 30 s` median budget (2026-11-26), with single-trial S3 variance noted (`S1-spark-aarch64-20261126.md`).
- **V2 / SC-004 (`data_spatial` vs `data_run` parity)**: Bit-identical parity (`max |Δ| == 0.0`) verified on a `447`-point `2°×2°` patch plus live single-404 fallback; full-extent `data_run` scan omitted due to per-point HTTP request cost (`S1-spark-aarch64-20261126.md`).
- **Pre-review GUI capture timing**: `V1`–`V6` screenshots and `V5-gui-map-exported.png.omo.json` were captured on Spark prior to the post-review contract/UI alignment pass; the aligned `/3` sidecar schema and composite PNG export are verified across workspace tests and the live `ci-verify-map.py` runs above.
