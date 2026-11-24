# Implementation Plan: M1 Stations × Forecast Series

**Branch**: `002-m1-station-time-series` (spec directory only; no git branch created) | **Date**: 2026-11-23 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/002-m1-station-time-series/spec.md`

## Summary

M1 turns the verified M0 skeleton into the first end-user workflow. A user imports a station
CSV, confirms the column mapping, and gets nearest-point ECMWF IFS HRES 9 km 2 m temperature
series for lead 0–168 h. The series comes from the latest completely published run, truncated
and labelled for 06Z/18Z runs. The user can view it as a table or chart, query it with SQL,
export it with a provenance sidecar, and save/reopen it as a project with crash-safe drafts.

Technical approach (see [research.md](research.md)):
- The default grid is reduced Gaussian O1280, and duckomo v0.4.0 nearest rejects it. M1 adds
  **Plan G**:
  - a pure-Rust O1280 row table and nearest search in `omo-engine`;
  - a selective `read_om(domain := 'ecmwf_ifs')` read of only the selected points from the
    per-run `data_run` object;
  - a coordinate cross-check on every request.
- A run is resolved from Open-Meteo `meta.json` against per-cycle horizons, because early
  +72 h uploads exist.
- All network I/O stays in the worker's httpfs.
- Spike **S1** on the locked stack gates the rest of the work.

## Technical Context

- **Language/Version**: Rust 1.100.0 (`rust-toolchain.toml`), workspace MSRV 1.97. Python 3
  for dev-only scripts and oracles.
- **Primary Dependencies**:
  - locked: NSH `=0.5.1` (UI only), DuckDB v2.0.0 / duckdb-rs `=1.20000.1` (worker only),
    duckomo v0.4.0 (locked artifact, both channels).
  - new: `csv` crate (`omo-project`, station import).
  - reused from the lockfile: `serde_json`, `sha2`, `uuid`, `ryu`, `tempfile`.
  - dev-only: Python `omfiles` (official OM reader) for the SC-003 oracle, pinned in
    `scripts/requirements-oracle.txt`.
- **Storage**:
  - project files `*.omoproj` (JSON `omo-project/1`);
  - drafts in `<app_data>/drafts/`;
  - session results as Arrow IPC + Parquet in `<session_tmp>/g<gen>/`;
  - exports as CSV/Parquet + sidecar `omo-export-sidecar/2`;
  - catalog `catalog/products.json` (`omo-catalog/1`).
- **Testing**:
  - `cargo test --workspace --locked`: unit and contract tests for row table, nearest,
    import, project I/O, catalog, run resolution, protocol bodies, preview keys, sidecar `/2`;
  - network-gated `#[ignore]` integration tests plus `scripts/ci-verify-series.py`;
  - Python oracles (R-09);
  - `omo-harness` fault injection (M0 tool, extended to series).
- **Target Platform**:
  - reference host for interactive acceptance: Linux aarch64 (`lizhan-spark`, X11);
  - CI/release (Constitution 1.3.0): Linux x86_64, macOS arm64, and build-only
    `macos-15-intel`.
- **Project Type**: Native desktop app (NSH) with a supervised engine worker process.
- **Performance Goals** (provisional, measured in S1):
  - 100 stations × 113 steps in < 2 min (median of 5, cold cache);
  - UI responsiveness governed by the M0 FR-022 rule;
  - plot cap 20 series × 10,000 points (M0).
- **Constraints**:
  - no DuckDB in `omo-app` release;
  - no NSH in core crates or `omo-app::host`;
  - no new DuckDB extensions;
  - no silent signed→unsigned fallback;
  - no data mixing across runs;
  - remote access only through the bootstrapped connection;
  - exports commit atomically through the sidecar rename.
- **Scale/Scope**:
  - 1–1,000 stations (provisional), 1 variable, 1 run per request;
  - 6 user stories, 30 FRs;
  - 1 catalog entry required ready.

No `NEEDS CLARIFICATION` remains. Research R-01–R-10 resolved every unknown. The items still
**UNVERIFIED** (DuckDB 2.0 pushdown on O1280, `data_run` dimension names, LUT cost, quit
interception) are each owned by spike S1 or a named mitigation.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design (below).*

| Principle | Check | Status |
|---|---|---|
| I UI-free core | Station import and project I/O are in `omo-project`; catalog, grid, and Plan G in `omo-engine`; protocol bodies in `omo-protocol`. The `csv` crate is UI-free. Only `omo-app::ui` touches NSH. `omo-app` gains no DuckDB and no HTTP client | PASS |
| II NSH pinned, no embedding | NSH stays `=0.5.1`. UI pieces are built by composition (R-07); no GPUI, Tauri, or QuickJS | PASS |
| III pinned engine, verified bootstrap | DuckDB, duckomo, and the lock are unchanged. Every `series`/`catalog_check`/`resolve_run` runs on `BootstrappedConn` after the function probe. Remote work is rejected with `CapabilityUnavailable(remote)` when httpfs is degraded. The channel policy follows OQ-4 | PASS |
| IV supervised process, state boundaries | All OM and JSON reads happen in the worker. Results go through `ResultRef`. `PreviewKey` gains run, window, and distance. Jobs snapshot the doc. `received` and `saved` stay distinct. Drafts never overwrite projects. Exports use the M0 commit protocol (sidecar `/2`) | PASS |
| V staged reports | L0 only: fixed workbench, no TSX/QuickJS. `station_series` schema unchanged; match report kept in a side table | PASS |
| VI evidence before claims | Unverified facts are labelled (research). S1 gate, SC-003 oracle, and acceptance evidence go to `docs/m1/evidence/` | PASS |
| VII hands off neighbours | duckomo was read only (git show at the `v0.4.0` tag). No code or test data is copied from duckomo. The O1280 rule is reimplemented from Open-Meteo's published producer rule, with the source cited. Interface gaps (nearest on Gaussian) become an optional upstream request (OQ-3), not a dependency | PASS |
| Workflow gates | Tests for preview staleness, export commit/recovery, and cancellation are extended. Fault injection is part of acceptance. No commit or push without the user asking | PASS |

## Project Structure

### Documentation (this feature)

```text
specs/002-m1-station-time-series/
├── spec.md
├── plan.md              # this file
├── research.md          # Phase 0
├── data-model.md        # Phase 1
├── quickstart.md        # Phase 1
├── contracts/
│   ├── worker-protocol-m1.md
│   ├── catalog.md
│   ├── project-file.md
│   ├── station-csv-import.md
│   └── export-sidecar-v2.md
├── checklists/requirements.md
└── tasks.md             # Phase 2 (/speckit.tasks; not created here)
```

### Source Code (repository root)

```text
catalog/products.json                     # omo-catalog/1 (replaces /0 placeholder)
crates/
├── omo-engine/src/
│   ├── catalog/{mod.rs,run.rs}           # NEW: catalog load/validate, run resolution, readiness
│   ├── grid/{mod.rs,reduced_gaussian.rs} # NEW: O1280 row table (Float32 producer rule), nearest search
│   ├── series/plan_g.rs                  # NEW: Plan G read + coordinate cross-check + time mapping
│   ├── series/{mod.rs,schema.rs}         # EXTEND: plan selection, match report, Parquet sibling
│   └── export.rs                         # EXTEND: sidecar /2, series-result export
├── omo-protocol/src/
│   ├── series.rs                         # NEW: SeriesRequest/Result, catalog_check/resolve_run bodies, FailKinds
│   └── export.rs                         # EXTEND: SIDECAR_SCHEMA_V2, accept /1|/2
├── omo-worker/src/
│   ├── server.rs                         # EXTEND: catalog_check, resolve_run, series, sql_session
│   └── bin/omo-series-spike.rs           # EXTEND: --plan G for S1
├── omo-project/src/
│   ├── stations/{mod.rs,import.rs}       # NEW: CSV parse/sniff/mapping/validate/canonical hash
│   ├── doc.rs                            # EXTEND: omo-project/1 fields, save/open (atomic), version gate
│   ├── draft.rs                          # NEW: draft staging/recovery
│   └── preview_key.rs                    # EXTEND: run_ref, window, max_distance_m
└── omo-app/src/
    ├── host/{runtime.rs,supervisor.rs,export_scan.rs}  # EXTEND: series/catalog tasks, sql_session after ready, sidecar /2
    ├── host/project_io.rs                # NEW: off-main-thread save/open/draft scheduling
    └── ui/
        ├── workspace.rs                  # EXTEND: menus (Open/Save/Save As), dirty title, close guard
        ├── import_view.rs                # NEW: mapping form (Select), preview, issues list
        ├── catalog_view.rs               # NEW: entries, readiness, attribution, run time
        ├── stations_view.rs              # NEW: filterable checkbox list (multi-select)
        ├── plot_view.rs                  # EXTEND: multi-series + composed legend, truncation label
        └── window_control.rs             # NEW: lead-time range (Slider/NumberInput)
crates/*/tests/                           # catalog, grid_o1280, plan_g (offline parts), stations_import,
                                          # project_file, draft, preview_key, sidecar_v2, protocol_series
scripts/
├── o1280-nearest-oracle.py               # NEW (dev): brute-force nearest oracle
├── o1280-value-oracle.py                 # NEW (dev): official omfiles values
├── requirements-oracle.txt               # NEW (dev)
└── ci-verify-series.py                   # NEW: CI network smoke test (SC-009)
fixtures/
├── stations/{m1-100.csv,m1-oracle.csv}   # NEW
└── reference/o1280-*/                    # NEW: oracle outputs + PROVENANCE entries
docs/m1/evidence/                         # NEW: S1, acceptance, IME, fault-injection records
```

**Structure Decision**: Keep the existing five-crate workspace (Constitution I) and add modules
only, with no new crates. Grid math and Plan G live in `omo-engine` so that the worker owns
sampling. Station import and project I/O live in `omo-project` so that the host can parse and
save without DuckDB. UI additions are confined to `omo-app::ui`.

## Phasing for /speckit.tasks (guidance)

1. **Gate S1**:
   - O1280 row table and nearest search, with unit tests;
   - `omo-series-spike --plan G`;
   - evidence S1 (a)–(d).

   Stop and ask the user on failure.
2. **Engine and protocol**: catalog `/1` and run resolver → `series`/`catalog_check`/
   `resolve_run`/`sql_session` → Parquet sibling and match report → sidecar `/2`.
3. **Project**: station import → `omo-project/1` save/open → drafts → `PreviewKey` extension.
4. **UI**: import view → catalog view → series task wiring → stations multi-select, plot
   legend, and window control → menus, dirty title, close guard → export and SQL wiring.
5. **Acceptance**:
   - oracle fixtures and SC-003;
   - V1–V9 on Spark;
   - `ci-verify-series.py` in CI;
   - fault injection;
   - IME pass;
   - evidence in `docs/m1/`.

## Post-design Constitution re-check

Re-evaluated after writing the data model and contracts: every row stays **PASS**.
- The new protocol kinds are additive and keep envelope v1 (IV).
- The sidecar `/2` keeps the single commit point (IV).
- The catalog adds no extension or network path outside the worker (I, III).
- The oracles are dev-only Python scripts and are not shipped (I, VII).

## Complexity Tracking

No constitution violations. Two additions are recorded for reviewers:

| Addition | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| New `csv` dependency in `omo-project` | RFC 4180 quoting, BOM, and delimiter handling for user CSVs (FR-001, FR-004) | The M0 strict 3-column parser cannot handle quotes, optional columns, or semicolons. DuckDB sniffing would need the worker and is not deterministic across versions |
| Plan G alongside Plan B | The user-selected default grid (O1280) is rejected by duckomo nearest | Substituting `ecmwf_ifs025` was rejected by the user. Full-grid reads (578 MB per run) break SC-002 |
