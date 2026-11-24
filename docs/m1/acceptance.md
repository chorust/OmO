# M1 Acceptance Matrix

| Scenario | Status | Result | Evidence |
|---|---|---|---|
| S1 — Plan G feasibility gate | passed | 100 stations, 57-point crosscheck, 59.6 s with 20 parallel single-point workers (user-approved Plan G) | [S1-spark-aarch64-20261123.md](evidence/S1-spark-aarch64-20261123.md) |
| V1 — Default series end to end (US1, US2; SC-001, SC-004) | passed | GUI flow verified via automated driver on isolated X11: 100 stations imported, mapping verified, series completed in 66.13 s, truncation note and badge `Received, not saved` rendered | [V1](evidence/V1-spark-aarch64-20261124.md), [CI-SERIES](evidence/CI-SERIES-spark-aarch64-20261123.md), [PERF](evidence/PERF-spark-aarch64-20261123.md) |
| V2 — Reference accuracy (SC-003) | passed | 57 stations, pinned run 2026-11-23T06Z: 57/57 coordinates equal, 6213 values, max \|Δ\| 0.0 | [SC-003](evidence/SC-003-spark-aarch64-20261123.md) |
| V3 — Exploration (US3) | passed | Station checklist, selection, window controls, and Plot tab rendered in GUI | [V1](evidence/V1-spark-aarch64-20261124.md) |
| V4 — Export (US4; SC-007) | partial | `export-kill` steps 1/2/3 with a `/2` series source pass; `/2` sidecar content covered by `export_v2` test | [V4](evidence/V4-spark-aarch64-20261123.md) |
| V5 — Project round trip and drafts (US5; SC-006) | passed | Draft recovery modal dialog verified on restart (`Recover unsaved project?`), 100 stations restored; close guard intercepted on dirty project; round trip and golden hash tested | [V5](evidence/V5-spark-aarch64-20261124.md) |
| V6 — SQL (US6) | passed | `sql_session` views and daily-max query covered by `omo-worker` test; GUI SQL editor rendered | [V6-screenshot](evidence/V6-01-sql-editor.png) |
| V7 — Fault injection (SC-005) | partial | Cancel 10/10 (after a cancellation fix), kill -9 10/10, offline 10/10; network-down-during-reading not run | [V7](evidence/V7-spark-aarch64-20261123.md) |
| V8 — Chinese IME (SC-008) | partial | Chinese station names (北京0, 上海1, 广州2, 深圳3, etc.) rendered and processed across CSV import, preview, and worker queries; interactive X11 IBus composition covered by M0 evidence | [V1](evidence/V1-spark-aarch64-20261124.md) |
| V9 — Cross-platform (SC-009) | partial | `scripts/ci-verify-series.py` passes on linux_arm64 and is wired into CI; amd64/macOS jobs pending git push | [CI-SERIES](evidence/CI-SERIES-spark-aarch64-20261123.md) |
| SC-001 — First interaction time | passed | GUI startup immediate, 100-station import and series run completed in 66.13 s | [V1](evidence/V1-spark-aarch64-20261124.md) |
| SC-002 — 100 stations median of 5 (< 2 min) | passed | Median 77.99 s headless (73.26–86.55 s), 66.13 s GUI wall time (both well under 120 s) | [PERF](evidence/PERF-spark-aarch64-20261123.md), [V1](evidence/V1-spark-aarch64-20261124.md) |
| SC-003 — Reference accuracy | passed | See V2 | [SC-003](evidence/SC-003-spark-aarch64-20261123.md) |
| SC-004 — Default readiness & run resolution | passed | Catalog `Ready` and latest complete run (18Z) resolved and rendered in GUI badge | [V1](evidence/V1-spark-aarch64-20261124.md) |
| SC-005 — Fault injection recovery | partial | See V7 | [V7](evidence/V7-spark-aarch64-20261123.md) |
| SC-006 — Project round-trip & canonical hash | passed | Golden hash tested; draft recovery modal verified in GUI | [V5](evidence/V5-spark-aarch64-20261124.md) |
| SC-007 — Export provenance sidecar `/2` | partial | See V4 | [V4](evidence/V4-spark-aarch64-20261124.md) |
| SC-008 — Chinese IME in SQL & filter | partial | Chinese station names verified in GUI; IBus composition verified in M0 | |
| SC-009 — Cross-platform CI smoke test | partial | See V9 | [CI-SERIES](evidence/CI-SERIES-spark-aarch64-20261123.md) |

## Implementation status

- `cargo +1.100.0 test --workspace --locked` passes (94 passed, 0 failed, 1 ignored); `scripts/check-boundaries.sh` and `--self-test` pass.
- `cargo +1.100.0 fmt --check` reports one diff, in `crates/omo-engine/tests/lock.rs` (an M0 file this work did not touch).
- `cargo +1.100.0 clippy --workspace --locked --all-targets` reports warnings only; CI does not deny warnings.
- **GUI Scenarios V1, V3, V5, V6**: Driven and visually verified via automated X11 agent runner (`scripts/automate_m1_gui.py`) on an isolated Xvfb display, producing full PNG evidence.
- **Remaining gaps**: network-down-during-reading (needs root/netns), and the amd64/macOS remote CI jobs (triggered upon push).
- Known deviation: stations are listed in a plain scrolling checklist (capped at 300 rows) instead of a `virtual_list`.
