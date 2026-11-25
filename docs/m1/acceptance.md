# M1 Acceptance Matrix

On 2026-11-25 the user approved closing M1 (`002`) as `verified` under Constitution 1.3.0 with the
Spark evidence below plus [GitHub CI and release evidence](evidence/CI-CROSS-PLATFORM-20261125.md)
(CI runs `45348466177` / `45348599457` and `v0.1.0-m1` release run `45349560323`). Rows labelled
`accepted (scope note)` record user-accepted non-blocking scope gaps explicitly rather than
claiming unrun sub-cases passed.

| Scenario | Status | Result | Evidence |
|---|---|---|---|
| S1 — Plan G feasibility gate | passed | 100 stations, 57-point crosscheck, 59.6 s with 20 parallel single-point workers (user-approved Plan G) | [S1-spark-aarch64-20261123.md](evidence/S1-spark-aarch64-20261123.md) |
| V1 — Default series end to end (US1, US2; SC-001, SC-004) | passed | GUI flow verified via automated driver on isolated X11: 100 stations imported, mapping verified, series completed in 66.13 s, truncation note and badge `Received, not saved` rendered | [V1](evidence/V1-spark-aarch64-20261124.md), [CI-SERIES](evidence/CI-SERIES-spark-aarch64-20261123.md), [PERF](evidence/PERF-spark-aarch64-20261123.md) |
| V2 — Reference accuracy (SC-003) | passed | 57 stations, pinned run 2026-11-23T06Z: 57/57 coordinates equal, 6213 values, max \|Δ\| 0.0 | [SC-003](evidence/SC-003-spark-aarch64-20261123.md) |
| V3 — Exploration (US3) | passed | Station checklist, selection, window controls, and Plot tab rendered in GUI | [V1](evidence/V1-spark-aarch64-20261124.md) |
| V4 — Export (US4; SC-007) | accepted (scope note) | `export-kill` steps 1/2/3 with a `/2` series source pass; `/2` sidecar content covered by `export_v2` test; interactive GUI save/overwrite click-through accepted without rerun | [V4](evidence/V4-spark-aarch64-20261123.md), [CI 2026-11-25](evidence/CI-CROSS-PLATFORM-20261125.md) |
| V5 — Project round trip and drafts (US5; SC-006) | passed | Draft recovery modal dialog verified on restart (`Recover unsaved project?`), 100 stations restored; close guard intercepted on dirty project; round trip and golden hash tested | [V5](evidence/V5-spark-aarch64-20261124.md) |
| V6 — SQL (US6) | passed | `sql_session` views and daily-max query covered by `omo-worker` test; GUI SQL editor rendered | [V6-screenshot](evidence/V6-01-sql-editor.png) |
| V7 — Fault injection (SC-005) | accepted (scope note) | Cancel 10/10 (after cancellation fix), `kill -9` 10/10, offline-before-request 10/10; mid-read network drop not run on unprivileged Spark host and accepted as scope gap | [V7](evidence/V7-spark-aarch64-20261123.md), [CI 2026-11-25](evidence/CI-CROSS-PLATFORM-20261125.md) |
| V8 — Chinese IME (SC-008) | accepted (scope note) | Chinese station names (北京0, 上海1, 广州2, 深圳3, etc.) rendered and processed across CSV import, preview, and worker queries; interactive X11 IBus composition covered by M0 N6 evidence | [V1](evidence/V1-spark-aarch64-20261124.md), [M0 N6](../m0/acceptance.md), [CI 2026-11-25](evidence/CI-CROSS-PLATFORM-20261125.md) |
| V9 — Cross-platform (SC-009) | passed (GitHub CI) | `scripts/ci-verify-series.py` passed live network smoke (`rows: 339`, `plan: "G"`) on `linux_amd64`, `linux_arm64`, and `osx_arm64` in CI run `45348466177`; `v0.1.0-m1` release archives published in run `45349560323` | [CI 2026-11-25](evidence/CI-CROSS-PLATFORM-20261125.md), [CI-SERIES](evidence/CI-SERIES-spark-aarch64-20261123.md) |
| SC-001 — First interaction time | passed | GUI startup immediate, 100-station import and series run completed in 66.13 s | [V1](evidence/V1-spark-aarch64-20261124.md) |
| SC-002 — 100 stations median of 5 (< 2 min) | passed | Median 77.99 s headless (73.26–86.55 s), 66.13 s GUI wall time (both well under 120 s) | [PERF](evidence/PERF-spark-aarch64-20261123.md), [V1](evidence/V1-spark-aarch64-20261124.md) |
| SC-003 — Reference accuracy | passed | See V2 | [SC-003](evidence/SC-003-spark-aarch64-20261123.md) |
| SC-004 — Default readiness & run resolution | passed | Catalog `Ready` and latest complete run (18Z) resolved and rendered in GUI badge | [V1](evidence/V1-spark-aarch64-20261124.md) |
| SC-005 — Fault injection recovery | accepted (scope note) | See V7 | [V7](evidence/V7-spark-aarch64-20261123.md), [CI 2026-11-25](evidence/CI-CROSS-PLATFORM-20261125.md) |
| SC-006 — Project round-trip & canonical hash | passed | Golden hash tested; draft recovery modal verified in GUI | [V5](evidence/V5-spark-aarch64-20261124.md) |
| SC-007 — Export provenance sidecar `/2` | accepted (scope note) | See V4 | [V4](evidence/V4-spark-aarch64-20261123.md), [CI 2026-11-25](evidence/CI-CROSS-PLATFORM-20261125.md) |
| SC-008 — Chinese IME in SQL & filter | accepted (scope note) | See V8 | [V1](evidence/V1-spark-aarch64-20261124.md), [M0 N6](../m0/acceptance.md), [CI 2026-11-25](evidence/CI-CROSS-PLATFORM-20261125.md) |
| SC-009 — Cross-platform CI smoke test | passed (GitHub CI) | See V9 | [CI 2026-11-25](evidence/CI-CROSS-PLATFORM-20261125.md), [CI-SERIES](evidence/CI-SERIES-spark-aarch64-20261123.md) |

## Closure status (2026-11-25)

- Verified against commit `3affd776a4bab0324c43d6871ccabe90fddd5472` (`v0.1.0-m1`): `cargo +1.100.0 clippy --workspace --locked --all-targets` (warnings only; CI does not deny warnings), `cargo +1.100.0 test --workspace --locked` (97 passed, 0 failed, 1 ignored on Linux; 96 passed on macOS), `scripts/check-boundaries.sh`, and `scripts/check-boundaries.sh --self-test` all pass. Formatting diffs in `3affd77` (`cargo +1.100.0 fmt --check`) are left untouched so `crates/` stays byte-identical to `v0.1.0-m1`.
- **GUI Scenarios V1, V3, V5, V6**: Driven and visually verified via automated X11 agent runner (`scripts/automate_m1_gui.py`) on an isolated Xvfb display, producing full PNG evidence.
- **Cross-platform CI & release (V9 / SC-009)**: Closed by GitHub Actions CI run `45348466177` (live `ci-verify-series.py` pass on `linux_amd64`, `linux_arm64`, `osx_arm64`) and `v0.1.0-m1` release run `45349560323`; see [CI-CROSS-PLATFORM-20261125.md](evidence/CI-CROSS-PLATFORM-20261125.md).
- **Accepted non-blocking scope gaps**: GUI save-dialog click-through in V4, mid-read network drop in V7 (requires root/netns), separate interactive X11 IME rerun in V8 (covered by M0 N6 + M1 Chinese station rendering), and plain scrolling checklist (capped at 300 rows) instead of `virtual_list`.
