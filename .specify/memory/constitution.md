<!--
SYNC IMPACT REPORT
==================
Version change: (unversioned stock template) → 1.0.0 → 1.1.0 → 1.2.0 → 1.3.0
Bump rationale: MAJOR (initial ratification) — the stock Spec Kit placeholder template is
replaced by the first real omobench constitution; MINOR (1.1.0) amends Principle VII for the
user-approved, content-addressed Duckling snapshot selected for feature 001; MINOR (1.2.0)
records the user-approved addition of `macos-15-intel` (`x86_64-apple-darwin`) build/test CI
and published-release archive builds (`.github/workflows/release.yml`) as build-only coverage
without expanding M0 runtime acceptance or `duckomo.lock.json` platform scope; MINOR (1.3.0)
records the user-approved cross-platform release & CI policy: Spark (`linux_arm64`) carries
full interactive GUI acceptance while other target platforms (`linux_amd64`, `osx_arm64`, and
build-only `macos-15-intel`) are verified and published directly via GitHub Actions CI
(`ci.yml`, including native Duckomo bootstrap & live-worker smoke on locked platforms) and
`release.yml`, engaging dedicated target hardware only if CI or release execution fails.

Principles (template slot → new title):
  - [PRINCIPLE_1_NAME] → I. UI-Free Core Crates
  - [PRINCIPLE_2_NAME] → II. One Native Shell: NSH Pure Rust, Exactly Pinned
  - [PRINCIPLE_3_NAME] → III. Pinned Engine, Per-Connection Verified Bootstrap
  - [PRINCIPLE_4_NAME] → IV. Supervised Engine Process and Explicit State Boundaries
  - [PRINCIPLE_5_NAME] → V. Staged, Gated Dynamic Reports
  - (added)            → VI. Evidence Before Claims
  - (added)            → VII. Hands Off Neighbouring Repositories
  - Principle VII amended for feature 001: approved snapshot source bytes and per-file hashes
    are retained with path mapping and transformation notes.

Added sections:
  - Technology & Platform Constraints (template SECTION_2)
  - Development Workflow & Quality Gates (template SECTION_3)
  - Governance (filled)

Removed sections: none (all template slots filled; example comments removed).

Templates reviewed (read-only; this command does not edit them):
  ✅ .specify/templates/plan-template.md — "Constitution Check" is filled per feature from this file.
  ✅ .specify/templates/spec-template.md — no change needed.
  ✅ .specify/templates/tasks-template.md — no change needed; tests are mandatory for
     principle-bearing code by Principle VI / Workflow, which features state in their spec.

Deferred / follow-up TODOs:
  - Design §10 open questions Q1–Q8 (incl. 8a–8e) remain OPEN; recorded in
    .specify/memory/roadmap.md, not answered here.
  - BLOCKER B-1 (found 2026-11-16): the Duckling extraction sources named in design §3.2
    are not in any local Duckling commit. On 2026-11-16 the user approved a narrow Principle
    VII amendment for feature 001, allowing a content-addressed working-tree snapshot of the
    selected files. Capture and retain their bytes, hashes, mapping, and trimming notes. See
    roadmap OQ-B1.
  - Repo naming: design §2.1 proposes a repo named `omobench`; work currently happens in
    `chorust/OmO`. Final name/org is design §10 Q1 (open). No repo is created or renamed.

Sources: /home/blizhan/tmp/gpui-duckomo-studio-design/docs/design.md (v0.2, 2026-11-15),
STATUS.md, revision-input/DISCUSSION.md.
-->

# omobench Constitution

omobench (descriptive alias: `duckomo-workbench`, "a duckomo forecast workbench") is a native
desktop workbench for **forecasts × user station tables**. It reads Open-Meteo OM data via the
`duckomo` DuckDB extension, uses DuckDB engine code extracted one-way from Duckling, and renders
its UI with an NSH (Native Shell, Longbridge) pure-Rust application. It is a parallel product
line: it does not replace or modify Duckling or the duckomo extension. It is not "DuckOMO
Benchmark" (a separate 1.5.6 vs 2.0.0 performance comparison) and not "Tonust" (a historical
name). The workspace currently lives in `chorust/OmO`.

The canonical design is omobench design v0.2 (2026-11-15). Where this constitution and the
design disagree, this constitution governs and the disagreement MUST be raised as an amendment.

## Core Principles

### I. UI-Free Core Crates

- The workspace is split into `omo-engine`, `omo-protocol`, `omo-worker`, `omo-project` (the
  four **core crates**) and `omo-app` (the only UI binary).
- The four core crates MUST NOT depend, directly or transitively, on any UI framework:
  no `nsh-*`, `gpui*`, or `tauri*`. CI MUST enforce this with `cargo tree -p <crate>` on every
  change.
- `omo-app` is split into `omo-app::host`, which MUST NOT import `nsh_*`, and `omo-app::ui`,
  the only code that touches NSH. The `ui` layer is the only part replaced if the shell changes.
- The release build of `omo-app` MUST NOT link DuckDB (`duckdb`, `libduckdb-sys`). The
  in-process engine implementation may be used only behind an `omo-app` test feature.

Rationale: the shell is young and fast-moving, so a shell change, if the fallback in
Principle II is ever needed, must cost only `omo-app::ui`. DuckDB and native-extension crashes
must stay out of the UI process.

### II. One Native Shell: NSH Pure Rust, Exactly Pinned

- The main shell is an NSH **pure-Rust** application. All `nsh-*` crates MUST be pinned exactly
  (`=0.5.1` until a ratified amendment changes the pin). `Cargo.lock` is committed and every
  build and CI job uses `--locked`.
- Embedding MUST NOT be used: no NSH inside GPUI (`nsh-platform-gpui` AppView) and no GPUI
  inside NSH. `nsh-platform-gpui` and `gpui*` MUST NOT appear in the dependency tree.
- The shell MUST NOT return to Tauri.
- Pin upgrades are allowed only to a published NSH **release** that NSH lists as compatible,
  never to `main`. Each upgrade needs evidence (Principle VI) and an amendment of the pin.
- The only sanctioned fallback is design §7.3. If the M0 NSH acceptance (N1–N8) finds a
  blocker, the whole `omo-app::ui` layer moves to GPUI Kit. That fallback MUST be re-accepted
  on the same platform matrix. It MUST NOT start without recorded evidence and the user's
  decision.
- Until L2 (Principle V), `omo-app` MUST NOT link QuickJS or TSX tooling. NSH platform crates
  enable `js` by default, as checked on crates.io on 2026-11-16, so default features MUST be
  disabled and features MUST be selected explicitly.

Rationale: NSH was chosen for its governed dynamic-report runtime. Two UI runtimes doubles the
IME, focus, and event-loop debugging surface. Exact pins contain churn from a framework that
was created on 2026-10-14 and reached v0.5.1 on 2026-11-14.

### III. Pinned Engine, Per-Connection Verified Bootstrap

- The engine is pinned to DuckDB **v2.0.0** and duckdb-rs **`=1.20000.1`**. It is not upgraded
  or downgraded without an amendment.
- The duckomo extension is pinned to **v0.4.0** in `artifacts/duckomo.lock.json`. Each entry is
  keyed by channel × version × DuckDB version × platform and records SHA-256, size, and source
  (URL / PR / ref / download time). The two channels are `community-signed` and
  `github-release-unsigned`. The channel is chosen explicitly in config, and the runtime
  MUST NOT silently fall back from signed to unsigned.
- "Loaded" is a property of one connection handle, never global app state. Every new
  connection, clone, worker restart, or TTL rebuild MUST go through `open_bootstrapped`.
  The order is fixed: unsigned window (if any) / httpfs → duckomo → spatial only when needed.
  Then the config is locked.
- Bootstrap succeeds only when all probes pass: DuckDB `version()`, `PRAGMA platform`,
  duckomo `extension_version` exactly equal to the lock, the required functions present, and a
  golden query on a shipped fixture whose result hash matches the lock. The function probe is
  repeated before every task. A failure discards the connection.
- Engine APIs that run OM queries MUST accept only a bootstrapped-connection type.
- When httpfs is unavailable, the app degrades only to local SQL and reports it. Remote tasks
  MUST be rejected with an explicit capability error and MUST NOT degrade silently.

Rationale: extension LOAD state, signing, and version drift are the main correctness risks, and
function existence does not prove correct behaviour.

### IV. Supervised Engine Process and Explicit State Boundaries

- DuckDB and its extensions run only in supervised `omo-worker` child processes. stdout carries
  only length-prefixed protocol frames, and all logs go to stderr. Every message carries
  `(worker_generation, job_id, request_id)`. Messages from stale generations, and the result
  files they reference, are discarded.
- Large results travel as Arrow IPC files referenced by `ResultRef`, not through the pipe. UI
  surfaces hold only visible pages. Interactive queries have hard row and byte limits, and a
  truncation is labelled as truncated.
- Cancellation has three levels: soft interrupt, then up to 10 s drain, then kill the worker,
  restart it, and re-bootstrap. App exit goes straight to kill.
- Labelize-style state boundaries are mandatory:
  - Previews are bound to a full `PreviewKey`. A late or mismatched result is dropped.
  - Jobs snapshot the document at submit time.
  - `edited`, `draft_staged`, `received`, and `saved` are distinct states and are shown
    distinctly. `received` is never shown as saved.
  - Drafts never overwrite project files.
  - A failure keeps the last good preview, marked stale.
- In the UI process, only the NSH engine thread writes signals, and only after checking
  generation and revision. The UI main thread does no IO, decoding, or waiting on workers.
- Exports are written to a temporary location first. The **sidecar rename is the single commit
  point**. A cross-filesystem rename (EXDEV) is an error, never a copy fallback. A startup scan
  recovers or flags an interrupted export.

Rationale: crash isolation, reliable hard cancellation, and honest "is it saved" semantics.

### V. Staged, Gated Dynamic Reports

- **L0 (M0–M1)**: a fixed, hand-written workbench. No TSX, no QuickJS, no Agent-generated pages.
  It is not "YAML generates the whole workbench". L0 only guarantees the stable
  `station_series` schema (`station_id, variable, valid_time, value, grid_lat, grid_lon,
  distance_m, source_object, object_version`) and result snapshots that later reports can
  reference.
- **L1**: a versioned `ReportSpec` rendered by allow-listed, pre-authorised Rust components.
  Reports read only a frozen, read-only snapshot through an isolated report worker. L1 MUST NOT
  be enabled until the report-worker isolation tests pass. Those tests reject privileged
  statements, arbitrary file reads, network URLs, and extension install/load, and confirm that
  budget cancellation works. Unknown or over-limit specs are rejected, not rendered on a
  best-effort basis.
- **L2**: signed `.nshpkg` packages only, trusted only from configured omobench report signing
  roots. Because NSH capabilities are **not deny-by-default**, the host MUST tighten them
  explicitly to a minimal allow-list and refuse any package that declares more. Packages carry
  no native code. The host API is read-only `omo.*`. Resource limits are tighter than NSH
  defaults. L2 needs its own pre-spike to pass first.
- Declarative (L1) comes first. TSX (L2) is used only for needs L1 provably cannot express.

Rationale: AI-generated report UI is the reason NSH was chosen, and it must arrive in stages
with explicit trust boundaries. NSH sandboxing is application-level, not OS isolation.

### VI. Evidence Before Claims

- PASS, verified, supported, or working MAY be claimed only with evidence recorded in the repo:
  command, platform/host, date, versions, and output or artifact path. Otherwise the claim cites
  an existing source and states its limits.
- Version pins, SHA-256 locks, docs, and HTTP HEAD 200 responses are **not** compatibility
  evidence. For example, community duckomo 0.4.0 artifacts were present (HEAD 200) on
  linux_amd64, linux_arm64, and osx_arm64 as of 2026-11-16, but LOAD was **not** tested.
- Unverified facts MUST be labelled as unverified in specs, plans, and code comments. Unknowns
  become open questions, not guesses.
- Every milestone exit criterion MUST have a recorded measurement or test result, or an
  explicit documented fallback decision.

Rationale: the design rests on read-only research, and many key facts are still pending M0.

### VII. Hands Off Neighbouring Repositories

- omobench MUST NOT write to, commit to, push to, or open PRs against Duckling
  (`~/repo/github/duckling`) or duckomo (`chorust/duckomo`, `~/repo/github/duckomo`). It also
  MUST NOT put UI code into either.
- Duckling engine code is imported **once, one-way**, from a committed Duckling commit or,
  for feature 001 only, the user-approved content-addressed snapshot. Any future working-tree
  snapshot requires separate explicit user approval. For the approved snapshot,
  `vendor-provenance/DUCKLING.md` records the repository, HEAD, capture date, exact source file
  hashes, source-to-destination mapping, and trimming/adaptation notes; captured bytes are
  retained under `vendor-provenance/snapshots/`. Later fixes are manually imported and recorded
  there. There is no git/path dependency on Duckling's `connector`.
- duckomo is consumed only as published `.duckdb_extension` artifacts. Interface gaps go to
  duckomo as requests, and its maintainers decide.
- No GitHub repository is created or renamed without the user's explicit decision.
- Labelize Studio contributes patterns only. No code is copied.

Rationale: other product lines continue independently, and provenance keeps extracted code
auditable.

## Technology & Platform Constraints

- **Language/toolchain**: Rust, with `rust-toolchain.toml` pinned. The toolchain MUST meet the
  MSRV declared by the pinned crates. `nsh-* 0.5.1` declares `rust-version = 1.97` (crates.io,
  2026-11-16). The pin is chosen and recorded in M0.
- **Workspace**: `resolver = "2"`, a committed `Cargo.lock`, and `--locked` everywhere.
- **Platforms**: macOS 14+ arm64 (AppKit), Linux glibc x86_64 and aarch64 (GTK ≥ 4.14, both
  Wayland and X11). The reference aarch64 host is `lizhan-spark` (Ubuntu 24.04.4).
- **Build-only CI & release coverage (1.2.0 amendment, 2026-11-21)**: Per user approval,
  `.github/workflows/ci.yml` also builds and tests `macos-15-intel` (`x86_64-apple-darwin`),
  and `.github/workflows/release.yml` packages locked release archives (`omo-app`, `omo-worker`)
  for Linux x86_64/aarch64 and macOS arm64/x86_64 on published GitHub Releases. Intel Mac
  coverage is build-only and does NOT expand M0 runtime acceptance or `duckomo.lock.json`.
- **Out of scope**: Windows, Wasm/Web, mobile, TUI, Intel Mac runtime support and Duckomo
  locking (pending design §10 Q5; build-only CI/release assets authorized by 1.2.0),
  SoftTools/DuckStudio, a general-purpose SQL IDE, a global field browser, and
  ensabc/trading-model features.
- **Engine-side computation** stays in Rust/DuckDB extensions, with duckomo as the data source.
  Report modules live only in the UI host. DuckDB extensions and app/UI modules are separate
  layers.
- **Planned layout** (design §2.1): `crates/{omo-engine,omo-protocol,omo-worker,omo-project,
  omo-app}`, `artifacts/duckomo.lock.json`, `catalog/products.json`,
  `vendor-provenance/DUCKLING.md`, `docs/`.

## Development Workflow & Quality Gates

- Spec Kit flow per milestone: constitution → roadmap → specify → plan → tasks → implement →
  review. Implementation starts only after the user asks for it. The next milestone does not
  start automatically.
- Agents MUST NOT commit or push unless the user explicitly asks.
- Every plan MUST include a Constitution Check against Principles I–VII. Any violation needs a
  Complexity Tracking justification or a constitution amendment.
- Required CI on every change: Linux x86_64, Linux arm64, and macOS arm64 builds with
  `--locked` (plus build-only `macos-15-intel` coverage per 1.2.0), the core-crate UI-dependency
  check (Principle I), the `omo-app::host` no-NSH check, the `omo-app` release no-DuckDB check,
  and native locked Duckomo bootstrap + live-worker verification (`scripts/ci-verify-duckomo.py`)
  on `linux_amd64`, `linux_arm64`, and `osx_arm64`. The no-QuickJS check also runs until L2.
  Published GitHub Releases additionally run `.github/workflows/release.yml` to build and attach
  locked release binary archives.
- **Cross-platform release & acceptance policy (1.3.0 amendment, 2026-11-21)**: Per user
  approval, full interactive desktop/GUI acceptance (N2–N8) is executed on the reference host
  (`lizhan-spark`, `linux_arm64`), while other target platforms (`linux_amd64`, `osx_arm64`,
  plus build-only `macos-15-intel`) are verified and published directly through GitHub Actions
  (`ci.yml` + `release.yml`). Passing CI/release on those platforms is accepted as cross-platform
  readiness; dedicated non-reference machines are required only if CI or release execution
  exposes a platform-specific issue.
- Tests are required for:
  - bootstrap probes and the golden query;
  - protocol framing and generation discard;
  - the cancellation levels;
  - export commit and recovery;
  - preview staleness.
  Fault injection (worker kill, export kill at each commit step, no network) is part of
  milestone acceptance.
- Milestone exits (M0–M3, L1/L2 pre-checks) are recorded with evidence per Principle VI.

## Governance

- This constitution supersedes ad-hoc practices in this repository. Specs, plans, tasks, and
  reviews MUST verify compliance. Reviewers cite principle numbers when they flag a violation.
- **Amendments**: proposed in writing with the reason, the affected principles, and a migration
  note. The user approves them. The Sync Impact Report records them.
  Changing a locked decision from design §9 (shell, engine pins, crate boundaries, bootstrap
  order, state boundaries, report staging, repo boundaries) requires the user's explicit
  approval.
- **Versioning**: semantic versioning. MAJOR means removing or redefining a principle, or
  reversing a locked decision. MINOR means a new principle or section, or materially expanded
  guidance. PATCH means clarifications and wording changes.
- **Compliance review**: at every `/speckit.plan` (Constitution Check, gated) and at every
  milestone exit review.
- Runtime guidance and open questions are tracked in `.specify/memory/roadmap.md`.

**Version**: 1.3.0 | **Ratified**: 2026-11-16 | **Last Amended**: 2026-11-21
