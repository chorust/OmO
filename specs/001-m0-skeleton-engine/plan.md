# Implementation Plan: M0 Skeleton & Usable Engine

**Branch**: `001-m0-skeleton-engine` (spec dir only; no git branch created) | **Date**: 2026-11-16 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/001-m0-skeleton-engine/spec.md`

## Summary

M0 does four things:

- **Workspace**: builds the omobench Rust workspace: four UI-free core crates plus
  `omo-app`. It enforces dependency boundaries in CI.
- **Engine**: delivers a per-connection verified duckomo bootstrap with both distribution
  channels locked, and a supervised `omo-worker` that has generations, Arrow IPC results and
  three-level cancellation.
- **UI**: produces a minimal NSH 0.5.1 pure-Rust workbench with no QuickJS, used to run the
  N1–N9 acceptance on macOS arm64, Linux x86_64 and spark aarch64.
- **Spikes and export**: closes the nearest-series (A/B), channel and R4 spikes, and proves the
  atomic export protocol under kill injection.

Each exit is decided by recorded evidence (Principle VI). Engine code extraction from Duckling
is **gated** on the user's answer to FR-005 (research D4).

## Technical Context

**Language/Version**: Rust, exact stable pin ≥ 1.97. Proposed 1.100.0 (research D1). spark
currently has 1.96.0, which is too old for NSH 0.5.1.

**Primary Dependencies**:
- `nsh-core`/`nsh-base`/`nsh-platform-linux`/`nsh-platform-macos` `=0.5.1`, with
  `default-features=false` and features `editor, highlight, plot` (D2). These go in
  `omo-app` only.
- `duckdb =1.20000.1` (`bundled`, `extensions-full`), in `omo-engine`/`omo-worker` only.
- `arrow`/`arrow-ipc 60.x`.
- `tokio`, `serde`/`serde_json`, `sha2`.

**Storage**: local files only. The artifact lock is JSON. Results are Arrow IPC files in the
session temp dir. Exports are CSV/Parquet plus a JSON sidecar. Fixtures live under `fixtures/`.

**Testing**: `cargo test --locked`, headless harness binaries (`omo-probe`, `omo-harness`,
`omo-series-spike`), and manual N-checklists with evidence files. Tests are **required** by
the constitution workflow for bootstrap probes, protocol framing, generation discard,
cancellation, export commit and recovery, and preview staleness.

**Target Platform**: macOS 14+ arm64 (AppKit); Linux glibc x86_64 and aarch64 with GTK ≥
4.14, on Wayland and X11. The reference aarch64 host is `lizhan-spark` (Ubuntu 24.04.4).

**Project Type**: desktop app plus a Rust library/worker workspace.

**Performance Goals**: the design specifies none. These are measured instead:
- N2: 1e6 × 20 rows. Frame time p50/p95, max main-thread stall and RSS are measured.
- N3: 20 × 10,000 points. Render time is measured.
- N7: 50 Hz × 10 min with no main-thread blocking.
- 100 consecutive connections bootstrap successfully.

The blocking thresholds are set by the user under FR-022 (D8).

**Constraints**:
- No DuckDB in the `omo-app` release build.
- No QuickJS, GPUI or Tauri anywhere in the app.
- Builds use `--locked`.
- The signed-to-unsigned channel fallback is never silent.
- Cancellation drains for at most 10 s.
- httpfs gets a 15 s timeout.
- Exports never copy across filesystems.

**Scale/Scope**: 5 crates and 3 harness binaries. The workbench has a single window. The spike
uses a small station set (on the order of 10 stations) and one variable.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Gate | Pre-design | Post-design |
|---|---|---|---|
| I. UI-free core crates | Core crates contain no nsh/gpui/tauri. The app is split into host and ui layers. The app release build contains no DuckDB. | PASS (planned) | PASS: `ci-checks.md` enforces it; the `in-process-engine` feature is test-only |
| II. NSH pure Rust, exact pin, no embedding | Pins are `=0.5.1`; `nsh-platform-gpui` is absent; QuickJS is absent; the fallback is §7.3 only | PASS | PASS: D2 disables the default `js` feature (E3). The toolchain is raised to meet MSRV 1.97 |
| III. Pinned engine, per-connection bootstrap | `duckdb =1.20000.1`; duckomo v0.4.0 locked per channel; bootstrap order, probes and golden query; no silent fallback | PASS | PASS: the `artifact-lock.md` and `worker-protocol.md` contracts cover it |
| IV. Supervised process and state boundaries | Child worker, generations, IPC files, three-level cancel, sidecar commit point | PASS | PASS: `export-protocol.md` and the cancel state machine cover it |
| V. Staged reports | L0 only: no TSX or QuickJS | PASS | PASS |
| VI. Evidence before claims | Every exit needs a recorded evidence entry | PASS | PASS: `acceptance-record.md` defines it. Nothing is claimed as verified beyond research §0 |
| VII. Hands off neighbouring repos | One-way import from committed source or approved content-addressed snapshot; neighbouring repos are never modified | **AT RISK** | **AMENDED 1.1.0**: the user approved a working-tree snapshot with retained source bytes, per-file hashes, mapping, and trim notes. Capture/import evidence is still required |

No unjustified violations. The Principle VII item is handled as a hard gate, not as a waiver,
so no Complexity Tracking entry is needed.

**Resolved clarifications**: FR-005 is resolved by the user-approved hashed snapshot route;
FR-022 uses the user-approved D8 blocking examples, with N7 stall reported but no numeric
threshold. These decisions are captured in `research.md` and `spec.md`. Source capture,
installation, implementation, and platform measurements remain execution tasks. All other
technical unknowns are resolved in `research.md` or remain measurement tasks.

## Project Structure

### Documentation (this feature)

```text
specs/001-m0-skeleton-engine/
├── plan.md              # this file
├── research.md          # Phase 0: decisions D1–D10 + evidence log E1–E16
├── data-model.md        # Phase 1
├── quickstart.md        # Phase 1: validation runs
├── contracts/
│   ├── worker-protocol.md
│   ├── artifact-lock.md
│   ├── export-protocol.md
│   ├── ci-checks.md
│   └── acceptance-record.md
├── checklists/requirements.md
└── tasks.md             # /speckit.tasks
```

### Source Code (repository root)

```text
Cargo.toml                     # workspace, resolver=2; Cargo.lock committed
rust-toolchain.toml            # exact stable ≥1.97 (D1)
crates/
├── omo-engine/                # UI-free; duckdb =1.20000.1
│   └── src/{lib.rs, config.rs, bootstrap.rs, conn.rs, probe.rs, query.rs, results.rs,
│            limits.rs, series/{mod.rs, plan_a.rs, plan_b.rs}, export.rs, lock.rs}
├── omo-protocol/              # UI-free; envelope, frames, ResultRef, JobState, errors
│   └── src/{lib.rs, frame.rs, messages.rs, result_ref.rs, state.rs}
├── omo-worker/                # UI-free; bin omo-worker + omo-probe + omo-series-spike
│   └── src/{main.rs, server.rs, heartbeat.rs, bin/omo-probe.rs, bin/omo-series-spike.rs}
├── omo-project/               # UI-free; ProjectDoc (revision/dirty), PreviewKey
│   └── src/{lib.rs, doc.rs, preview_key.rs}
└── omo-app/                   # only crate with nsh-*; no duckdb in default features
    └── src/{main.rs,
             host/{mod.rs, supervisor.rs, cancel.rs, preview_queue.rs, export_scan.rs, ipc_pages.rs},
             ui/{mod.rs, workspace.rs, sql_view.rs, table_view.rs, plot_view.rs, tasks_view.rs, dialogs.rs},
             bin/omo-harness.rs}
artifacts/duckomo.lock.json
catalog/products.json          # placeholder with schema version (M1 fills)
fixtures/{om/, stations/, reference/}   # with PROVENANCE.md
vendor-provenance/DUCKLING.md  # content depends on FR-005
scripts/{check-boundaries.sh, fetch-duckomo.sh}
docs/m0/{acceptance.md, n-checklist.md, evidence/, decisions/}
.github/workflows/ci.yml
```

**Structure Decision**: a Cargo workspace using the crate split from design §2.1. Tests live
beside each crate (`crates/*/tests/`). The harness binaries are the integration surface for
fault injection.

## Phase summary

- **Phase 0 (done)**: `research.md`. Toolchain, NSH features, Linux prerequisites, channels,
  IPC, spike data, thresholds, golden fixture and CI are decided or gated. Evidence log
  attached.
- **Phase 1 (done)**: `data-model.md`, `contracts/*`, `quickstart.md`.
- **Constitution re-check**: passes, with the Principle VII extraction gate kept as shown.

## Complexity Tracking

Not required. There are no unjustified violations.
