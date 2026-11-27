---
description: "Task list for M2 Regional Tile Map & Layer Stack"
---

# Tasks: M2 Regional Tile Map & Layer Stack

**Input**: Design documents from `specs/003-m2-regional-tile-map/`:
- [plan.md](plan.md)
- [spec.md](spec.md)
- [research.md](research.md)
- [data-model.md](data-model.md)
- [contracts/](contracts/)
- [quickstart.md](quickstart.md)

**Prerequisites**: `plan.md` and `spec.md` (required); `research.md`, `data-model.md`, `contracts/`, and `quickstart.md` are all present.

**Tests**: INCLUDED. Under the project Constitution (Development Workflow & Principle VI), automated tests and fault injection are mandatory for bootstrap/config locks, protocol framing, generation discard & artifact reaping, cancellation, preview staleness, numerical parity, and atomic export commit/recovery. Write each test task before its implementation task and confirm it fails first.

**Organization**: Tasks are grouped by user story (`US1`–`US3` from [spec.md](spec.md)) so each story can be implemented, tested, and demonstrated independently after the Foundational phase.

**Toolchain**: Run every Cargo command as `cargo +1.100.0 … --workspace --locked`, except `T001`, which performs the single deliberate `Cargo.lock` update.

## Format: `[ID] [P?] [Story?] Description`

- **[P]**: Can run in parallel (different files, no dependency on an incomplete task)
- **[Story]**: `[US1]`, `[US2]`, or `[US3]` (User Story phases only)
- All paths are repository-relative

---

## Phase 1: Setup & Gate `S0-NSH-070` (Shared Infrastructure)

**Purpose**: Shell pin upgrade gate (`S0-NSH-070`), pure-Rust dependencies, module skeletons, deterministic offline geodata/PMTiles fixtures, and evidence scaffolding.

- [X] T001 Propose and record the Constitution Principle II pin amendment (`nsh-* =0.5.1 → =0.7.0`) in `.specify/memory/constitution.md`, update `nsh-*` pins and add pure-Rust `omo-engine` dependencies (`pmtiles = { version = "0.28.1", default-features = false, features = ["brotli"] }`, `ruzstd = "0.9.1"`, `flate2 = { version = "1.1.11", default-features = false, features = ["zlib-rs"] }`, `mvt-reader = "2.5.0"`, `tiny-skia = "0.12.0"`, `colorgrad = { version = "0.9.0", features = ["preset"] }`, `png = { version = "0.18.1", features = ["zlib-rs"] }`, `image = { version = "0.25.10", default-features = false, features = ["png", "jpeg", "webp"] }`, `memmap2 = "0.9.12"`) in `Cargo.toml`, `crates/omo-app/Cargo.toml`, and `crates/omo-engine/Cargo.toml`, update `=0.7.0` pin assertions (plus forbid `zstd-sys`/`tokio`/`reqwest` in `omo-engine`) in `scripts/check-boundaries.sh`, update `Cargo.lock`, and verify `cargo +1.100.0 test --workspace --locked` and `scripts/check-boundaries.sh --self-test`. **Outcome (2026-11-26): the `nsh-* =0.7.0` bump was reverted to `=0.5.1` after `S0-NSH-070` failed (see T002); the pure-Rust `omo-engine` dependencies and boundary-check additions remain.**
- [X] T002 Run Gate `S0-NSH-070` interactive X11 regression smoke via `scripts/automate_m1_gui.py` on Spark and record `docs/m2/evidence/S0-NSH070-spark-aarch64-<date>.md` (**STOP and revert `T001`'s NSH bump to `=0.5.1` if any existing GUI workflow regresses**). **Outcome: FAILED on `=0.7.0` (main window never mapped; `0.5.1` baseline fine) → bump reverted; evidence `docs/m2/evidence/S0-NSH070-spark-aarch64-20261126.md`.**
- [X] T003 [P] Create empty M2 module skeletons and register exports in `crates/omo-protocol/src/{lib.rs,map.rs}`, `crates/omo-project/src/{lib.rs,map.rs}`, and `crates/omo-engine/src/{lib.rs,map/mod.rs,map/mercator.rs,map/fetch.rs,map/raster.rs,map/coastlines.rs,map/pmtiles_layer.rs,map/compose_export.rs}`.
- [X] T004 [P] Write deterministic Natural Earth `1:50m` packer `scripts/build-ne50m-ocb1.py` and generate `fixtures/geodata/ne_50m_coast_borders.ocb1` (`OCB1` format, coastlines + land admin-0 borders).
- [X] T005 [P] Create the 100-station `30°×40°` East Asia synoptic fixture `fixtures/stations/m2-regional-100.csv` (including CJK station names, coastal points, and row-boundary coordinates).
- [X] T006 [P] Write `scripts/build-pmtiles-fixtures.py` and generate deterministic offline test archives `fixtures/pmtiles/sample-raster.pmtiles`, `fixtures/pmtiles/sample-mvt.pmtiles`, and `fixtures/pmtiles/corrupt.pmtiles`.
- [X] T007 Record source URLs, versions, generation commands, and SHA-256 digests for the `T004`–`T006` assets in `fixtures/PROVENANCE.md`, and create `docs/m2/evidence/README.md` and `docs/m2/acceptance.md` with initial rows for `S0`, `S1`, `V1`–`V8`, and `SC-001`–`SC-009`.

---

## Phase 2: Foundational (Blocking Prerequisites & Gate `S1-M2`)

**Purpose**: Catalog `spatial_layout`, pre-lock `duckomo` request tuning, `EPSG:3857` & antimeridian math, protocol wire types, `MapPreviewKey`, `SupervisorCore` stale-generation directory reaping, parallel `data_spatial`/`data_run` field loader, and Gate `S1-M2`.

**⚠️ CRITICAL**: No user-story implementation starts until `T020` (Gate `S1-M2`) passes. If `SC-001` (`< 30 s` median) or `SC-004` (`max |Δ| == 0.0`) fails, STOP and ask the user.

### Foundational Tests (write first, confirm failing)

- [X] T008 [P] Write `omo-catalog/1` `SpatialLayout` validation and URL-template tests in `crates/omo-engine/tests/catalog_m2.rs` (accepts omitted `spatial_layout` and valid `data_spatial`, rejects unknown `kind` or missing `{valid_*}` tokens, builds exact `{YYYY}/{MM}/{DD}/{HH}00Z/{YYYY}-{MM}-{DD}T{HH}{mm}.om` URLs).
- [X] T009 [P] Write `EPSG:3857` tile math and station-framing tests in `crates/omo-engine/tests/mercator.rs` (`z = 1..12` pixel ↔ WGS 84 round-trip, `±85.05112878°` polar clamp classification, exact `u64` `east_px` wrap at `0°` and `±180°`, single-station `±2.0°` minimum span, and shortest-arc antimeridian bbox for `+178°` & `-178°`).
- [X] T010 [P] Write wire serialization and contract tests for `omo-protocol::map`, `ValueProvenance` (`native_numeric`, `palette_decoded`, `display_only`), `SIDECAR_SCHEMA_V3`, and `ValidTimeUnavailable` / `InvalidLayer` `FailKind`s in `crates/omo-protocol/tests/map_protocol.rs`.
- [X] T011 [P] Write `MapPreviewKey` equality, `-0.0 → 0.0` range normalization, and staleness-invalidation tests in `crates/omo-project/tests/map_preview_key.rs`.
- [X] T012 [P] Write `SupervisorCore` tests in `crates/omo-app/tests/supervisor.rs` verifying tracked map request kinds (`map_load_field`, `map_render_tiles`, `map_inspect`, `map_layer_probe`), stale-message `.rgba`/`.png`/`.f32bin` deletion, and recursive `<result_root>/g<old_gen>/` directory reaping on worker crash/restart (`SC-007`).

### Foundational Implementation

- [X] T013 [P] Implement `omo-protocol::map` wire structs and `ValueProvenance` in `crates/omo-protocol/src/map.rs`, add optional `SpatialLayout` validation in `crates/omo-protocol/src/series.rs`, add `SIDECAR_SCHEMA_V3` in `crates/omo-protocol/src/export.rs`, and add `ValidTimeUnavailable` / `InvalidLayer` in `crates/omo-protocol/src/state.rs`.
- [X] T014 [P] Implement `EPSG:3857` coordinate/tile transforms, polar station filter, and shortest-arc antimeridian bounding-box framing in `crates/omo-engine/src/map/mercator.rs`.
- [X] T015 [P] Implement `MapPreviewKey` in `crates/omo-project/src/preview_key.rs`.
- [X] T016 Add `spatial_layout` to `catalog/products.json` (`version: "2026-11-25.1"`) and implement `spatial_object_url` in `crates/omo-engine/src/catalog/mod.rs`.
- [X] T017 Add pre-lock `duckomo_remote_max_request` (`4 MiB` default for remote sessions) and `duckomo_remote_merge_gap` (`64 KiB` default) fields to `BootstrapConfig` in `crates/omo-engine/src/config.rs`, apply them before `SET lock_configuration = true;` in `crates/omo-engine/src/bootstrap.rs`, and add lock verification in `crates/omo-engine/tests/lock.rs`.
- [X] T018 Implement `CachedField` (2-entry LRU, global min/max stats) and the parallel latitude-band `data_spatial` fetcher (`variable := '<var>'`, `≤ 16` readers sharing `SeriesCancel`, `[object_not_found] (HTTP 404)` single fallback to `data_run`, ETag stability check, and `60°×80°` wide-area viewport clipping) in `crates/omo-engine/src/map/{mod.rs,fetch.rs}`.
- [X] T019 Extend `SupervisorCore` (`is_tracked_request`, `.rgba`/`.png`/`.f32bin` whitelist, and `g<old_gen>/` rollover reaper) in `crates/omo-app/src/host/supervisor.rs`.
- [X] T020 Write `scripts/perf-map.py` (with minimal `map_load_field` dispatch in `crates/omo-worker/src/{server.rs,main.rs}`) and execute **Gate `S1-M2`** on Spark: (a) `N = 5` cold-field live `s3://openmeteo/data_spatial/ecmwf_ifs` runs on `fixtures/stations/m2-regional-100.csv` using a fresh worker process and fresh temporary data directory per trial (`SC-001` median `< 30 s`, relaxed from 15 s by user decision after the first S1-M2 attempt), and (b) raw retrieved `f32` `CachedField` grid-value parity (`SC-004` `max |Δ| == 0.0` before tile interpolation) between `data_spatial` and `data_run` for the same pinned run and valid time, recording `docs/m2/evidence/S1-spark-aarch64-<date>.md` (**STOP and escalate to the user if either gate fails**).

**Checkpoint**: Foundation & `S0`/`S1` gates verified — User Story phases can now proceed.

---

## Phase 3: User Story 1 - Interactive Forecast Tile Map Around My Stations (Priority: P1) 🎯 MVP

**Goal**: Auto-frame imported user stations on a smooth Web Mercator (`EPSG:3857`) `256×256` `z/x/y` forecast tile map with valid-time selection, `data_spatial → data_run` fallback notice, color scale legend, station markers, and exact `"native numeric"` cursor/point inspection.

**Independent Test**: Import `fixtures/stations/m2-regional-100.csv` (and `m1-100.csv`), open the Map tab, verify automatic bbox framing, drag-pan and scroll-zoom with `< 100 ms` cached tile rendering (`0` blank flashes, `0` seams), step valid times, inspect cursor values against M1 series values, and verify cancel/crash/offline stale retention (`quickstart.md` `V1`–`V3`).

### Tests for User Story 1 (write first, confirm failing)

- [X] T021 [P] [US1] Write zero-seam (`max |ΔRGBA| == 0` across `2×2` `256×256` tiles vs `512×512` single pass across interior, `0°`, and `±180°` seams) and render-speed tests for O1280 `TrapezoidBilinear` and regular 2D grids in `crates/omo-engine/tests/map_tiles.rs` (`SC-002`, `SC-006`).
- [X] T022 [P] [US1] Write `PointReadout` tests (`native_numeric` exact match against `O1280::nearest`, `NoData` on `NaN` cells, `OutsideCachedExtent`, polar/antimeridian handling) in `crates/omo-engine/tests/map_inspect.rs` (`SC-003`).
- [X] T023 [P] [US1] Write worker protocol integration tests for `map_load_field`, `map_render_tiles`, `map_inspect`, `busy` rejection, and `SeriesCancel` interruption in `crates/omo-worker/tests/m2_map_handlers.rs`.
- [X] T024 [P] [US1] Write `MapRequestScheduler` unit tests (request serialization, rapid pan/zoom tile-batch coalescing, superseding field-revision cancellation, and `Stale` state retention on error) in `crates/omo-app/tests/map_scheduler.rs`.

### Implementation for User Story 1

- [X] T025 [P] [US1] Implement O1280 `TrapezoidBilinear` and regular-grid 2D bilinear `256×256` RGBA8 tile rasterizer with 4096-entry `colorgrad` LUT and uniform field-wide color normalization in `crates/omo-engine/src/map/raster.rs`.
- [X] T026 [P] [US1] Implement `PointReadout` (`native_numeric`) sampling and generation-scoped `field_*.f32bin` hover-index serialization in `crates/omo-engine/src/map/mod.rs`. **Outcome: hover values are served by `map_inspect` (authoritative numeric sample); the optional `f32bin` zero-IPC hover index was not built — `omo-app` cannot link `omo-engine` grid tables without breaking the duckdb boundary, so `readout_path` stays `None`.**
- [X] T027 [US1] Wire `map_load_field`, `map_render_tiles` (forecast layer), and `map_inspect` handlers with `SeriesCancel` checks in `crates/omo-worker/src/{server.rs,main.rs}`.
- [X] T028 [US1] Implement `MapRequestScheduler`, `.rgba` tile file reader, and `f32bin` zero-IPC hover reader in `crates/omo-app/src/host/map_host.rs`, and route map requests in `crates/omo-app/src/host/{runtime.rs,mod.rs}`. **Outcome: `f32bin` hover reader not built (see T026); hover uses `map_inspect`. Load bbox is the viewport plus a 1-tile gutter, capped at 60°×80° around the center; `busy` replies are retried.**
- [X] T029 [US1] Implement the interactive slippy-map surface in `crates/omo-app/src/ui/map_view.rs` (clipped container, keyed `each(visible_tiles)` `ImageSrc::rgba` tiles, station markers with selection highlight, polar-station notice, `TouchClaim::on_start()` responder drag-pan, cursor-anchored `on_wheel` / double-click / `on_pinch` zoom, 120 Hz coordinate + `"native numeric"` readout HUD, colorbar legend, and `"received, not saved"` / `stale` / `data_run` fallback badges).
- [X] T030 [US1] Implement valid-time selector, palette & min/max lock controls, and `+`/`-`/`"Fit to stations"` toolbar in `crates/omo-app/src/ui/map_controls.rs`, and register the **Map** center tab in `crates/omo-app/src/ui/{workspace.rs,mod.rs}`.

**Checkpoint**: User Story 1 (MVP) is complete and independently testable via `quickstart.md` `V1`–`V3`.

---

## Phase 4: User Story 2 - Multi-Layer Stack, Offline Coastlines & Local PMTiles (Priority: P1)

**Goal**: Provide an ordered **Layer Stack** (visibility, opacity, z-order) with built-in offline Natural Earth `1:50m` coastlines/borders (`OCB1`) and user-mounted local `.pmtiles` v3 archives (raster `png`/`jpeg`/`webp` and deterministic geometry-only MVT), surfacing attributions and `"display only"` readout provenance.

**Independent Test**: Run `omo-app` offline (`bwrap --unshare-net`), verify crisp `OCB1` coastlines across `z = 1..12`, mount `sample-raster.pmtiles`, `sample-mvt.pmtiles`, and `corrupt.pmtiles`, reorder/toggle/fade layers, and confirm `"display only"` vs `"native numeric"` readout behavior (`quickstart.md` `V4`).

### Tests for User Story 2 (write first, confirm failing)

- [X] T031 [P] [US2] Write `OCB1` header/index parser, bbox culling, and Cohen–Sutherland segment-clipping tests against `fixtures/geodata/ne_50m_coast_borders.ocb1` in `crates/omo-engine/tests/coastlines.rs` (`SC-005`).
- [X] T032 [P] [US2] Write local `.pmtiles` v3 tests (raster PNG/JPEG/WebP decode, gzipped & `ruzstd` MVT geometry rasterization, `minzoom`/`maxzoom` overzoom & bounds clamping, metadata attribution extraction, and `corrupt.pmtiles` error isolation) in `crates/omo-engine/tests/pmtiles_layer.rs` (`SC-005`).

### Implementation for User Story 2

- [X] T033 [P] [US2] Implement `OCB1` offline coastline/admin-0 reader and viewport segment clipper in `crates/omo-engine/src/map/coastlines.rs`.
- [X] T034 [P] [US2] Implement sync `mmap` `.pmtiles` v3 reader (`PmTilesReader` + `ruzstd` + `image` + `mvt-reader` + `tiny-skia` deterministic MVT basemap style, demultiplied to straight RGBA8) in `crates/omo-engine/src/map/pmtiles_layer.rs`.
- [X] T035 [US2] Wire `map_layer_probe` and multi-layer `.pmtiles` tile rendering (with per-layer `layer_errors` isolation) in `crates/omo-worker/src/{server.rs,main.rs}` and `crates/omo-app/src/host/map_host.rs`.
- [X] T036 [US2] Render `OCB1` coastlines/borders in `canvas(Painter)` (z-order configurable below/above forecast tiles) and mount keyed `.pmtiles` tile layers with reactive `visible` and `opacity` bindings in `crates/omo-app/src/ui/map_view.rs`.
- [X] T037 [US2] Implement the **Layer Stack** panel (per-layer visibility checkbox, `0–100%` opacity slider, z-order up/down buttons, `"Add local PMTiles…"` picker + manual path input fallback, remove button, error & attribution display, and `"display only"` / hidden-forecast readout handling) in `crates/omo-app/src/ui/map_controls.rs`.

**Checkpoint**: User Stories 1 AND 2 work online and offline via `quickstart.md` `V1`–`V4`.

---

## Phase 5: User Story 3 - Atomic Composite Map Export & Project Persistence (Priority: P2)

**Goal**: Export settled multi-layer maps as a single composite PNG + atomic `.png.omo.json` (`omo-export-sidecar/3`) provenance sidecar, and persist/restore all Map view settings in `.omoproj` (`omo-project/1`) with crash-safe drafts and M1 backward compatibility.

**Independent Test**: Export a multi-layer map to PNG and verify its sidecar `/3` and `export-kill` steps 1–3 (`quickstart.md` `V5`); open `fixtures/projects/m1-roundtrip.omoproj`, customize and save `m2-roundtrip.omoproj`, verify byte-identical cross-platform round-trip hash and draft recovery (`quickstart.md` `V6`).

### Tests for User Story 3 (write first, confirm failing)

- [X] T038 [P] [US3] Write `.omoproj` M2 persistence tests (backward-compatible open of `fixtures/projects/m1-roundtrip.omoproj`, deterministic `-0.0 → 0.0` `MapViewState` round-trip hash, pre-deserialize `"omo-project/2"` `UnsupportedSchema` rejection, and draft recovery) in `crates/omo-project/tests/project_map.rs` (`SC-008`).
- [X] T039 [P] [US3] Write composite PNG export and `omo-export-sidecar/3` tests (layer z-order/opacity compositing, colorbar/station/attribution rendering, `/data/sha256` verification, and `export_scan::is_saved` `/3` acceptance) in `crates/omo-engine/tests/export_map_v3.rs` and `crates/omo-app/tests/export_scan.rs` (`SC-007`).

### Implementation for User Story 3

- [X] T040 [P] [US3] Implement `MapViewState`, `MapViewport`, `ColorScaleConfig`, and `MapLayerConfig` (with `-0.0 → 0.0` float canonicalization) in `crates/omo-project/src/map.rs`, and add `#[serde(default)] pub map: MapViewState` plus pre-deserialize schema major-version checking in `crates/omo-project/src/doc.rs`.
- [X] T041 [P] [US3] Implement synchronous settled-snapshot composite PNG rendering (`tiny-skia` compositing active `.pmtiles`, forecast field, `OCB1` coastlines, stations, colorbar legend, run/valid-time header, and attribution footer) in `crates/omo-engine/src/map/compose_export.rs`, and wire `format: "png"` + `omo-export-sidecar/3` staging/commit in `crates/omo-engine/src/export.rs` and `crates/omo-worker/src/main.rs`.
- [X] T042 [US3] Extend `omo-harness export-kill --at step1|step2|step3 --source map` in `crates/omo-app/src/bin/omo-harness.rs` (`SC-007`).
- [X] T043 [US3] Wire `ProjectDoc.map` signals into `ProjectController::{fingerprint, snapshot_doc, apply_doc}` in `crates/omo-app/src/ui/project_ctl.rs` so map edits update dirty state (`●`), 2 s / 30 s auto-drafts, crash recovery, and `.omoproj` save/open.
- [X] T044 [US3] Add `"Export Map PNG…"` (toolbar button + `File` menu item, settled-revision gating, overwrite confirmation, manual path fallback, sidecar `/3` builder, and `"saved → <path>"` badge) in `crates/omo-app/src/host/map_host.rs` and `crates/omo-app/src/ui/{map_controls.rs,dialogs.rs,workspace.rs}`.

**Checkpoint**: All three user stories (`US1`, `US2`, `US3`) are complete and independently testable.

---

## Phase 6: Polish & Cross-Cutting Acceptance

**Purpose**: Cross-platform CI smoke, fault injection, GUI visual evidence, documentation, workspace hygiene, and acceptance matrix completion.

- [X] T045 [P] Create `scripts/ci-verify-map.py` (`map_load_field` + 4-tile `map_render_tiles` + `map_inspect` + composite PNG `export`, reporting `skipped(network)` on remote outage and never a false pass) and add the step to `.github/workflows/ci.yml` on `linux_amd64`, `linux_arm64`, and `osx_arm64` (`SC-009`).
- [X] T046 [P] Write `scripts/fault-inject-map.py` and run ≥ 10 trials each on Spark (cancel during `map_load_field`, `kill -9` during field read and tile render verifying 0 leaked `g<old_gen>/` files, offline before request) plus `omo-harness export-kill --source map` steps 1–3, recording `docs/m2/evidence/V7-spark-aarch64-<date>.md` (`SC-007`).
- [X] T047 [P] Write `scripts/automate_m2_gui.py` and run isolated Xvfb X11 GUI verification on Spark covering `V1` (station framing, pan/zoom, readout), `V4` (LayerStack, offline coastlines, `.pmtiles`), `V5` (composite PNG export), and `V6` (`.omoproj` round-trip & draft recovery), saving screenshots and `docs/m2/evidence/V1-V6-spark-aarch64-<date>.md`.
- [X] T048 [P] Run `SC-002` cached pan/zoom latency benchmark (`< 100 ms` p95, `0` remote requests) and `SC-003` 57-station/edge-point `map_inspect` parity check against `fixtures/reference/o1280-*` on Spark, recording `docs/m2/evidence/SC-002-003-spark-aarch64-<date>.md`.
- [X] T049 [P] Update `README.md` and `README.en.md` with M2 tile map usage, `data_spatial`/`data_run` archive fallback, `.pmtiles` layer mounting, Natural Earth attribution, and `omo-export-sidecar/3`.
- [X] T050 Run `cargo +1.100.0 fmt --check`, `cargo +1.100.0 clippy --workspace --locked --all-targets`, `cargo +1.100.0 test --workspace --locked`, `cargo +1.100.0 test -p omo-app --all-features --locked`, and `scripts/check-boundaries.sh && scripts/check-boundaries.sh --self-test`, fixing any findings in touched files.
- [X] T051 Execute the full `specs/003-m2-regional-tile-map/quickstart.md` (`S0`, `S1`, `V1`–`V8`) and populate `docs/m2/acceptance.md` with observed results and evidence links for every row.
- [X] T052 After `T051` has complete recorded evidence, update `003` status in `.specify/memory/roadmap.md` to `implemented` (and `verified` once the user accepts), without committing or pushing unless explicitly requested.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup & Gate `S0-NSH-070`, `T001`–`T007`)**: Starts immediately. `T002` depends on `T001`; `T003`–`T006` run in parallel after `T001`, followed by `T007`.
- **Phase 2 (Foundational & Gate `S1-M2`, `T008`–`T020`)**: Depends on Phase 1. **BLOCKS all User Story phases until Gate `S1-M2` (`T020`) passes.**
- **Phase 3 (`US1` Forecast Tile Map MVP, `T021`–`T030`)**: Starts once `T020` passes.
- **Phase 4 (`US2` LayerStack, Coastlines & PMTiles, `T031`–`T037`)**: Engine tasks (`T031`–`T035`) can run **in parallel with `US1`** once Phase 2 completes; UI tasks (`T036`–`T037`) integrate cleanly onto `T029`–`T030`.
- **Phase 5 (`US3` Atomic PNG Export & `.omoproj`, `T038`–`T044`)**: `omo-project` persistence (`T038`, `T040`) can run **in parallel with `US1`/`US2`** once Phase 2 completes; composite export & UI wiring (`T039`, `T041`–`T044`) build on `US1` + `US2` renderers.
- **Phase 6 (Polish & Acceptance, `T045`–`T052`)**: Depends on Phases 3–5 completing (`T045`–`T049` run in parallel → `T050` → `T051` → `T052`).

### Parallel Opportunities Summary

- **Phase 1**: `T003` ∥ `T004` ∥ `T005` ∥ `T006` (4 completely disjoint file sets, before `T007` consolidates `fixtures/PROVENANCE.md`).
- **Phase 2**: `T008` ∥ `T009` ∥ `T010` ∥ `T011` ∥ `T012` (all 5 foundational test suites), then `T013` ∥ `T014` ∥ `T015` (3 disjoint crates).
- **Across Stories after Phase 2**:
  - `US1` rasterizer & scheduler (`T021`–`T026`) ∥ `US2` `OCB1` & `.pmtiles` engine modules (`T031`–`T034`) ∥ `US3` `.omoproj` `MapViewState` (`T038`, `T040`).
- **Phase 6**: `T045` ∥ `T046` ∥ `T047` ∥ `T048` ∥ `T049` (5 independent verification/doc tracks).

---

## Parallel Example: User Story 1 + Cross-Story Engine Tracks

```bash
# 1. Launch US1 tests in parallel (disjoint test files):
Task: "T021 [P] [US1] Write zero-seam and render-speed tests in crates/omo-engine/tests/map_tiles.rs"
Task: "T022 [P] [US1] Write PointReadout tests in crates/omo-engine/tests/map_inspect.rs"
Task: "T023 [P] [US1] Write worker protocol tests in crates/omo-worker/tests/m2_map_handlers.rs"
Task: "T024 [P] [US1] Write MapRequestScheduler tests in crates/omo-app/tests/map_scheduler.rs"

# 2. Launch disjoint engine/project implementations in parallel across US1, US2, US3:
Task: "T025 [P] [US1] Implement O1280 TrapezoidBilinear tile rasterizer in crates/omo-engine/src/map/raster.rs"
Task: "T033 [P] [US2] Implement OCB1 offline coastline reader in crates/omo-engine/src/map/coastlines.rs"
Task: "T034 [P] [US2] Implement sync mmap PMTiles v3 reader in crates/omo-engine/src/map/pmtiles_layer.rs"
Task: "T040 [P] [US3] Implement MapViewState in crates/omo-project/src/map.rs and crates/omo-project/src/doc.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete **Phase 1** (including Gate `S0-NSH-070` X11 smoke).
2. Complete **Phase 2** and **pass Gate `S1-M2` (`T020`)** (`SC-001` `< 30 s` median, `SC-004` `max |Δ| == 0.0`) — stop and consult user if missed.
3. Complete **Phase 3 (`US1`)** → validate interactively via `quickstart.md` `V1`–`V3` (end-to-end regional forecast tile map, smooth pan/zoom, valid-time stepping, exact `"native numeric"` cursor readout).

### Incremental Delivery

1. **MVP (`US1`)**: Forecast `z/x/y` tile map + numeric readout + `data_spatial`/`data_run` resilience.
2. **Multi-Layer & Offline (`US2`)**: Ordered `LayerStack` + built-in `OCB1` Natural Earth coastlines + local `.pmtiles` raster & MVT archives + `"display only"` provenance seam.
3. **Durability (`US3`)**: Settled composite PNG + `omo-export-sidecar/3` atomic export + `.omoproj` persistence & draft recovery.
4. **Acceptance (Phase 6)**: Fault injection, X11 GUI automation, cross-platform CI smoke, and `docs/m2/acceptance.md` closure.
