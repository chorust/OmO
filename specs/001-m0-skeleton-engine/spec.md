# Feature Specification: M0 Skeleton & Usable Engine (M0 骨架与引擎可用)

**Feature Branch**: `001-m0-skeleton-engine` (spec directory only; no git branch was created)

**Created**: 2026-11-16

**Status**: Draft

**Input**: User description: "First milestone only: M0 骨架与引擎可用 (design §7.1–7.3, including
NSH acceptance N1–N9 and the documented fallback)." Source of truth: omobench design v0.2
(2026-11-15) §7.1–§7.3, with §2–§5 as referenced; constitution v1.0.0; roadmap entry 001.

## Context

M0 is the prerequisite milestone in which **every spike and every go/no-go decision is
closed**. It delivers no end-user forecast features (those are M1). Its "users" are the
omobench maintainer and the reviewer who must decide, on recorded evidence, whether:

1. the workspace and its boundaries are sound;
2. the engine bootstraps duckomo correctly on every target platform;
3. the NSH shell is acceptable, or the documented fallback is triggered;
4. which nearest-series plan and which duckomo distribution channel M1 will use.

Technology names in this spec (NSH, DuckDB, duckomo, GTK/AppKit) are **locked product
constraints** (constitution I–III), not implementation choices this spec makes.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Workspace skeleton with enforced boundaries (Priority: P1)

The maintainer opens the omobench workspace and finds the five planned components: the engine,
the protocol, the worker, the project document model, and the desktop app. All of them build
reproducibly from locked dependencies on every target platform. Automated checks reject any
change that lets a core component depend on a UI framework, lets the desktop app's release
build link the database engine, or lets the app pull in the script runtime (QuickJS) before L2.

**Why this priority**: every later story builds inside this skeleton. The boundary checks
make the §7.3 fallback affordable, because only the UI layer would need to change.

**Independent Test**: Run the locked build and the boundary checks on each platform. Then
deliberately add a forbidden dependency on a scratch branch and confirm the check fails.

**Acceptance Scenarios**:

1. **Given** a clean checkout, **When** the locked build runs on Linux x86_64, Linux aarch64
   and macOS arm64, **Then** all components build and the result is recorded per platform.
2. **Given** a core component that gains a UI-framework dependency (NSH, GPUI or Tauri),
   **When** checks run, **Then** they fail and name the offending dependency path.
3. **Given** the app release build, **When** its dependency tree is inspected, **Then** it
   contains no database engine library, no GPUI, no Tauri and no QuickJS (N9).

---

### User Story 2 - Verified engine bootstrap on every platform (Priority: P1)

The maintainer opens an engine connection on each target platform and the engine:

- verifies the pinned duckomo artifact against its lock (size and SHA-256);
- loads the remote-file capability (httpfs), then duckomo;
- locks the configuration;
- runs the version, platform, extension-version and function probes;
- runs a golden query against a small shipped fixture.

It then shows a bootstrap report with the versions, platform, distribution channel, remote
capability and timings. Both distribution channels (community-signed and GitHub-release-unsigned)
are evaluated, and the result decides the default channel.

**Why this priority**: the product exists only if duckomo loads and answers correctly. Pins
and SHA locks are not evidence of compatibility.

**Independent Test**: A headless run (no UI) on each platform opens bootstrapped connections
for each channel, prints the bootstrap report and golden-query hash, and repeats 100 fresh
connections.

**Acceptance Scenarios**:

1. **Given** a platform and a configured channel, **When** a connection is opened, **Then** the
   reported DuckDB version is v2.0.0, the platform matches the artifact, the duckomo
   extension version equals the locked version exactly, and the required table functions
   exist.
2. **Given** a bootstrapped connection, **When** the golden query runs on the shipped fixture,
   **Then** its result hash equals the locked hash. Both channels produce the same golden
   result on that platform.
3. **Given** an artifact whose size or SHA-256 differs from the lock, **When** bootstrap runs,
   **Then** the connection is refused with a duckomo bootstrap error, and nothing is loaded.
4. **Given** the signed channel is configured and fails to load, **When** bootstrap runs,
   **Then** it fails explicitly and does **not** fall back to the unsigned channel.
5. **Given** httpfs can be neither loaded nor installed within 15 s, **When** bootstrap
   completes, **Then** remote capability is marked unavailable. Local queries still work, and
   any remote task is rejected with a capability-unavailable error.
6. **Given** 100 consecutive fresh connections, **When** each is bootstrapped, **Then** all
   probes pass every time.
7. **Given** a locked configuration, **When** spatial is requested later, **Then** the result
   of trying to load spatial is recorded (works / fails). If the unsigned channel is used,
   the result of trying to close the unsigned window after opening is recorded too (risk R4).
   The chosen mitigation is documented.

---

### User Story 3 - Supervised engine worker (Priority: P1)

The app never runs the database in its own process. Queries go to a supervised worker
process. Large results come back as page-readable result files rather than through the pipe.
The supervisor survives worker crashes, restarts and re-bootstraps the worker, and ignores
anything that arrives late from a previous worker generation. Cancellation escalates from a
soft interrupt, to a bounded drain, to killing the worker.

**Why this priority**: crash isolation and reliable hard cancellation are the reasons the
engine runs out of process. M1's task model depends on them.

**Independent Test**: A headless harness sends queries to a worker, injects kills and slow
queries, and checks recovery, generation filtering and the three cancellation levels. No UI
is needed.

**Acceptance Scenarios**:

1. **Given** a running worker, **When** a query returns a large result, **Then** the worker
   writes the result to a session-scoped file and replies with a result reference (id,
   generation, path, row count, schema hash). The reply is readable page by page.
2. **Given** a worker that is killed mid-task, **When** the supervisor detects it, **Then**
   affected tasks are marked failed (worker crashed), a new worker is started with a higher
   generation and re-bootstraps, and later work succeeds.
3. **Given** a reply or result reference from an old generation, **When** it arrives, **Then** it
   is discarded and its result file is deleted.
4. **Given** a long-running query, **When** it is cancelled, **Then** a soft interrupt is tried
   first, then a drain of at most 10 s, and then the worker is killed, restarted and
   re-bootstrapped. The level that succeeded is recorded.
5. **Given** the worker writes diagnostics, **When** it runs, **Then** only protocol frames
   appear on its standard output and all logs go to its error stream.

---

### User Story 4 - NSH minimal workbench and shell decision (Priority: P1)

The maintainer launches a minimal native workbench built on the locked NSH release. It
has a docked layout and a SQL editor. A query typed in the editor runs in the worker, and its
paged results appear in a virtualised table. A line chart plots a result. A task panel shows
the bootstrap report. Native open and save dialogs pick a station CSV and an export path.
The maintainer runs acceptance items N1–N9 on the platform matrix. With the evidence in hand,
they decide one of three things:

- NSH is accepted.
- A later NSH compatible release is evaluated.
- The documented fallback is triggered: a whole-UI-layer swap to GPUI Kit, which is then
  re-accepted on the same matrix.

Embedding one UI framework in the other is never an option.

**Why this priority**: the shell choice is locked only provisionally. M1 UI work must not
start until this decision is evidenced.

**Independent Test**: On each platform/session in the matrix, launch the minimal workbench
and run N1–N9 scripts/checklists. Record pass/fail plus the measurements.

**Acceptance Scenarios** (matrix: macOS 14+ arm64; Linux x86_64 GTK ≥ 4.14 on Wayland and
X11; Ubuntu 24.04 aarch64 "spark" on Wayland and X11, including building from source):

1. **N1 Build** — **Given** the locked NSH release, **When** built with locked dependencies on
   each platform, **Then** it succeeds. On spark it also builds from source, and the required
   system packages are recorded.
2. **N2 Table** — **Given** a 1,000,000-row × 20-column result, **When** it is scrolled in the
   table, **Then** rows are fetched by visible range, and frame time / main-thread blocking
   and app memory are recorded. A sort request triggers a page re-fetch.
3. **N3 Plot** — **Given** 20 series × 10,000 points, **When** plotted, **Then** the chart is
   interactive. Render time is recorded and the effective series/point limits are set from
   the measurements.
4. **N4 Editor** — **Given** the SQL editor, **When** editing multi-line SQL, **Then**
   highlighting, undo/redo and copy/paste work.
5. **N5 File dialogs** — **Given** open and save dialogs on macOS and Linux (with and without
   a desktop portal), **When** used, **Then** they work. Cancel returns "no selection". A
   portal failure falls back to typing a path manually.
6. **N6 CJK IME** — **Given** Chinese pinyin input (macOS system pinyin; Linux ibus and
   fcitx5, each on Wayland and X11), **When** composing in the editor, text fields (and table
   cell editing if used), **Then** composition, candidate selection and commit work.
7. **N7 Threading** — **Given** the supervisor pushes progress events at 50 Hz for 10 minutes,
   **When** the UI is used meanwhile, including during a worker kill/restart, **Then** the UI
   main thread is never blocked and stays operable.
8. **N8 Memory** — **Given** an empty workbench and the N2 scenario, **When** measured,
   **Then** app memory is recorded. The effect of NSH's per-node memory cost (issue #1345,
   ~2.7 KB/node, open) on trees, headers and task lists is assessed.
9. **N9 Dependencies** — see User Story 1, scenario 3.
10. **Fallback** — **Given** a blocking N1–N8 result, **When** the decision is made, **Then**
    the steps run in order:
    1. Check whether a later NSH compatible **release** fixes it. Only a release, never
       `main`.
    2. Otherwise swap the UI layer as a whole to GPUI Kit (one of `gpui-kit =0.9.1` or
       `gpui =0.2.2 + gpui-component =0.5.0`). Re-run the equivalent acceptance.
    3. Record that L1 survives the swap with a new renderer and that L2 needs re-selection.

    No other part of the system changes.

---

### User Story 5 - Station × nearest-grid series plan (Priority: P2)

For a small set of stations, the maintainer runs both candidate plans for producing
nearest-grid time series across forecast chunks:

- **A**: the host pre-computes the nearest grid index and the engine reads grid slices.
- **B**: per-chunk nearest-region sampling, then the results are unioned and de-duplicated.

Each output is compared with reference data. The comparison checks the spatial and temporal
semantics item by item, and records bytes read and time taken. One plan is chosen. Both
plans emit the stable `station_series` schema.

**Why this priority**: M1's core feature depends on the choice. It is lower than P1 only
because it builds on stories 2–3.

**Independent Test**: A headless run of plan A and plan B over the same stations and window
produces two outputs plus a comparison report against the reference.

**Acceptance Scenarios**:

1. **Given** the same stations and time window, **When** plan A and plan B run, **Then** both
   produce rows in the `station_series` schema (station id, variable, valid time, value, grid
   lat, grid lon, distance in m, source object, object version).
2. **Given** reference data, **When** compared, **Then** the following are each checked and
   recorded:
   - longitude boundary handling and the 0/360 convention;
   - projection/CRS;
   - grid-distance algorithm;
   - missing values;
   - chunk overlap and de-duplication;
   - UTC timestamps.
3. **Given** the comparison and the cost measurements, **When** the decision is made, **Then**
   the chosen plan and the reasons are recorded. If plan A turns out to be infeasible (the
   series function cannot be limited to a few grid points), that finding is recorded.

---

### User Story 6 - Atomic export with crash recovery (Priority: P2)

The maintainer exports a result to a path chosen in the native save dialog. The data file
and its provenance sidecar are written to a hidden temporary directory first. The sidecar
records:

- a job summary;
- the bootstrap report;
- the engine, extension and channel versions;
- the data hash;
- Open-Meteo attribution.

The commit moves the old sidecar aside, then the data file into place, then the new sidecar
into place. The sidecar rename is the only point at which the export counts as saved. If the
app is killed between any two steps, the next startup scan recognises the state correctly.

**Why this priority**: the export protocol is a locked state boundary. Proving it in M0
de-risks M1 export and later report export.

**Independent Test**: A harness kills the exporting process at each commit step, restarts
it, and inspects the startup-scan verdict for each case.

**Acceptance Scenarios**:

1. **Given** an export target that already exists, **When** exporting, **Then** the user
   confirms the overwrite first, and the old sidecar is invalidated before the new data
   replaces it.
2. **Given** a completed export, **When** inspected, **Then** the sidecar exists and its
   recorded hash matches the data file. Only then is it shown as saved.
3. **Given** the process is killed after step 1, after step 2, or after step 3, **When** the app
   starts and scans recent export directories, **Then** finished exports have their leftovers
   cleaned, and unfinished ones are flagged "incomplete export, please re-run".
4. **Given** the temporary directory and the target are on different file systems, **When**
   committing, **Then** the export fails with an explicit error. It does not fall back to
   copy-and-delete.

---

### Edge Cases

- The signed channel works on some platforms but not others. The channel decision is recorded
  **per platform**. The lock keeps both channel entries. No silent mixing happens.
- duckomo bumps on the community site during M0. Community URLs are not versioned. The lock
  stores the downloaded file's hash and source metadata, and a hash mismatch blocks bootstrap.
- After the config is locked, spatial cannot be loaded. The R4 mitigation (pre-load spatial
  or start a spatial-capable worker) is chosen and recorded.
- spark has no GTK4 development packages (2026-11-16). The required packages are recorded as
  a build prerequisite. The spec does not assume they are present.
- The installed Rust toolchain is older than the locked crates' declared minimum (NSH 0.5.1
  declares 1.97; spark has 1.96.0). The toolchain pin must satisfy it.
- No desktop portal is available on Linux. The file dialog falls back to manual path entry.
- A Wayland or X11 session is unavailable on a test host. The N-item is marked "not run" with
  the reason. It is never marked pass.
- The worker hangs without exiting. Missed heartbeats trigger level-3 cancellation.
- An unsigned extension window cannot be closed after opening. This is recorded as an R4
  finding, and the unsigned channel's risk is noted in the channel decision.

## Requirements *(mandatory)*

### Functional Requirements

**Workspace & boundaries**

- **FR-001**: The workspace MUST contain the five components (engine, protocol, worker,
  project, app). It MUST also contain the duckomo artifact lock, the catalog placeholder, the
  provenance record location and docs (design §2.1).
- **FR-002**: Builds MUST be reproducible from committed lock files on Linux x86_64, Linux
  aarch64 and macOS arm64, and continuous integration MUST run them.
- **FR-003**: Automated checks MUST fail if engine, protocol, worker or project depend on any
  UI framework. They MUST also fail if the app's release build includes the database engine,
  GPUI, Tauri or (before L2) QuickJS.
- **FR-004**: The app MUST be organised so that NSH-specific code is confined to a UI layer
  and host logic has no NSH dependency.

**Engine extraction & provenance**

- **FR-005**: Engine code reused from Duckling MUST come from a committed Duckling commit
  or, for feature 001 only, the user-approved content-addressed snapshot of selected
  working-tree files. The repository MUST retain the exact captured source bytes and record
  Duckling HEAD, capture date, per-file source hashes, source-to-destination mapping, and
  trimming/adaptation notes in `vendor-provenance/DUCKLING.md`. Any future working-tree
  snapshot requires separate explicit user approval. No git or path dependency on Duckling may
  exist.
  **Resolution (2026-11-16)**: the user approved the hashed working-tree snapshot route; exact
  source capture and import remain implementation work.

- **FR-006**: All bootstrap paths (data dir, extension cache dir, duckomo artifact including
  channel, platform) MUST be passed as explicit configuration and never discovered from
  another app's data directories.

**Bootstrap**

- **FR-007**: Each new connection MUST run the fixed bootstrap order:
  1. the unsigned window (only for the unsigned channel) and httpfs (15 s timeout,
     degrade-to-local);
  2. duckomo after size/SHA-256 verification;
  3. config lock;
  4. the probes;
  5. the golden query.

  Any failure in steps 2–5 MUST make the connection unusable.
- **FR-008**: The probes MUST check that the DuckDB version is exactly v2.0.0, that the
  platform equals the artifact's, that the duckomo version equals the lock exactly, and that
  `read_om`, `read_om_series`, `om_grid_info` and `duckomo_last_scan_metrics` exist. The
  function probe MUST also run before every task, and a failure MUST discard and re-bootstrap
  the connection.
- **FR-009**: Only bootstrapped connections may run OM queries. The system MUST make it
  impossible to run them on a non-bootstrapped connection.
- **FR-010**: The channel MUST be chosen explicitly by configuration. A signed-channel failure
  MUST NOT fall back to unsigned.
- **FR-011**: The artifact lock MUST hold entries for both channels. Each entry MUST record
  channel, duckomo version, DuckDB version, platform, SHA-256, size and source (URL / PR / ref
  / download time). A download-and-verify procedure MUST exist for each channel.
- **FR-012**: The bootstrap report MUST include the versions, platform, extension version,
  channel, httpfs state and timings. It MUST be shown in the task panel and included in
  export sidecars.
- **FR-013**: Spatial MUST be loadable on demand only for region/map work. The behaviour
  after config lock MUST be measured and the mitigation recorded.

**Worker & protocol**

- **FR-014**: The engine MUST run in supervised worker processes. Messages MUST be
  length-prefixed frames on stdout and logs MUST go to stderr. Every message MUST carry
  worker generation, job id and request id.
- **FR-015**: Large results MUST be delivered as page-readable files referenced by id,
  generation, path, rows and schema hash. Results MUST be written batch-wise, never collected
  into one batch. Interactive queries MUST enforce hard row and byte limits and label any
  truncation.
- **FR-016**: The supervisor MUST detect a worker crash or hang by heartbeat, mark the
  affected tasks failed, restart the worker with generation + 1 and re-bootstrap it. It MUST
  discard stale-generation messages and their files.
- **FR-017**: Cancellation MUST proceed in three levels (interrupt → drain ≤ 10 s → kill +
  restart + re-bootstrap). On app exit it MUST skip straight to kill and remove uncommitted
  result files.

**Minimal workbench (NSH)**

- **FR-018**: The minimal workbench MUST provide a docked layout, a SQL editor, a paged
  virtualised result table, a line chart, a task panel with the bootstrap report, and native
  open/save dialogs with a manual-path fallback.
- **FR-019**: Only the UI engine thread may update UI state, and only after checking
  generation and revision. The UI main thread MUST NOT perform IO, decoding or worker waits.
- **FR-020**: The chart MUST enforce a series/point cap, with downsampling labelled on the
  chart. The cap value MUST come from the N3 measurements (initial default ≤ 20 series ×
  ≤ 10,000 points per series).
- **FR-021**: N1–N9 MUST be executed on the reference host (`lizhan-spark`) with results and
  measurements recorded per platform and session. Under Constitution 1.3.0 (2026-11-21),
  non-reference platforms (`linux_amd64`, `osx_arm64`, plus build-only `macos-15-intel`) are
  verified and published via GitHub Actions (`ci.yml` + `release.yml`, including native
  Duckomo bootstrap and live-worker smoke via `scripts/ci-verify-duckomo.py` on locked
  platforms); items without a local interactive desktop session remain explicitly recorded as
  "not run" with the reason, and dedicated hardware is engaged only if CI or release execution
  exposes an issue.
- **FR-022**: The shell decision (accept NSH / evaluate a compatible release / trigger the
  GPUI Kit fallback) MUST be recorded with the evidence behind it. Embedding NSH and GPUI in
  each other MUST NOT be offered. N2/N3 workload sizes and N6 coverage remain as specified.
  The user approved the research D8 blocking examples on 2026-11-16: spark build failure, an
  unusable N6 IME path under GTK with no viable fix, or an unusable million-row N2 table are
  blocking examples. Other results are recorded as limitations. N7 MUST record maximum
  main-thread stall and whether the UI remains operable during the 10-minute run; no numeric
  stall threshold is imposed in M0.

**Nearest-series spike**

- **FR-023**: Plans A and B MUST both be run on the same stations and window. Both MUST emit
  the stable `station_series` schema and be compared with reference data on each listed
  semantic item, with bytes read and time recorded. The choice MUST be recorded.

**Export**

- **FR-024**: Export MUST stage in a same-filesystem hidden temp directory. It MUST then
  commit in this order: old sidecar aside → data → sidecar, then fsync the directory. Saved
  status MUST be granted only when the sidecar exists and its hash matches the data.
- **FR-025**: A startup scan MUST clean finished leftovers and flag unfinished exports.
- **FR-026**: Export MUST refuse cross-filesystem commits rather than copy.

**Evidence**

- **FR-027**: Every M0 exit condition MUST have a recorded evidence entry with the command,
  host/platform, date, versions and the output or artifact path. Pins and hashes alone MUST
  NOT count as compatibility evidence.

### Key Entities

- **Workspace component**: one of engine / protocol / worker / project / app. Its
  allowed-dependency class is UI-free core or UI app.
- **Artifact lock entry**: channel, duckomo version, DuckDB version, platform, SHA-256, size,
  source metadata, golden-query hash.
- **Bootstrap configuration**: data dir, extension cache dir, artifact entry, platform,
  threads/memory settings.
- **Bootstrapped connection**: a connection plus its capabilities (remote, spatial), probe
  report and generation.
- **Bootstrap report**: versions, platform, extension version, channel, httpfs state, timings.
- **Worker generation**: a monotonically increasing id per worker start.
- **Result reference**: id, generation, file path, rows, schema hash.
- **Job / task state**: Queued → Bootstrapping → Planning → Reading → Writing → Committing →
  Saved | Failed(kind) | Cancelled.
- **Station series row**: the stable schema from FR-023.
- **Export bundle**: data file plus sidecar (job summary, bootstrap report, versions/channel,
  data hash, attribution).
- **Acceptance record**: an N-item or exit condition, the platform/session, the result
  (pass / fail / not run), measurements and evidence path.
- **Provenance record**: Duckling commit, path mapping, trimming notes, cherry-picks.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On the reference host (`linux_arm64`), duckomo loads and the golden query matches
  the locked hash in 100% of 100 consecutive fresh connections across both channels; on
  `linux_amd64` and `osx_arm64`, GitHub Actions CI (`scripts/ci-verify-duckomo.py`) verifies
  locked bootstrap, golden-hash match, and live worker execution on native runners (Constitution
  1.3.0).
- **SC-002**: For each platform where both channels load, the two channels' golden results
  are identical. The default channel decision is recorded with evidence.
- **SC-003**: Nearest-series output from the chosen plan matches the reference data on every
  listed semantic item for the test stations. The cost (bytes read, duration) of both plans
  is recorded.
- **SC-004**: In 100% of injected worker kills, the system recovers without a user restart,
  and 0 stale-generation results are ever shown.
- **SC-005**: In 100% of kill injections at each export commit step (3 positions), the
  startup scan gives the correct verdict (saved vs incomplete), and nothing unsaved is ever
  shown as saved.
- **SC-006**: N1–N9 each have a recorded result on the reference host (`lizhan-spark`), and
  every non-reference platform/session cell records either its local measurement or its
  GitHub-CI-gated `not-run` scope per Constitution 1.3.0. Either the applicable gates pass, or
  the fallback decision is recorded.
- **SC-007**: During a 10-minute 50 Hz progress stream and a worker restart, the UI stays
  responsive, with the maximum main-thread stall and responsiveness outcome recorded. The
  user-approved FR-022 rule imposes no numeric stall threshold; any N7 limitation is recorded
  and classified under that rule.
- **SC-008**: The boundary checks reject 100% of seeded forbidden-dependency changes (UI
  framework in a core component; database, GPUI, Tauri or QuickJS in the app release build).

## Assumptions

- Test hardware for macOS 14+ arm64 and Linux x86_64 (with both Wayland and X11 sessions)
  will be made available. Only spark (aarch64, X11 session observed) is confirmed on
  2026-11-16 (roadmap OQ-B3).
- Installing build prerequisites on test hosts (GTK4/GStreamer dev packages, a Rust
  toolchain that meets MSRV) is acceptable during implementation. Nothing was installed
  during specification.
- The reference data for the nearest-series comparison will come from Duckling `specs/008`
  fixture revision 2 or a duckomo test reference CSV, as the design says. Whether those are
  committed has not been checked (roadmap OQ-B2). If they are unavailable, a reference is
  produced independently and documented.
- A small OM fixture for the golden query can be shipped with the app. Its provenance and
  licence will be recorded.
- Network access to the community extension host and GitHub Releases is available during M0
  for downloading artifacts. At runtime, artifacts are loaded from verified local files.
- The design §10 open questions remain open except Q8e (FR-022), resolved by the user on
  2026-11-16 using research D8, and blocker OQ-B1 (FR-005), resolved by approval of the
  content-addressed snapshot route on 2026-11-16.
- Out of scope for M0:
  - M1 features (full station import UX, catalog readiness, product defaults);
  - M2/M3;
  - L1/L2 (including the L1 isolation pre-check and the L2 pre-spike);
  - Windows, Wasm, and Intel Mac runtime acceptance / Duckomo artifact locking (note:
    Constitution 1.2.0 authorizes build-only `macos-15-intel` coverage in
    `.github/workflows/ci.yml` and published-release archive builds in
    `.github/workflows/release.yml` without implying Intel Mac runtime acceptance);
  - any change to Duckling or duckomo;
  - creating or renaming any repository.
