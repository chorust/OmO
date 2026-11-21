# Feature 001 implementation and acceptance audit — 2026-11-20

## Outcome

**M0 remains incomplete.** The implementation and a substantial Spark evidence set are present, but the full cross-platform acceptance gate is not satisfied. The roadmap remains `in-progress`; EXIT-3 is `not-run` until the complete shell gate is closed.

## Current evidence inventory

- The task file has 66 tasks: 61 checked and five open — T029, T040, T049, T050, and T061. T019 completed on Spark on 2026-11-19; the remaining tasks require cross-platform/session/channel runs. Their dispositions are retained in [tasks.md](../../specs/001-m0-skeleton-engine/tasks.md).
- The acceptance matrix has 74 expected cells: 26 `pass`, 48 `not-run`, zero `fail`. EXIT-1 and EXIT-5 now pass for both channels on Spark. Every row links to evidence, and the local evidence links resolve. `not-run` rows remain open.
- On Spark, GTK4/Pango/GStreamer development packages are installed in the host package database. The exact current source passes Rust 1.100.0 locked workspace debug and release builds and 52 tests with no `/tmp/gtkdev` override; formatting and boundary checks pass, including all 10 boundary self-test injections. Linux x86_64 and macOS 14 arm64 host builds remain not-run. See [current-source N1 evidence](evidence/N1-spark-aarch64-current-20261120.md), [package installation](evidence/N1-spark-aarch64-n_a-20261119.md), and [host availability](evidence/PLATFORM-AVAILABILITY-20261120.md).
- The 2026-11-19 N9 refresh regenerated all five crate dependency trees for Linux x86_64, Linux aarch64, and macOS arm64 against the current locked manifests. They are target metadata, not host builds. All three N9 rows remain scoped to dependency-boundary checks.
- The 2026-11-20 nested Weston N5 follow-up used a mock portal service that returned `org.freedesktop.portal.Error.Failed`; it verified GTK-native open/save chooser fallback, no-service `ServiceUnknown` fallback, cancellation after each route, and manual-path acceptance after the open error. The successful portal chooser UI from the prior Wayland-client run rendered on host X11. N5 passes for the scoped Spark nested Weston client; native Wayland, Linux x86_64, and macOS remain not-run.
- The source/build/test inputs are identified by the [source snapshot manifest](evidence/SOURCE-SNAPSHOT-20261120.md): 125 files / 1,110,872 bytes, manifest SHA-256 `12e56f6b06e80b29f82f19b1cd31c35244f1ac3d0bef1d4af68a7cd3148483dd`. Runtime reports retain their tested binary hashes and remain scoped to those binaries. Current bootstrap/channel probe SHA-256 is `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d`; current N1 release app/worker hashes are recorded in [N1 current-source evidence](evidence/N1-spark-aarch64-current-20261120.md). Earlier UI runs retain their own app hashes and session scope; the current N1 binaries were not used for N2–N8 runs.
- CI and release workflow files exist locally. Hosted workflow results and a GitHub Release upload have not been produced. No commit or push was made.

## Functional requirement audit

| Requirement | Status | Evidence and scope |
|---|---|---|
| FR-001 | Pass | Five workspace crates, artifact lock, catalog, provenance, and M0 docs are present; see [workspace](../../Cargo.toml) and [tasks](../../specs/001-m0-skeleton-engine/tasks.md). |
| FR-002 | Partial | Spark locked builds pass against installed system prerequisites; Linux x86_64 and macOS arm64 host builds and hosted CI results are unavailable. See [N1](evidence/N1-spark-aarch64-n_a-20261119.md) and [CI workflow](../../.github/workflows/ci.yml). |
| FR-003 | Pass for inspected manifests/targets | Boundary checker and its 10-case self-test pass; current N9 target trees contain no prohibited app/core dependencies. Target trees do not prove target builds. |
| FR-004 | Pass for implementation | NSH code is in the app/UI layer and core dependency trees are UI-free; cross-target build acceptance remains under FR-002. |
| FR-005 | Pass | The approved Duckling snapshot bytes, hashes, mapping, and adaptations are retained and verifiable in [provenance](../../vendor-provenance/DUCKLING.md). |
| FR-006 | Pass | Bootstrap paths and artifact/platform configuration are explicit in the engine configuration and lock. |
| FR-007 | Partial | Bootstrap order and failure handling are implemented; both current Spark channels passed 100/100 on the current probe. Linux x86_64 and macOS channel runs remain open under T029. |
| FR-008 | Partial | Required probes and pre-task checks are implemented and tested; both current Spark channels passed 100/100 with the locked golden result. Full cross-platform repetition remains open under T029. |
| FR-009 | Pass | OM query APIs require a bootstrapped connection; typing checks are recorded in the engine tests. |
| FR-010 | Partial | Channel selection is explicit and no signed failure falls back to unsigned. Current signed and unsigned Spark routes both pass 100/100 and match the golden result; cross-platform combinations and global channel preference remain open. |
| FR-011 | Pass for lock and fetch mechanisms | Both channels and platform entries plus verification tooling are present; compatibility runs remain incomplete. |
| FR-012 | Pass | Bootstrap report fields are shown in the task panel and included in export sidecars; see T044 and EXIT-6 evidence. |
| FR-013 | Partial | Both current Spark channels provision spatial before configuration lock and load it after; a network-isolated signed run also rejects a remote task. Other platform runs remain open. |
| FR-014 | Pass for implementation | Supervised worker framing, generation/request identity, and stdout/stderr separation are implemented and exercised in protocol/live-worker evidence. |
| FR-015 | Pass for implementation | Batch-wise Arrow result files, paging, row/byte limits, and truncation behavior are implemented and covered by engine/app tests. |
| FR-016 | Partial | Crash detection, stale-generation discard, restart, and live recovery are covered on Spark; T040 cross-platform runs remain open. |
| FR-017 | Partial | Interrupt/drain/kill state handling is implemented; some cancel levels use fake-worker state transitions, and cross-platform EXIT-4 runs remain open. |
| FR-018 | Partial | The workbench, editor, paged table, plot, task panel, dialogs, and manual path exist. Acceptance remains scoped to tested Spark sessions; native Wayland and other platform/session cases remain open. |
| FR-019 | Pass for implementation; acceptance scoped | Generation/revision checks and background page decoding are implemented. Spark X11 and nested Weston N7 demonstrate operability; the full matrix remains open. |
| FR-020 | Pass for measured Spark scope | N3 selected a 20-series × 10,000-point cap and labelled downsampling from five activation trials in Spark X11 and nested Weston. Isolated application-only render duration and other platforms remain unmeasured. |
| FR-021 | Open | The full N1–N9 platform/session matrix is incomplete: 49 matrix cells remain `not-run`. |
| FR-022 | Partial | [shell.md](decisions/shell.md) records a provisional NSH 0.5.1 decision for tested Spark scope. The full gate and EXIT-3 remain open. |
| FR-023 | Pass | Plans A and B emitted 27 oracle-matching rows each, passed 10 semantic checks, recorded costs, and selected Plan B for eligible grids; see [series decision](decisions/series-plan.md). |
| FR-024 | Pass for implementation and Spark fault injection | Staging, hash-checked saved state, fsync, and three-step commit are implemented; Spark steps 1–3 produced the expected incomplete/incomplete/finished results. |
| FR-025 | Pass for implementation and tests | Startup recovery scanning and saved-predicate behavior are implemented and covered by tests; cross-platform export runs remain open. |
| FR-026 | Pass for implementation and tests | Cross-filesystem commits fail with the explicit EXDEV error; covered by export tests. |
| FR-027 | Pass for evidence disposition | Every N/SPIKE/EXIT cell has a linked record, including explicit `not-run` reasons. This records evidence scope; it does not turn unavailable runs into passes. |

## Success criteria audit

| Criterion | Status | Evidence and limitation |
|---|---|---|
| SC-001 | Open | Both current Spark channels passed 100/100 standard network-enabled repeats on the same current probe. Linux x86_64/macOS channel runs remain open. |
| SC-002 | Open | Current Spark signed and unsigned repeats both report the locked 64-row golden hash while retaining distinct artifact hashes. Linux x86_64, macOS arm64, and the global channel decision remain open. |
| SC-003 | Pass | Both station-series plans matched the independent 27-row oracle and semantic checks; Plan B was selected for eligible grids. |
| SC-004 | Pass for exercised Spark harness scope | Crash recovery passed 20/20 fake-worker injections and live restart was observed. Cross-platform harness runs remain open. |
| SC-005 | Pass for Spark; cross-platform open | Spark export fault injections at all three steps yielded incomplete, incomplete, and finished. Linux x86_64/macOS runs remain open. |
| SC-006 | Open | The matrix has 48 `not-run` cells; the documented provisional NSH decision is not a completed full fallback/acceptance decision. |
| SC-007 | Pass for tested Spark sessions | Spark X11 and nested Weston each completed a ten-minute 50 Hz progress workload, UI interaction, and worker restart; maximum callback gaps were 64.945 ms and 45.072 ms respectively. These include scheduler delay and are not input-to-render latency. |
| SC-008 | Pass | All 10 seeded boundary violations were detected by the self-test. |

## Open completion gates

- **T029:** Both current Spark channels passed 100/100 fresh standard runs on probe SHA-256 `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d`; their artifact hashes are distinct and their golden hashes match. Linux x86_64 and macOS arm64 channel repeats and two-channel comparisons remain incomplete. Current host access failures are recorded in [platform availability evidence](evidence/PLATFORM-AVAILABILITY-20261120.md).
- **T040 and T061:** Spark harness evidence exists. Linux x86_64 and macOS arm64 runs are unavailable.
- **T049 and T050:** Linux x86_64 Wayland/X11 sessions and macOS 14 arm64 host/session runs are unavailable.
- **N5/N6:** Spark nested Weston N5 now covers portal D-Bus errors, no-service fallback, open/save cancellation, and manual path. Native Wayland, Linux x86_64, and macOS dialog cases remain unverified. Nested Wayland IBus candidate selection, native Wayland, and other platform IME cases remain open.
- **Hosted CI/release:** Workflow matrices were reviewed locally; hosted CI and release upload remain unverified. No commit or push was made.

Do not mark feature 001 `verified` or close EXIT-3 until these gates have evidence or the user-approved decision process resolves them.
