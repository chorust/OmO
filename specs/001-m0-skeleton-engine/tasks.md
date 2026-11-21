---
description: "Task list for M0 Skeleton & Usable Engine"
---

# Tasks: M0 Skeleton & Usable Engine

**Input**: Design documents from `/specs/001-m0-skeleton-engine/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: REQUIRED. The constitution's "Development Workflow & Quality Gates" makes tests
mandatory for bootstrap probes, protocol framing, generation discard, cancellation, export
commit/recovery, and preview staleness. Fault injection is part of acceptance. Write the
tests first and confirm they fail.

**Resolved decisions and gates (2026-11-16)**:
- **FR-005**: user approved the content-addressed Duckling working-tree snapshot route. Capture
  and retain exact source bytes, then record per-file hashes and path mapping; no Duckling edits.
- **FR-022**: user approved research D8's blocking examples. Record N7 stall and responsiveness
  without a numeric threshold; other non-blocking measurements are recorded as limitations.
- **GATE INSTALL**: user approved Rust 1.100 and the documented Ubuntu build/IME packages.
  Package installation still requires host elevation; do not mark install-dependent tasks done
  until the package/build/session evidence exists.

**Evidence**: every acceptance task writes an evidence file per
`contracts/acceptance-record.md`. Never mark `pass` without one.

## Format: `[ID] [P?] [Story] Description`

## Phase 1: Setup (Shared Infrastructure)

- [x] T001 Create workspace `Cargo.toml` (resolver=2, members `crates/*`, shared exact pins: `duckdb =1.20000.1`, `arrow`/`arrow-ipc 60`, `nsh-* =0.5.1`) and `rust-toolchain.toml` pinned to an exact stable ≥1.97 (research D1, proposed 1.100.0)
- [x] T002 [P] Create crate skeletons `crates/omo-engine/Cargo.toml`, `crates/omo-protocol/Cargo.toml`, `crates/omo-worker/Cargo.toml`, `crates/omo-project/Cargo.toml` (no UI deps) and `crates/omo-app/Cargo.toml` (NSH platform crates per target with `default-features=false, features=["editor","highlight","plot"]`; optional non-default `in-process-engine` feature → `omo-engine`), each with `src/lib.rs`/`src/main.rs` stubs
- [x] T003 [P] Create placeholders `catalog/products.json` (`{"schema":"omo-catalog/0","products":[]}`), `artifacts/duckomo.lock.json` (`{"schema":"omo-artifact-lock/1","entries":[]}`), `fixtures/PROVENANCE.md`, `vendor-provenance/DUCKLING.md` (header + "source pending FR-005")
- [x] T004 [P] Create `docs/m0/acceptance.md` (empty matrix per `contracts/acceptance-record.md`), `docs/m0/n-checklist.md` (N1–N9 steps from spec US4), `docs/m0/evidence/.gitkeep`, `docs/m0/decisions/.gitkeep`
- [x] T005 [P] Write `docs/m0/linux-prereqs.md` listing the candidate Ubuntu packages from research D3, marked unverified until N1
- [x] T006 Generate `Cargo.lock` with `cargo generate-lockfile` and confirm `cargo build --workspace --locked` on the dev host; record the resolved `nsh-*`, `duckdb`, and `arrow` versions in `docs/m0/evidence/T006-lock-spark-aarch64-20261116.md`

---

## Phase 2: Foundational (Blocking Prerequisites)

- [x] T007 [P] Tests for frame codec (round-trip, >16 MiB rejected, truncated frame) in `crates/omo-protocol/tests/frame.rs`
- [x] T008 [P] Tests for envelope/messages serde and generation filtering helper in `crates/omo-protocol/tests/messages.rs`
- [x] T009 Implement `crates/omo-protocol/src/{frame.rs,messages.rs,result_ref.rs,state.rs}` per `contracts/worker-protocol.md` and data-model (Envelope, ResultRef, JobState, FailKind, CancelLevel)
- [x] T010 [P] Tests for revision/dirty semantics and PreviewKey equality in `crates/omo-project/tests/doc.rs`
- [x] T011 Implement `crates/omo-project/src/{doc.rs,preview_key.rs}` (ProjectDoc minimal, PreviewKey)
- [x] T012 [P] Tests for lock parsing and lookup by (channel, version, duckdb_version, platform), with a missing entry rejected and a placeholder golden hash treated as failure, in `crates/omo-engine/tests/lock.rs`
- [x] T013 Implement `crates/omo-engine/src/lock.rs` (ArtifactLockEntry, lookup, size + SHA-256 verification of the decompressed file) per `contracts/artifact-lock.md`
- [x] T014 [P] Implement `scripts/fetch-duckomo.sh <channel> <platform>` (download into a fresh temp dir; verify against release `SHA256SUMS` for the unsigned channel; print the lock entry JSON; never edit the lock in place)

**Checkpoint**: the protocol, project doc, and lock compile and pass their tests. The user stories can start.

---

## Phase 3: User Story 1 - Workspace skeleton with enforced boundaries (P1) 🎯 MVP

**Goal**: locked builds on 3 platforms, and CI that rejects forbidden dependencies (FR-001–004, N9).
**Independent Test**: run `scripts/check-boundaries.sh` and its `--self-test`. The CI matrix is green.

- [x] T015 [US1] Implement `scripts/check-boundaries.sh` per `contracts/ci-checks.md` (core crates UI-free; app default build has no duckdb/libduckdb-sys; no gpui/nsh-platform-gpui/tauri/quickjs/nsh-js; exact pins)
- [x] T016 [US1] Implement `scripts/check-boundaries.sh --self-test`: copy the manifests to a temp dir, inject each forbidden dependency, and assert detection (SC-008)
- [x] T017 [US1] Create `.github/workflows/ci.yml` with a matrix of `ubuntu-24.04`, `ubuntu-24.04-arm`, and `macos-14`: install the Linux GTK/GStreamer dev packages on runners, then `cargo build/test --workspace --locked` and `scripts/check-boundaries.sh` (research D10; runner availability unverified). A later release-asset request also adds `macos-15-intel` to CI and adds `.github/workflows/release.yml`, which builds Linux arm64/x86_64 and macOS arm64/x86_64 archives only after a GitHub Release is published. The workflow files exist locally; their hosted-runner results are not acceptance evidence.
- [x] T018 [US1] Record N9 per platform in `docs/m0/evidence/N9-<platform>-n_a-<date>.md` (the `cargo tree` outputs for `omo-app` and each core crate)
- [x] T019 [US1] APPROVED INSTALL — On spark, install the toolchain (≥1.97) and the GTK4/Pango/GStreamer dev packages, build from source with `--locked`, and record N1 with the exact package list in [the 2026-11-19 Spark N1 record](../../docs/m0/evidence/N1-spark-aarch64-n_a-20261119.md); update `docs/m0/linux-prereqs.md`

**T019 Spark status (2026-11-20):** Required GTK4/Pango/GStreamer development packages are installed in the host dpkg database; `dpkg --audit`, root `apt-get check`, and host pkg-config discovery pass. The exact current source passes Rust 1.100 locked debug and release workspace builds and 52 workspace tests without `/tmp/gtkdev`; see the [current N1 verification](../../docs/m0/evidence/N1-spark-aarch64-current-20261120.md), the [package transaction](../../docs/m0/evidence/N1-spark-aarch64-n_a-20261119.md), and [Linux prerequisite guide](../../docs/m0/linux-prereqs.md).
- [x] T020 [P] [US1] Record N1 on macOS 14 arm64 and Linux x86_64 in `docs/m0/evidence/N1-<platform>-n_a-<date>.md` (hardware per roadmap OQ-B3; otherwise `not-run (no host)`)

---

## Phase 4: User Story 2 - Verified engine bootstrap on every platform (P1)

**Goal**: `open_bootstrapped` with probes and golden query on 3 platforms × 2 channels; channel and R4 decisions (FR-005–013).
**Independent Test**: `omo-probe --repeat 100` per channel per platform.

- [x] T021 [P] [US2] Tests (written first, expected to fail): artifact size/SHA mismatch → `ArtifactMismatch` with nothing loaded; wrong extension version → `ProbeFailed`; a signed-channel failure never retries unsigned; probe failure before a task discards the connection. In `crates/omo-engine/tests/bootstrap.rs` (needs the T024 fixture and the T025 lock entries for the live cases; pure-logic cases use a fake)
- [x] T022 [P] [US2] Test: OM query APIs accept only `&BootstrappedConn` (compile-fail doc test or trybuild) in `crates/omo-engine/tests/typing.rs`
- [x] T023 [US2] Implement from the approved FR-005 snapshot — Capture and retain selected Duckling engine sources, verify their per-file hashes, then adapt the session/connection/query/limits code into `crates/omo-engine/src/{conn.rs,query.rs,limits.rs}`; write `vendor-provenance/DUCKLING.md`; Arrow results are written batch-wise, never collected into one batch (design §3.3)
- [x] T024 [P] [US2] Select and add the golden OM fixture `fixtures/om/<name>.om` with source, licence (Open-Meteo CC BY 4.0), and hash in `fixtures/PROVENANCE.md`; define query `golden-v1` (deterministic ORDER BY) in `crates/omo-engine/src/probe.rs`
- [x] T025 [US2] Run `scripts/fetch-duckomo.sh` for 2 channels × {linux_amd64, linux_arm64, osx_arm64}; commit-ready lock entries go into `artifacts/duckomo.lock.json` (golden hash still a placeholder); record the downloads in `docs/m0/evidence/SPIKE-3-fetch-<date>.md`
- [x] T026 [US2] Implement bootstrap internals from the approved FR-005 route — Implement `crates/omo-engine/src/{config.rs,bootstrap.rs,probe.rs}`: BootstrapConfig; open with `extension_directory`, setting `allow_unsigned_extensions` only for the unsigned channel; httpfs LOAD → INSTALL with 15 s timeout → degrade (`caps.remote=false`); duckomo verify+load per channel; close the unsigned window and lock config; run the probes (version, platform, extension_version == lock, the four functions) and the golden query; produce BootstrapReport; `ensure_spatial`
- [x] T027 [US2] Implement signed loading through a private local DuckDB repository configured with the lock-pinned community key; verify installed bytes before `LOAD`, close repository access afterward, and preserve no-fallback behavior. Spark local-route evidence: `docs/m0/evidence/SPIKE-3-community-local-repository-spark-aarch64-20261120.md`; other target platforms remain unrun.
- [x] T028 [US2] Implement the `omo-probe` bin (`--channel`, `--repeat N`, `--r4`, `--no-network`, JSON report) in `crates/omo-worker/src/bin/omo-probe.rs`
- [ ] T029 [US2] On each platform × channel: run `omo-probe --repeat 100`, then fill `golden.result_sha256` only from the first verified run, and confirm both channels match. Evidence goes to `docs/m0/evidence/EXIT-1-<platform>-<date>.md` and `EXIT-5-<platform>-<date>.md`

**T029 Spark status (2026-11-20):** Both current channels passed 100/100 fresh connections on probe SHA-256 `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d`; they report distinct locked artifact hashes and the same 64-row golden hash `bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f`. Current reports and the Spark-only channel decision are in [EXIT-1](../../docs/m0/evidence/EXIT-1-spark-aarch64-n-a-20261120.md) and [EXIT-5](../../docs/m0/evidence/EXIT-5-spark-aarch64-current-20261120.md). T029 remains open because Linux x86_64 and macOS arm64 host/channel repeats are not-run; see [availability evidence](../../docs/m0/evidence/PLATFORM-AVAILABILITY-20261120.md).
- [x] T030 [US2] Run `omo-probe --r4` (unsigned close after open; LOAD spatial after config lock) and `--no-network` (httpfs degrade; a remote task is rejected with `CapabilityUnavailable(remote)`). Write `docs/m0/decisions/r4.md`, including the mitigation choice (pre-load spatial or a spatial-capable worker)

**T030 current evidence:** Both channels pass R4 on the current probe; spatial is provisioned before lock and loads after lock. The unsigned window closes, signed route starts with unsigned loading disabled, and network-isolated remote work is rejected. See [current R4 evidence](../../docs/m0/evidence/SPIKE-4-current-r4-spark-aarch64-20261120.md).
- [x] T031 [US2] Write `docs/m0/decisions/channel.md` (per-platform default under the design §4.1 rule; OQ-4 remains with the user)

---

## Phase 5: User Story 3 - Supervised engine worker (P1)

**Goal**: child-process engine with generations, IPC results, crash recovery, and 3-level cancel (FR-014–017).
**Independent Test**: the `omo-harness crash-inject` and `cancel` runs.

- [x] T032 [P] [US3] Tests: supervisor drops stale-generation frames and deletes their files; a crash marks `Failed(WorkerCrashed)` and restarts at generation+1; heartbeat timeout → Kill; cancel escalates Interrupt → Drain(≤10 s) → Kill. In `crates/omo-app/tests/supervisor.rs` (host layer only, using a fake worker)
- [x] T033 [P] [US3] Test: the worker's stdout contains only valid frames under log pressure, in `crates/omo-worker/tests/stdout_clean.rs`
- [x] T034 [US3] Implement the `omo-worker` server loop (`hello` → bootstrap → `ready`/`bootstrap_failed`; `query` with the pre-task function probe; `interrupt`; `shutdown`; 1 s heartbeat; logs to stderr) in `crates/omo-worker/src/{main.rs,server.rs,heartbeat.rs}` — live bootstrap/query/export/cancel/heartbeat evidence: `docs/m0/evidence/T034-worker-live-spark-aarch64-20261116.md`
- [x] T035 [US3] Implement batch-wise Arrow IPC file writing with row/byte limits and the `truncated` flag at `<session_tmp>/g<gen>/<id>.arrow` in `crates/omo-engine/src/results.rs` (depends on T023)
- [x] T036 [US3] Implement `EngineSupervisor` (own thread + tokio runtime; spawn/kill; generation counter; heartbeat watchdog; request routing; stale discard; stderr ring buffer) in `crates/omo-app/src/host/supervisor.rs`, and the cancel state machine in `crates/omo-app/src/host/cancel.rs`
- [x] T037 [US3] Implement mmap page reads of ResultRef files (no duckdb dependency) in `crates/omo-app/src/host/ipc_pages.rs`
- [x] T038 [US3] Implement `PreviewQueue` (one running preview per doc, coalescing, interrupting the superseded preview, PreviewKey check, stale-retain on error) in `crates/omo-app/src/host/preview_queue.rs`, with tests in `crates/omo-app/tests/preview_queue.rs`

**T039 implementation and runtime status (2026-11-20):** All five subcommands have source implementations. `crash-inject`, `cancel`, `gen-result`, `progress`, and `export-kill` have Spark evidence in [T039 harness evidence](../../docs/m0/evidence/T039-harness-20261116.md), [current EXIT-4 follow-up](../../docs/m0/evidence/EXIT-4-spark-aarch64-n-a-20261120.md), and [current EXIT-6 follow-up](../../docs/m0/evidence/EXIT-6-spark-aarch64-n-a-20261120.md). Cancellation levels 2/3 simulate host state transitions, not live worker drain timing; live interrupt and restart scope is separately recorded. T039 implementation is checked; cross-platform acceptance runs remain in T040 and T061.

- [x] T039 [US3] Implement `omo-harness` subcommands `crash-inject`, `cancel`, `gen-result`, `progress`, and `export-kill` in `crates/omo-app/src/bin/omo-harness.rs`
**T040 status (2026-11-20):** Spark harness rerun passed 20/20 fake crash recoveries and all three cancellation harness paths; supervisor tests record 0 stale results accepted. The current follow-up is [EXIT-4](../../docs/m0/evidence/EXIT-4-spark-aarch64-n-a-20261120.md). Linux x86_64 and macOS arm64 runs remain not-run, so T040 stays open.

- [ ] T040 [US3] Run the crash and cancel harness on each platform; record EXIT-4 in `docs/m0/evidence/EXIT-4-<platform>-<date>.md` (recoveries, 0 stale results accepted, the cancel level that ended each task)

---

## Phase 6: User Story 4 - NSH minimal workbench and shell decision (P1)

**Goal**: a minimal workbench and N1–N8 on the full matrix, leading to a shell decision (FR-018–022).
**Independent Test**: `docs/m0/n-checklist.md` per platform/session.

**Implementation and evidence status (2026-11-18):** T041–T047 implementation tasks are checked. T044 creates a generation-scoped plot artifact (≤20 numeric columns × ≤10,000 evenly sampled rows) alongside the full IPC result; the UI reads that artifact for worker results. The fixture-only app path still samples `OMOBENCH_FIXTURE_RESULT` for N2/N3 workloads. Engine, worker, app-supervisor, live protocol, and workspace validation evidence is recorded in [T044 evidence](../../docs/m0/evidence/T044-worker-plot-sampling-20261117.md). T048's spark evidence sweep is recorded for N2–N8: X11 cells have scoped runs; nested Weston Wayland has scoped N2–N5 and N7/N8 evidence plus partial N6 evidence, with explicit not-run or limited status for unavailable/incomplete subcases. Checking T048 records that every requested spark cell has an evidence disposition; it does not convert a not-run cell to pass or close the full N matrix. T019 is complete; remaining unchecked tasks are T029, T040, T049, T050, and T061, which retain explicit platform/channel work. The latest partial nested Weston Fcitx N6 run displayed and selected candidate 2, then committed it; its Wayland input-panel error and the unverified IBus selection are recorded in [N6 follow-up evidence](../../docs/m0/evidence/N6-spark-aarch64-wayland-20261118.md). The full Wayland cell remains not-run. Spark X11 N5 open/save cancellation with portal services available and unavailable, plus manual-path fallback via the validation error seam, is recorded in [N5 fallback evidence](../../docs/m0/evidence/N5-spark-aarch64-x11-20261118.md). The 2026-11-20 nested Weston run verifies mocked portal D-Bus errors and no-service fallback for open/save, cancellation, and manual-path acceptance after a portal error; see [current N5 evidence](../../docs/m0/evidence/N5-spark-aarch64-wayland-20261120.md). The successful portal UI rendered on host X11 in the 2026-11-18 Wayland-client run; native Wayland and other platform cells remain unverified. The spark X11 six-section smoke run and debrief are recorded in [quickstart evidence](../../docs/m0/evidence/QUICKSTART-spark-aarch64-x11-20261117.md) and [the debrief report](roadmap-reviews/debrief-20261117T103201Z.md). Additional evidence records N3 hover/cap/timing and the N7 main-thread callback interval. Five-trial spark Xvfb X11 and nested Weston Wayland runs recorded Plot-tab activation-to-visible timing and selected 20×10,000 as the measured M0 plot cap; Linux x86_64 and macOS N3 cells remain open. Not-run acceptance cells are not marked passed.

- [x] T041 [US4] Implement the app entry and `DockArea` layout (left placeholder, centre tabs, bottom task panel), the app menu, and shortcuts (run, cancel) in `crates/omo-app/src/{main.rs,ui/mod.rs,ui/workspace.rs}`. Record the minimal NSH feature set that compiles (research D2)
- [x] T042 [P] [US4] Implement the SQL `Editor` view that submits to the PreviewQueue in `crates/omo-app/src/ui/sql_view.rs`
- [x] T043 [P] [US4] Implement the `DataTable` view (rowCount from ResultRef, visible-range page fetch via `ipc_pages`, sort → re-query) in `crates/omo-app/src/ui/table_view.rs`
- [x] T044 [P] [US4] Implement the `Plot` view with a series/point cap and a "downsampled" label (initial ≤20 × ≤10,000; worker-side downsampling) in `crates/omo-app/src/ui/plot_view.rs` — [worker and UI evidence](../../docs/m0/evidence/T044-worker-plot-sampling-20261117.md)
- [x] T045 [P] [US4] Implement the task panel (task states, cancel/retry, BootstrapReport including channel and httpfs state) in `crates/omo-app/src/ui/tasks_view.rs`
- [x] T046 [P] [US4] Implement the file dialogs (`pick_paths` for station CSV, `pick_save_path` for export; cancel → None; on error → manual path entry) in `crates/omo-app/src/ui/dialogs.rs`
- [x] T047 [US4] Enforce the threading contract (only the engine thread writes signals, after the generation/revision check; page decoding in `background`). If `spawn`→UI delivery differs on 0.5.1, use NSH's cross-thread post and record the difference in `docs/m0/evidence/N7-notes-<date>.md`
- [x] T048 [US4] APPROVED INSTALL — Run N2–N8 on spark × {X11, Wayland}, recording frame time p50/p95, max main-thread stall, and RSS, in `docs/m0/evidence/N<k>-spark-aarch64-<session>-20261116.md`; any session that cannot be run is recorded as `not-run (reason)`. Evidence sweep completed: X11 runs and scoped nested Weston Wayland evidence (N2–N5, N7/N8; partial N6) are linked from `docs/m0/acceptance.md`; remaining unavailable or unverified cells stay `not-run` and are not passes.
- [ ] T049 [P] [US4] Run N2–N8 on Linux x86_64 × {Wayland, X11} → `docs/m0/evidence/N<k>-linux-x86_64-<session>-<date>.md` (host per OQ-B3)
- [ ] T050 [P] [US4] Run N2–N8 on macOS 14 arm64 → `docs/m0/evidence/N<k>-macos14-arm64-n_a-<date>.md` (host per OQ-B3)
- [x] T051 [US4] Apply user-approved FR-022 D8 classification — Classify N-results as blocking or recorded limitation using the approved examples; report N7 maximum stall without a numeric threshold; set the final plot cap from N3; write `docs/m0/decisions/shell.md` (accept NSH / evaluate a compatible NSH release / trigger the §7.3 GPUI Kit fallback; embedding is never an option). The decision retains NSH provisionally for tested spark scope and keeps unverified platforms, Wayland subcases, and EXIT-3 open. If the fallback triggers, stop and open a new spec for the `omo-app::ui` swap; do not implement it here

---

## Phase 7: User Story 5 - Station × nearest-grid series plan (P2)

**Goal**: the A vs B spike, with the `station_series` schema and a reference comparison (FR-023).
**Independent Test**: `omo-series-spike --plan A|B` produces two outputs and a comparison report.

- [x] T052 [P] [US5] Acquire reference data read-only from committed Duckomo v0.4.0 fixture and official-reader CSV. Retain byte-identical files, the station list, independent nearest-cell oracle, and hashes/provenance in `fixtures/reference/`, `fixtures/stations/m0.csv`, and `fixtures/PROVENANCE.md`; source Duckomo was not modified (OQ-B2)
- [x] T053 [P] [US5] Test the `station_series` schema, de-duplication on (station_id, variable, valid_time), conflicting duplicate rejection, longitude normalisation, UTC, and null values in `crates/omo-engine/tests/series_schema.rs`
- [x] T054 [US5] Implement Plan A (`om_grid_info` → host nearest-index for eligible regular spherical grids → `read_om_series` point-index join) in `crates/omo-engine/src/series/plan_a.rs`; record result: semantically correct, but the locked API has no selected-point argument and the run falls back to 150 full-grid candidates for five selected points
- [x] T055 [US5] Implement Plan B (enumerate chunks via `chunk_time_length/time_step` → `read_om_regions(method:='nearest')` with buffer polygons and stable `region_id` → append chunk results as UNION ALL → dedup/sort) in `crates/omo-engine/src/series/plan_b.rs`; spatial preinstall and `ensure_spatial` passed under the r4.md mitigation
- [x] T056 [US5] Implement `omo-series-spike` with per-item semantic oracle comparison, provenance checks, `duckomo_last_scan_metrics()` byte fields, elapsed time, CSV output, and JSON report in `crates/omo-worker/src/bin/omo-series-spike.rs`
- [x] T057 [US5] Run both plans on the same fixture/stations/window; write `docs/m0/evidence/EXIT-2-spark-aarch64-20261117.md`, the per-plan CSV/reports, comparison JSON, and `docs/m0/decisions/series-plan.md`

---

## Phase 8: User Story 6 - Atomic export with crash recovery (P2)

**Goal**: an export whose sidecar rename is the commit point, plus a startup scan (FR-024–026).
**Independent Test**: `omo-harness export-kill --at step1|step2|step3`.

- [x] T058 [P] [US6] Tests: the saved predicate; cleanup or flagging per `contracts/export-protocol.md`; EXDEV → `Export(CrossFilesystem)`; overwrite invalidates the old sidecar first. In `crates/omo-engine/tests/export.rs` and `crates/omo-app/tests/export_scan.rs` — `EXIT-6-spark-aarch64-n-a-20261116.md`
- [x] T059 [US6] Implement the export (staging dir, `COPY` to CSV/Parquet, sidecar `omo-export-sidecar/1` with BootstrapReport/versions/channel/data sha256/attribution, fsyncs, three-step commit) in `crates/omo-engine/src/export.rs`, and the worker `export` request in `crates/omo-worker/src/server.rs` — engine tests and live result-ID export: `docs/m0/evidence/EXIT-6-spark-aarch64-n-a-20261116.md`, `docs/m0/evidence/T034-worker-live-spark-aarch64-20261116.md`
- [x] T060 [US6] Implement the startup scan of recent export dirs in `crates/omo-app/src/host/export_scan.rs`, and wire the overwrite confirmation and `pick_save_path` in `crates/omo-app/src/ui/dialogs.rs`
**T061 Spark status (2026-11-20):** The existing `export-kill` command was rebuilt against host-installed prerequisites and all three fault-injection points passed on Spark; see the [current EXIT-6 follow-up](../../docs/m0/evidence/EXIT-6-spark-aarch64-n-a-20261120.md). T061 remains unchecked because Linux x86_64 and macOS arm64 runs are not-run.

- [ ] T061 [US6] Add `omo-harness export-kill --at step1|step2|step3` in `crates/omo-app/src/bin/omo-harness.rs`; run it on each platform and record EXIT-6 in `docs/m0/evidence/EXIT-6-<platform>-<date>.md` (expected: incomplete, incomplete, finished)

---

## Phase 9: Polish & Cross-Cutting Concerns

- [x] T062 SPIKE-5: record dependency coexistence (NSH + arrow-ipc in `omo-app`; duckdb only in engine/worker; no duplicate arrow majors) in `docs/m0/evidence/SPIKE-5-2026-11-16.md`
- [x] T063 Fill `docs/m0/acceptance.md` (every N-item × platform × session, SPIKE-1..5, EXIT-1..6) with links; any blank cell is a defect
- [x] T064 Write `docs/m0/decisions/engine-source.md` (FR-005 outcome) and finalise `vendor-provenance/DUCKLING.md`
- [x] T065 [P] Draft the duckomo upstream request text (series regions/nearest, version function, error taxonomy, stale `docs/community-extensions.md`) in `docs/m0/upstream-requests-draft.md`. It is a draft only; whether to file it is OQ-3, and nothing is posted to duckomo
- [x] T066 Run quickstart.md end to end on spark X11; run `/speckit.roadmap.debrief` and update roadmap entry 001 to `in-progress` from the recorded open acceptance work — [quickstart evidence](../../docs/m0/evidence/QUICKSTART-spark-aarch64-x11-20261117.md), [debrief](roadmap-reviews/debrief-20261117T103201Z.md)

---

## Dependencies & Execution Order

- Setup (T001–T006) → Foundational (T007–T014) → user stories.
- **US1** (T015–T020) needs only Setup. It is the MVP and unblocks CI for everything else.
- **US2** needs Foundational. **T023 and T026 are blocked by GATE FR-005**. T021, T022,
  T024, T025, T027 (preparation) and T028 can proceed against fakes or the raw duckdb crate.
- **US3** needs Foundational. T035 needs T023. Everything else uses fake workers. The full
  run (T040) needs US2.
- **US4** needs US3 host pieces (T036–T038) for live data. T041–T046 can start against
  generated IPC files (T039 `gen-result`). T051 is blocked by GATE FR-022.
- **US5** needs US2 (bootstrapped connections) and T030 (spatial mitigation).
- **US6** needs US2 and US3. The UI wiring (T060) needs T046.
- Polish runs after all stories.

## Parallel Opportunities

- T002–T005 in parallel. T007/T008/T010/T012/T014 in parallel.
- US1 and the preparation parts of US2/US3 in parallel once Foundational is done.
- In US4, T042–T046 touch different files and run in parallel. The per-platform runs
  T049/T050 run in parallel with T048 on different hosts.

```text
# Example: after T041
T042 sql_view.rs  ║  T043 table_view.rs  ║  T044 plot_view.rs  ║  T045 tasks_view.rs  ║  T046 dialogs.rs
```

## Implementation Strategy

1. **MVP = US1**: a locked, boundary-enforced workspace that is green on CI.
2. Get the user's FR-005 decision as early as possible. It gates US2/US3 internals.
3. US2 → US3 → US4 (shell decision) form the critical path to the M0 exit. US5 and US6 run
   in parallel once US2/US3 land.
4. Stop at each checkpoint and record evidence. Never auto-start M1.
