# Cross-platform GitHub CI and release evidence — 2026-11-25

- **Date**: 2026-11-25 UTC (evaluating GitHub Actions runs executed 2026-11-24 UTC)
- **Hosts / Targets**: GitHub Actions runners (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, and build-only `x86_64-apple-darwin`)
- **Locked versions**: `rustc 1.100.0`, `DuckDB v2.0.0` (`duckdb =1.20000.1`), `duckomo v0.4.0`, `NSH 0.5.1`
- **Commands exercised in CI**:
  - `cargo +1.100.0 build --workspace --locked`
  - `cargo +1.100.0 test --workspace --locked`
  - `scripts/check-boundaries.sh && scripts/check-boundaries.sh --self-test`
  - `python3 scripts/ci-verify-duckomo.py --repeat 3` (locked Duckomo platforms)
  - `python3 scripts/ci-verify-series.py` (locked Duckomo platforms; SC-009)

This record closes the M1 cross-platform gate (`V9` / `SC-009`) and documents the user's
2026-11-25 acceptance decision closing `002 — M1 Stations × Forecast Series` as `verified` under
Constitution 1.3.0.

## Workflow runs

| run | workflow | trigger | head SHA | result |
|---|---|---|---|---|
| [45348466177](https://github.com/chorust/OmO/actions/runs/45348466177) | CI (`.github/workflows/ci.yml`) | push `main`, 2026-11-24T07:30:07Z | `3affd776a4bab0324c43d6871ccabe90fddd5472` | success |
| [45348599457](https://github.com/chorust/OmO/actions/runs/45348599457) | CI (`.github/workflows/ci.yml`) | push `v0.1.0-m1`, 2026-11-24T07:31:05Z | `3affd776a4bab0324c43d6871ccabe90fddd5472` | success |
| [45349560323](https://github.com/chorust/OmO/actions/runs/45349560323) | Release assets (`.github/workflows/release.yml`) | release `v0.1.0-m1`, 2026-11-24T07:38:29Z | `3affd776a4bab0324c43d6871ccabe90fddd5472` | success |

## CI jobs (run 45348466177)

| job id | runner / image | target | Duckomo platform | workspace tests | Duckomo bootstrap + live worker | M1 series smoke (`ci-verify-series.py`) |
|---|---|---|---|---|---|---|
| 138288726371 | `ubuntu-24.04` (`20261116.133.1`) | `x86_64-unknown-linux-gnu` | `linux_amd64` | 97 passed, 0 failed, 1 ignored | passed | passed (`rows: 339`, `plan: "G"`) |
| 138288726250 | `ubuntu-24.04-arm` (`20261116.188.1`) | `aarch64-unknown-linux-gnu` | `linux_arm64` | 97 passed, 0 failed, 1 ignored | passed | passed (`rows: 339`, `plan: "G"`) |
| 138288726029 | `macos-15` (`20261118.0550.1`) | `aarch64-apple-darwin` | `osx_arm64` | 96 passed, 0 failed, 1 ignored | passed | passed (`rows: 339`, `plan: "G"`) |
| 138288726289 | `macos-15-intel` (`20260824.0482.1`) | `x86_64-apple-darwin` | — (build-only) | 96 passed, 0 failed, 1 ignored | not applicable | not applicable |

Recorded `scripts/ci-verify-series.py` stdout on the three locked platforms (live network run,
never `skipped(network)`):

```json
{"scenario": "ci_verify_series", "platform": "linux_amd64", "readiness": "ready", "status": "passed", "run": "2026-11-24T00:00:00Z", "cycle": "00", "rows": 339, "plan": "G"}
{"scenario": "ci_verify_series", "platform": "linux_arm64", "readiness": "ready", "status": "passed", "run": "2026-11-24T00:00:00Z", "cycle": "00", "rows": 339, "plan": "G"}
{"scenario": "ci_verify_series", "platform": "osx_arm64",   "readiness": "ready", "status": "passed", "run": "2026-11-24T00:00:00Z", "cycle": "00", "rows": 339, "plan": "G"}
```

## Release jobs and artifacts (run 45349560323, `v0.1.0-m1`)

| job id | job | artifact | bytes |
|---|---|---|---|
| 138292298586 | Build linux-x86_64 | `omobench-v0.1.0-m1-linux-x86_64.tar.gz` | 39708060 |
| 138292298151 | Build linux-aarch64 | `omobench-v0.1.0-m1-linux-aarch64.tar.gz` | 38170703 |
| 138292298585 | Build macos-aarch64 | `omobench-v0.1.0-m1-macos-aarch64.tar.gz` | 33183375 |
| 138292298560 | Build macos-x86_64 | `omobench-v0.1.0-m1-macos-x86_64.tar.gz` | 36024051 |

## Accepted scope boundaries (user decision, 2026-11-25)

On 2026-11-25 the user accepted M1 (`002`) as `verified` on the existing Spark + GitHub CI/release
evidence, recording the following non-blocking scope gaps explicitly rather than claiming unrun
sub-cases passed:

- **V4 / SC-007 (Export)**: `omo-harness export-kill --source series` (steps 1–3) and `export_v2`
  sidecar/query provenance tests pass; interactive GUI save-dialog / overwrite click-through was
  not separately driven.
- **V7 / SC-005 (`T064` fault injection)**: cancel during reading (10/10), `kill -9` during
  reading (10/10), and network unavailable before request (10/10) pass; mid-read network drop
  (`network-down-during-reading`) was not run on the unprivileged Spark account.
- **V8 / SC-008 (`T066` Chinese IME)**: Chinese station names are verified end-to-end in M1 CSV
  import, preview, and GUI rendering (`V1-spark-aarch64-20261124.md`), and interactive X11 IBus
  composition was verified in M0 (`N6`); a separate M1 interactive IME session in the SQL editor /
  filter was accepted without rerun.
- **UI checklist note**: M1 renders the station list in a plain scrolling checklist (capped at
  300 rows) rather than `virtual_list`.
