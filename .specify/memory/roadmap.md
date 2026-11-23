<!--
SYNC IMPACT REPORT (1.0.5, 2026-11-23)
=====================================
Version change: 1.0.4 → 1.0.5
Bump rationale: PATCH — record user decisions closing M0 non-reference gates with GitHub CI
evidence, resolving OQ-4 (channel policy) and OQ-6 (M1 default product/variable/window).

Changes this revision:
  - Status: 001 in-progress → verified. Non-reference cells are closed by CI run 45104328529
    and release run 45031395194 (docs/m0/evidence/CI-CROSS-PLATFORM-20261123.md). The user's
    FR-022 call accepts NSH 0.5.1; N6 Wayland IBus candidate display is a non-blocking
    limitation.
  - Status: 002 needs-info → planned (OQ-6 resolved).
  - C-06 updated with cross-platform CI channel outcomes and the global channel policy.
  - OQ-4 and OQ-6 resolved.

Specs affected: 001, 002
Open questions added/resolved: OQ-4 and OQ-6 resolved; none added.
-->

<!--
SYNC IMPACT REPORT (1.0.4)
==================
Version change: 1.0.3 → 1.0.4
Bump rationale: PATCH — record Constitution 1.3.0 cross-platform GitHub CI/release acceptance
policy (non-reference platforms validated via `ci.yml` + `release.yml`, dedicated hardware
engaged only on CI/release failure).

Previous revisions: 1.0.0 created the roadmap; 1.0.1 moved 001 from planned to specced.

Changes this revision:
  - Status: 001 specced → in-progress; implementation and evidence exist, while acceptance work remains open.
  - C-06 updated with measured Linux arm64 channel outcomes and remaining platform limits.
  - C-09 / OQ-B1 updated to record the approved feature-001 content-addressed snapshot route.
  - OQ-B2 updated with the retained independent station-series oracle.
  - OQ-8e updated with the user's approved FR-022 D8 blocking examples.

Specs affected: 001
Open questions added/resolved: OQ-B1, OQ-B2, and OQ-8e resolved for feature 001; none added.

Notes:
  - Evidence: docs/m0/decisions/engine-source.md, docs/m0/decisions/channel.md,
    docs/m0/evidence/EXIT-1-spark-aarch64-n-a-20261117.md,
    docs/m0/evidence/EXIT-2-spark-aarch64-20261117.md, and docs/m0/acceptance.md.
  - OQ-4 and OQ-B3 remain open. This amendment records the current evidence and does not
    claim the M0 exit criteria are complete.
  - The initial creation and 1.0.1 status-transition history remains noted above in the
    prior revision summary; the original design and historical blocker facts remain below
    with their current resolutions annotated.
-->

# omobench — Spec Roadmap

This is a living, non-binding map of the specs planned for omobench, the NSH × duckomo forecast
workbench in `chorust/OmO`. It is **not a commitment to order or scope**. It records
decisions, outcomes, and constraints from the design and constitution phases so they are not
lost before the spec that needs them is written. Foundations: the project
[constitution](constitution.md) and design v0.2 (2026-11-15, external:
`/home/blizhan/tmp/gpui-duckomo-studio-design/docs/design.md`). No PRD exists.

Status legend (lifecycle): **undecided** · **needs-info** · **planned** ·
**specced** · **in-progress** · **implemented** · **verified** · **deferred** ·
**abandoned**.

---

## Vision & End States

- A native desktop workbench (macOS 14+ arm64, Linux glibc x86_64/aarch64) where a user
  imports a station table and gets nearest-grid forecast time series from Open-Meteo OM data
  via duckomo. The user can browse, query (SQL), chart, and export them with provenance, and
  never mistakes a received result for a saved one.
- An engine (`omo-engine/-protocol/-worker/-project`) that runs headless and does not depend
  on any UI framework. Its DuckDB/duckomo bootstrap is verified per connection and pinned to
  exact versions.
- Later, governed dynamic reports: declarative `ReportSpec` (L1), then signed TSX packages
  (L2), built on frozen read-only snapshots, so AI/Agent-generated report UI can arrive without
  shipping a new app build.
- Later: a static regional map (M2) and Dynamical Icechunk as a second source (M3), both
  producing the same `station_series` schema.

## Constraints & Decisions

- **C-01 — Product identity:** The product is **omobench**, alias `duckomo-workbench`. It is
  not DuckOMO Benchmark and not Tonust. The workspace is `chorust/OmO`; the final repo name
  and org are OQ-1. (Design §2.3)
- **C-02 — Shell:** NSH pure Rust, `nsh-* =0.5.1`. No NSH↔GPUI embedding and no Tauri. The
  only fallback is a whole-UI-layer GPUI Kit swap after a recorded M0 blocker plus the user's
  decision. (Constitution II; design §5.1, §7.3)
- **C-03 — Core crates are UI-free:** `omo-engine`, `omo-protocol`, `omo-worker`, and
  `omo-project` have no `nsh-*`, `gpui*`, or `tauri*` dependencies, enforced in CI. The
  `omo-app` release build does not link DuckDB. (Constitution I)
- **C-04 — Engine pins:** DuckDB v2.0.0 / duckdb-rs `=1.20000.1`; duckomo v0.4.0 is locked
  by SHA-256 per channel × platform. (Constitution III)
- **C-05 — Per-connection bootstrap:** unsigned/httpfs → duckomo → spatial on demand → lock
  config → probes + golden query. LOAD does not persist across connections. There is no silent
  signed→unsigned fallback. (Design §4.3)
- **C-06 — Two distribution channels, chosen in M0:** `community-signed` (preferred) vs
  `github-release-unsigned`. Facts as of 2026-11-16:
  - The community v2.0.0 artifact returns HEAD 200 on linux_amd64, linux_arm64, and
    osx_arm64, and 404 on osx_amd64.
  - The chorust/duckomo GitHub Release v0.4.0 (published 2026-11-09) has `duckdb-v2.0.0`
    builds for linux_amd64/arm64 and osx_amd64/arm64, plus `SHA256SUMS`.
  - ~~**Neither has been LOAD-tested in this project.** (Design §4.1)~~
  - **M0 evidence (2026-11-17):** On spark, community-signed failed both tested local loading
    routes. GitHub-release-unsigned passed locked bootstrap and the independent golden query
    in 100/100 fresh connections; no unsigned fallback followed the signed failure. Linux
    x86_64/macOS arm64 channel runs and cross-channel equality remain not-run. See
    `docs/m0/decisions/channel.md`; ~~global channel preference remains OQ-4.~~
  - **CI evidence (2026-11-23):** In CI run 45104328529, both channels passed locked SHA-256,
    bootstrap, and golden checks 3/3 on linux_amd64, linux_arm64, and osx_arm64, with identical
    golden hashes. **Policy (OQ-4 resolved):** `community-signed` is preferred;
    `github-release-unsigned` stays allowed by explicit selection; no silent fallback.
- **C-07 — Supervised worker + state boundaries:** child-process engine, generations,
  Arrow IPC `ResultRef`, three-level cancellation, Labelize-style
  `edited`/`draft_staged`/`received`/`saved`, and an export whose single commit point is the
  sidecar rename. (Constitution IV)
- **C-08 — Report staging:** L0 for M0–M1 (no TSX/QuickJS). L1 is a versioned `ReportSpec`
  with preauthorised Rust components on a read-only snapshot, gated by isolation tests. L2 is
  signed `.nshpkg` with explicitly tightened capabilities. (Constitution V)
- **C-09 — Duckling extraction:** one-way, from a committed commit, with
  `vendor-provenance/DUCKLING.md`. Duckling and duckomo are never modified.
  (Constitution VII) ~~Currently blocked; see OQ-B1.~~ The user approved the narrow,
  content-addressed snapshot route for feature 001 on 2026-11-16; see FR-005 and
  `vendor-provenance/DUCKLING.md`.
- **C-10 — Stable output schema:** `station_series(station_id, variable, valid_time, value,
  grid_lat, grid_lon, distance_m, source_object, object_version)` is the only schema UI,
  export, and reports consume.
- **C-11 — Evidence before claims:** no PASS without a recorded measurement.
  (Constitution VI)
- **C-12 — Toolchain:** the pinned crates' MSRV must be met. `nsh-* 0.5.1` declares
  `rust-version 1.97`. As of 2026-11-16, `lizhan-spark` has rustc 1.96.0 installed, and rustup
  reports stable 1.100.0 available. Nothing has been installed.

## Planned Specs

### Foundations

### 001 — M0 Skeleton & Usable Engine  [status: verified]

- **Status note (2026-11-23):** `v0.1.0-m0` is released. Non-reference platforms are closed by
  GitHub CI/release evidence under Constitution 1.3.0 (user decision). The user's FR-022 call
  accepts NSH 0.5.1, with no fallback; SPIKE-1 and EXIT-3 pass, and the N6 Wayland IBus
  candidate popup under nested Weston 13 is recorded as a non-blocking limitation. See
  `docs/m0/acceptance.md` and `docs/m0/evidence/EXIT-3-spark-aarch64-n-a-20261123.md`.
- **Description:** Workspace skeleton with enforced crate boundaries and CI; a verified
  duckomo bootstrap; a supervised worker; NSH minimal-window acceptance N1–N9 on the
  platform matrix (or the documented §7.3 fallback); the nearest-series spike (A vs B);
  the channel decision; atomic export and recovery.
- **Outcome:** Every M0 exit condition in design §7.1 has recorded evidence. The shell decision
  (NSH accepted, or fallback triggered) and the channel decision are recorded.
- **Scope (in):** design §7.1–§7.3: N1–N9, spikes 1–5, and the exit conditions.
- **Scope (out):** M1 product features (station import UX, catalog readiness, full views);
  L1/L2; M2/M3; GPUI Kit implementation, unless the fallback is formally triggered.
- **Depends on:** none (first spec).
- **Governed by:** C-02..C-11.
- **Spec dir:** specs/001-m0-skeleton-engine/
- **Notes:** ~~Engine extraction is blocked by OQ-B1 (spec FR-005). NSH thresholds are OQ-8e
  (spec FR-022).~~ The approved snapshot route and D8 blocking examples are recorded in the
  feature spec and evidence. Full platform/session acceptance and several measurements remain
  open; see `docs/m0/acceptance.md`.

### Product milestones

### 002 — M1 Stations × Forecast Series  [status: planned]

- **Description:** Station CSV import (snapshot + SHA-256 + mapping confirmation), catalog
  with readiness probes, `station_series` via the M0-chosen plan, table/plot/SQL views, export
  + sidecar, project save/restore, revision-bound previews, task queue. Pure-Rust NSH, no
  TSX.
- **Outcome:** 100 stations × 7 days × 1 variable on Linux (incl. spark) and macOS, with
  cancel, crash, and offline (httpfs degrade) fault injection, and one Chinese IME regression
  pass. (Design §7.4)
- **Scope (in):** design §7.4.
- **Scope (out):** maps, reports, Icechunk.
- **Depends on:** 001.
- **Notes:** ~~needs-info because the default product, variable, and time window are not set
  (OQ-6).~~ OQ-6 resolved 2026-11-23: the default is ECMWF IFS HRES (deterministic),
  2 m air temperature, forecast horizon 0–7 days from the selected run. The exact Open-Meteo OM
  dataset/variable identifiers and run-selection rule will be determined during 002 research.

### 003 — M2 Static Regional Map  [status: planned]

- **Description:** A bbox around the user's stations, one time step, one variable; rasterised
  to PNG in the worker and shown in NSH `Image`; an export task with atomic commit.
- **Outcome:** A map task with revision binding, stale retention on failure, and PNG + sidecar
  export. (Design §7.5)
- **Scope (in):** design §7.5.
- **Scope (out):** a global field browser; interactive overlays (open inside M2).
- **Depends on:** 002; `ensure_spatial` behaviour from the 001 R4 spike.

### 004 — L1 Declarative Reports  [status: needs-info]

- **Description:** A versioned `ReportSpec` (`omo-report/1`), an allow-listed Rust component
  renderer, frozen snapshots, an isolated report worker.
- **Outcome:** Reports render from snapshots only and pass the isolation pre-check. (Design
  §6.3, §7.2)
- **Scope (in):** design §6.3.
- **Scope (out):** TSX/QuickJS.
- **Depends on:** 002; the L1 isolation pre-check passing.
- **Notes:** OQ-8a (spec format), 8b (agent integration), and 8c (snapshot-only vs new reads)
  are open. Can run in parallel with 003.

### 005 — M3 Dynamical Icechunk Source  [status: needs-info]

- **Description:** A `Source` abstraction (`OmSource`, `IcechunkSource`), STAC discovery, the
  same `station_series` output, and the Icechunk snapshot id in the sidecar.
- **Outcome:** Icechunk-backed series flow through unchanged UI, export, and reports. (Design
  §7.6)
- **Scope (in):** design §7.6.
- **Depends on:** 002 (a STAC/reader spike may run in parallel after 002).
- **Notes:** reader placement is OQ-7.

### 006 — L2 Signed TSX Report Packages  [status: undecided]

- **Description:** `.nshpkg` reports in `AppView`, verification against omobench signing
  roots, revocation/rollback checks, tightened capabilities, read-only `omo.*` host API.
- **Outcome:** _to be defined_. It depends on the L2 pre-spike and OQ-8d.
- **Scope (in):** design §6.4.
- **Depends on:** 004; the L2 pre-spike.
- **Notes:** QuickJS is first allowed here (C-02, Constitution II/V). Whether AppView runs in
  a separate process is still open (R14).

Sequence (design §7): 001 → 002 → (003 ∥ 004 ∥ 005-spike) → 005 → 006.

## Open Questions

From design §10. OQ-8e has been resolved for M0 with the user's approval; the other design questions listed here remain open.

- **OQ-1** Final repo name/org. The design suggests `omobench`; work happens in `chorust/OmO`.
  No repo is created or renamed.
- **OQ-2** Whether to later factor Duckling `connector` DuckDB/Studio code into a shared crate.
  That would require changing Duckling, which is not authorised.
- **OQ-3** Whether to file duckomo upstream requests (series regions/nearest, a version
  function, error taxonomy), and whether to tell duckomo that `docs/community-extensions.md`
  is stale.
- ~~**OQ-4** If the signed channel passes M0, does it become the default with unsigned fully
  off? Who updates the lock on the next duckomo bump?~~ **Resolved (2026-11-23, user):**
  `community-signed` is preferred; `github-release-unsigned` remains allowed by explicit
  selection (not disabled); no silent fallback. See `docs/m0/decisions/channel.md`.
  Lock ownership: the user decides whether to upgrade; the lock is never bumped without the
  user's explicit confirmation. After confirmation, both channels × three locked platforms are
  refreshed and must pass `scripts/ci-verify-duckomo.py` in CI before release.
- **OQ-5** Intel Mac: not supported for M0 runtime acceptance or `artifacts/duckomo.lock.json`
  (community has no `osx_amd64` build, and NSH needs macOS 14+) — confirm? Note: the GitHub
  Release v0.4.0 *does* include an unsigned `duckdb-v2.0.0-osx_amd64` build (2026-11-16).
  *Constitution 1.2.0 amendment (2026-11-21):* `macos-15-intel` (`x86_64-apple-darwin`) is
  included in `.github/workflows/ci.yml` and `.github/workflows/release.yml` for build/archive
  coverage only; this does not imply Intel Mac runtime acceptance.
- ~~**OQ-6** M1 default product, variable, and time window. Duckling's catalog entries are all
  pending.~~ **Resolved (2026-11-23, user):** ECMWF IFS HRES, 2 m air temperature, forecast
  horizon 0–7 days. The exact OM identifiers will be determined during 002 research.
- **OQ-7** Where the M3 Icechunk reader lives: an omobench crate, a standalone DuckDB
  extension, or duckomo.
- **OQ-8** Report form and permissions:
  - **8a** custom `ReportSpec` vs an existing protocol (A2UI / OpenUI / json-render);
  - **8b** agent integration (file import / in-app LLM / MCP);
  - **8c** snapshot-only vs budgeted new reads;
  - **8d** L2 signing root (local-only vs team) and AppView process isolation;
  - ~~**8e** M0 NSH thresholds (N2/N3 numbers, N6 IME coverage) and which failures trigger the
    §7.3 fallback.~~ **Resolved for M0 (2026-11-16):** The user approved the D8 blocking
    examples: spark build failure, an unusable N6 GTK IME path without a viable fix, or an
    unusable million-row N2 table. N2/N3 workload sizes and N6 coverage remain as specified;
    N7 records maximum stall and operability without a numeric threshold. See spec FR-022.

Historical blockers found while writing this roadmap (2026-11-16, read-only checks; resolutions annotated):

- **OQ-B1 — resolved for feature 001.** The original 2026-11-16 finding remains historical:
  ~~the design §3.2 extraction sources were not in any local Duckling commit. This covered
  `connector/src/dialect/duckdb/{session,bootstrap}.rs`, `connector/src/studio/*`, and
  `scripts/studio/duckomo-artifacts.json`. At Duckling `main` `8ec3bec` (2026-10-26), they
  existed only in the uncommitted working tree, as did the `duckdb =1.20000.1` pin (committed:
  `1.20000.0`). The check used `git cat-file -e HEAD:<path>` and `git log --all -- <path>`.
  The options were to wait for an upstream commit, approve a hash-recorded snapshot, or
  reimplement using Duckling only as reference.~~ **Resolution (2026-11-16):** The user
  approved the narrow snapshot route for feature 001. The 25 selected source files, hashes,
  source-to-destination mapping, and adaptations are retained under
  `vendor-provenance/snapshots/duckling-8ec3bec-20261116/` and documented in
  `vendor-provenance/DUCKLING.md`; `scripts/verify-duckling-snapshot.py` verifies the bytes.
- **OQ-B2 — resolved (2026-11-17).** ~~The design's reference data for the nearest-series
  spike (Duckling `specs/008` fixture revision 2, or duckomo test reference CSVs) had not been
  checked for presence in a committed tree.~~ The M0 spike uses retained Duckomo v0.4.0
  fixture values plus an independently derived timestamp/nearest-cell oracle. Source commit,
  bytes, hashes, and derivation are recorded in `fixtures/PROVENANCE.md` and
  `fixtures/reference/station_series_manifest.json`; both plans matched the oracle in EXIT-2.
- **OQ-B3** Test hardware: availability of a macOS 14+ arm64 host and a Linux x86_64 host with
  Wayland and X11 sessions is unknown. `lizhan-spark` is aarch64, runs an X11 session
  (`XDG_SESSION_TYPE=x11` at check time), and has no GTK4 dev packages.

## Cross-Cutting Notes

- The NSH capability table (design §5.1) was built from `main`-branch docs. Treat every item
  as unverified on the v0.5.1 tag until M0 confirms it.
- crates.io facts (2026-11-16):
  - `nsh-platform-{linux,macos} 0.5.1` default features include `js` → `nsh-engine/js-core` →
    `nsh-js` → `quickjs-jit`.
  - `nsh-platform-linux` also requires gtk4 0.11, pango 0.22, and gstreamer 0.25.
  So L0's "no QuickJS" means `default-features = false` with explicit features (e.g.
  `editor`, `highlight`, `plot`). This must be proven in M0 N9.
- Community vs GitHub-release duckomo 0.4.0 are not shown to be the same build. The `.gz`
  sizes differ (linux_arm64 v2.0.0: 8,846,401 vs 8,783,732 bytes); signing alone may account
  for this, but it is unverified. Use cross-channel golden-query comparison.

---

**Version**: 1.0.5 | **Ratified**: 2026-11-16 | **Last Amended**: 2026-11-23
