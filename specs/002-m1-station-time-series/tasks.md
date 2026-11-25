---
description: "Task list for M1 Stations × Forecast Series"
---

# Tasks: M1 Stations × Forecast Series

**Input**: Design documents from `specs/002-m1-station-time-series/`:
- [plan.md](plan.md)
- [spec.md](spec.md)
- [research.md](research.md)
- [data-model.md](data-model.md)
- [contracts/](contracts/)
- [quickstart.md](quickstart.md)

**Prerequisites**: plan.md and spec.md (required); research.md, data-model.md, and contracts/
are all present.

**Tests**: INCLUDED. The constitution (Development Workflow) makes tests mandatory for:
- bootstrap/probes;
- protocol framing and generation discard;
- cancellation;
- export commit/recovery;
- preview staleness.

It also makes fault injection part of milestone acceptance. Spec SC-003 requires an
independent reference. Write each test task before its implementation task and confirm the
test fails first.

**Organization**: Tasks are grouped by user story (spec.md US1–US6) so each story can be
implemented and tested on its own.

**Toolchain for every cargo command**: `cargo +1.100.0 … --workspace --locked`, except T001,
which is the one deliberate lockfile update.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel (different files, no dependency on an incomplete task)
- **[Story]**: US1…US6 from spec.md
- Paths are repo-relative

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Dependencies, module skeletons, and evidence locations.

- [X] T001 Add the new dependencies in Cargo.toml and crates/omo-project/Cargo.toml:
  - add `csv = "1"` to `[workspace.dependencies]` in Cargo.toml;
  - in crates/omo-project/Cargo.toml, add `csv`, `sha2`, `uuid`, and `ryu` (all workspace)
    and `tempfile` as a dev-dependency;
  - run `cargo +1.100.0 build --workspace` once without `--locked` to update Cargo.lock;
  - confirm `cargo +1.100.0 build --workspace --locked` then passes.
- [X] T002 [P] Create the omo-engine module skeletons under crates/omo-engine/src/ and register them in
  crates/omo-engine/src/lib.rs and crates/omo-engine/src/series/mod.rs, with doc comments
  pointing at research R-01/R-02:
  - crates/omo-engine/src/grid/mod.rs
  - crates/omo-engine/src/grid/reduced_gaussian.rs
  - crates/omo-engine/src/catalog/mod.rs
  - crates/omo-engine/src/catalog/run.rs
  - crates/omo-engine/src/series/plan_g.rs
- [X] T003 [P] Create crates/omo-protocol/src/series.rs and export it from
  crates/omo-protocol/src/lib.rs (empty types for now).
- [X] T004 [P] Create crates/omo-project/src/stations/mod.rs,
  crates/omo-project/src/stations/import.rs, and crates/omo-project/src/draft.rs, and register
  them in crates/omo-project/src/lib.rs.
- [X] T005 [P] Create docs/m1/evidence/README.md, using the evidence-file conventions of
  docs/m0/evidence (host, date, versions, command, result, artifacts). Create
  docs/m1/acceptance.md with one empty row per quickstart scenario: S1, V1–V9, SC-001…SC-009.
- [X] T006 [P] Extend scripts/check-boundaries.sh (and its `--self-test` seeded violations)
  to fail if `omo-app` resolves an HTTP client crate (`reqwest`, `ureq`, `hyper`, `isahc`) in
  any feature set (research R-03). Run `scripts/check-boundaries.sh --self-test`.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**:
- O1280 grid math;
- the **S1 feasibility gate**;
- catalog `/1` and the run resolver;
- protocol bodies;
- the PreviewKey extension.

**⚠️ CRITICAL**: No user-story work starts until T016 (S1 gate) passes. If S1 fails, STOP
and ask the user (spec Assumptions; research R-01). Do not substitute `ecmwf_ifs025`.

### Grid math (tests first)

- [X] T007 [P] Write row-table tests in crates/omo-engine/tests/grid_o1280.rs:
  - total points = 6,599,680;
  - nx(0) = 20, nx(1279) = 5136, nx(1280) = 5136, nx(2559) = 20;
  - lat(row 0) equals the Float32 producer rule `f32((N-y-1)*dy)+dy/2` with
    `dy = f32(180/f32(2N+0.5))`;
  - flat index ↔ (y, x) round-trips at the row boundaries;
  - longitudes wrap to [−180, 180).
- [X] T008 [P] Write nearest-search tests in crates/omo-engine/tests/grid_o1280.rs:
  - an exact grid point gives distance 0;
  - |lat| > 89.9 picks the polar rows;
  - lon ±180 and 359.99 wrap correctly;
  - an equidistant tie picks the smaller flat index;
  - for 10,000 deterministic-seed random stations, the result equals a brute-force search
    over rows ±3;
  - the maximum observed nearest distance is ≤ 7,200 m.
- [X] T009 Implement the O1280 row table in crates/omo-engine/src/grid/reduced_gaussian.rs:
  - `O1280::{points(), nx(y), row_offset(y), point(flat) -> (f32, f32), flat_index(y, x), row_near_lat(lat)}`;
  - evaluate the Float32 producer rule exactly (cite Open-Meteo `GaussianGrid.swift` and
    research R-01);
  - copy no code or data from duckomo (Constitution VII).
- [X] T010 In crates/omo-engine/src/grid/reduced_gaussian.rs, implement `nearest(lat, lon) -> Nearest{flat_index, lat, lon, distance_m}` in
  crates/omo-engine/src/grid/reduced_gaussian.rs:
  - check candidate rows ±2, and the two longitude neighbours in each row;
  - move the haversine function (radius 6,371,008.8 m) from
    crates/omo-engine/src/series/plan_a.rs to crates/omo-engine/src/grid/mod.rs and use it
    from both places;
  - add `max_nearest_distance_bound_m()`, the half-cell diagonal bound that T008 asserts.

  T007 and T008 must pass.

### Independent oracle tooling (dev-only, for SC-003)

- [X] T011 [P] Write scripts/o1280-nearest-oracle.py:
  - pure Python, no imports from crates or duckomo;
  - its own implementation of the producer row rule, asserting 6,599,680 points;
  - brute-force haversine over all points in rows ±3;
  - the same tie rule (smaller flat index);
  - input: a station CSV;
  - output CSV: `station_id,flat_index,grid_lat,grid_lon,distance_m`.

  Also create scripts/requirements-oracle.txt, pinning `omfiles` to the current release
  (record the version).
- [X] T012 [P] Write scripts/o1280-value-oracle.py: use the official `omfiles` reader to read
  `s3://openmeteo/data_run/ecmwf_ifs/<run>/temperature_2m.om` (anonymous HTTPS) at the flat
  indices from T011, map time indices through the run's `meta.json` `valid_times`, and output
  CSV `station_id,valid_time,value`. Record the object ETag in the output header comment.

### S1 spike and gate

- [X] T013 Implement the Plan G read core in crates/omo-engine/src/series/plan_g.rs:
  - `build_point_predicate(&[Nearest]) -> String`: an OR of `lat BETWEEN p±1e-4 AND lon BETWEEN p±1e-4`
    boxes, handling the antimeridian;
  - `read_points(conn: &BootstrappedConn, object_url, points, dims) -> Vec<PointRow{lat, lon, time_index, value}>`
    using `read_om(object_url, domain := 'ecmwf_ifs')`;
  - `verify_exact(points, rows)`: exactly one returned coordinate per selected point, Float32
    equality, otherwise `SeriesError::GridMismatch`;
  - make sure no `s3://`-scoped DuckDB secret is created (duckomo README pitfall).
- [X] T014 Extend crates/omo-worker/src/bin/omo-series-spike.rs with
  `--plan G --run <latest|YYYY-MM-DDTHHZ> --stations <csv> [--crosscheck N] [--select box|regions] [--repeat K]`.
  It prints one JSON report containing:
  - the object dimensions (via `read_om_raw`/DESCRIBE);
  - selected vs returned point counts;
  - the cross-check of N sampled flat indices: Rust `point()` vs duckomo `lat`/`lon`;
  - `duckomo_last_scan_metrics`;
  - wall time per repeat.

  `--select regions` uses `read_om_regions(method := 'points')` with tiny polygons as the
  fallback.
- [X] T015 [P] Create the station fixtures in fixtures/stations/:
  - fixtures/stations/m1-100.csv: 100 stations, header `id,name,lat,lon`, including Chinese
    names and some 0..360 longitudes;
  - fixtures/stations/m1-oracle.csv: ≥ 50 stations covering the research R-09 categories
    (poles, antimeridian, 0/360 wrap, midway between rows, exact grid points, the M0 set).

  Add PROVENANCE entries in fixtures/PROVENANCE.md.
- [X] T016 **GATE S1** (docs/m1/evidence/S1-*.md): on Spark, on the locked stack (DuckDB 2.0.0, duckomo v0.4.0), run
  T014 against the latest complete run for 1 and 100 stations
  (fixtures/stations/m1-100.csv from T015), with `--crosscheck 10000 --repeat 5`, and capture
  S1 (a)–(d) from research R-01. Write:
  - docs/m1/evidence/S1-spark-aarch64-<date>.md
  - docs/m1/evidence/S1-spark-aarch64-<date>.json

  Then decide:
  - if (b) or (c) fails with `box`, retry with `--select regions`;
  - if both fail, STOP and report to the user;
  - record the chosen select mode and the real `dimensions` in research.md R-01 as "S1
    outcome".

### Catalog and run resolution (tests first)

- [X] T017 [P] Write crates/omo-engine/tests/catalog.rs per contracts/catalog.md:
  - a valid `/1` loads;
  - reject: schema `/0` or `/2`, zero or two `is_default` entries, an unknown `layout.kind`
    or `grid.kind`, `default_window_h` beyond the cycle horizons, a missing `version`.
- [X] T018 [P] Write crates/omo-engine/tests/run_resolution.rs with JSON fixtures in
  crates/omo-engine/tests/data/ecmwf_ifs/. Fixtures:
  - `latest.json` pointing at a partial 00Z with 73 valid times;
  - a complete 18Z meta with 109 valid times;
  - a complete 00Z with 145;
  - a meta with the variable missing.

  Expectations:
  - the resolver picks the complete 18Z;
  - an explicit incomplete run gives `RunIncomplete`;
  - 8 misses give `NoCompleteRun`;
  - a missing variable gives `VariableMissing`;
  - `EffectiveWindow` for [0,168] on 18Z is [0,144] with `truncated = true`;
  - [0,168] on 00Z gives exactly 113 valid times;
  - the time-index → valid_time mapping follows the non-uniform steps.
- [X] T019 Implement crates/omo-engine/src/catalog/mod.rs per contracts/catalog.md and
  data-model CatalogEntry: `Catalog::load(path)` plus validation, `default_entry()`, and
  `CatalogEntry` path templating (`object_url(run)`, `meta_url(run)`). T017 must pass.
- [X] T020 Implement crates/omo-engine/src/catalog/run.rs:
  - a `MetaSource` trait (`latest()`, `meta(run)`);
  - `resolve(entry, RunRef) -> ForecastRun` using the cycle-horizon completeness rule and a
    6 h step-back over ≤ 8 candidates;
  - `effective_window(LeadWindow, &ForecastRun)`;
  - `time_index_table(&ForecastRun, &EffectiveWindow)`;
  - a DuckDB-backed `ConnMetaSource` that uses `read_text` over httpfs on `BootstrappedConn`
    and returns `CapabilityUnavailable(remote)` when `caps.remote = false`.

  T018 must pass.
- [X] T021 Replace catalog/products.json with `omo-catalog/1`, containing the
  `ecmwf_ifs.temperature_2m` default entry exactly as in contracts/catalog.md, with the
  attribution strings verbatim from research R-08. Version `2026-11-23.1`.

### Protocol and preview key (tests first)

- [X] T022 [P] Write crates/omo-protocol/tests/series.rs:
  - serde round-trips for the `SeriesRequest`, `series` result body, `catalog_check`,
    `resolve_run`, and `sql_session` bodies (contracts/worker-protocol-m1.md);
  - the new FailKinds `NoCompleteRun`, `RunIncomplete`, `GridMismatch`, `VariableMissing`,
    and `SourceChanged` serialize stably;
  - M0 bodies are unchanged.
- [X] T023 Implement the body types in crates/omo-protocol/src/series.rs and add the
  FailKind variants in crates/omo-protocol/src/state.rs. T022 and the existing
  crates/omo-protocol/tests/messages.rs must pass.
- [X] T024 [P] Write crates/omo-project/tests/preview_key.rs: changing `run_ref`, `window`,
  `max_distance_m`, `stations_hash`, or `catalog_version` changes key equality. Extend
  crates/omo-app/tests/preview_queue.rs so that a result for a superseded key (window
  changed) is dropped and the previous good result is marked stale.
- [X] T025 Extend `PreviewKey` in crates/omo-project/src/preview_key.rs with `run_ref`,
  `window`, and `max_distance_m`. In crates/omo-app/src/ui/workspace.rs, replace the
  hard-coded `catalog_version: "omo-catalog/0"` (two sites) with the loaded catalog version.
  T024 must pass.

**Checkpoint**: S1 passed, the grid/catalog/run/protocol tests are green, and story work can
begin.

---

## Phase 3: User Story 1 — Default forecast series for my stations (Priority: P1) 🎯 MVP

**Goal**:
1. Import a station CSV with mapping confirmation.
2. Run the default ECMWF IFS HRES 9 km 2 m temperature series (latest complete run,
   0–168 h, truncated and labelled for 06Z/18Z).
3. See the table, chart, matched points, and distances.

**Independent Test**: quickstart V1. On a fresh install, use fixtures/stations/m1-100.csv
with default settings:
- all 100 stations get 113 valid times (00Z/12Z run) or 109 with the truncation label
  (06Z/18Z run);
- distances are ≤ 7.2 km;
- the result badge reads "received, not saved".

### Tests for User Story 1

- [X] T026 [P] [US1] Write crates/omo-project/tests/stations_import.rs per
  contracts/station-csv-import.md:
  - comma and semicolon sniffing; ambiguous delimiter → `AmbiguousDelimiter`;
  - BOM; RFC 4180 quotes;
  - alias mapping proposal; incomplete mapping → `MappingIncomplete`;
  - all row issues reported with line numbers (duplicate, non-numeric, out of range);
  - longitude 0..360 normalization;
  - non-UTF-8 → `Encoding{byte_offset}`; empty input → `Empty`;
  - a golden canonical SHA-256 for fixtures/stations/m1-100.csv (the same on all CI
    platforms).
- [X] T027 [P] [US1] Write offline Plan G tests in crates/omo-engine/tests/plan_g.rs, with a
  fake point reader behind a trait:
  - the predicate builder handles the antimeridian;
  - the match report gives `BeyondMaxDistance` (max_distance_m tightened) and
    `InvalidCoordinate`;
  - `GridMismatch` when a point returns 0 or 2 rows;
  - the time-index join yields valid_time;
  - rows are sorted by (station_id, valid_time);
  - columns are exactly the C-10 `station_series` schema.
- [X] T028 [P] [US1] Write a network-gated `#[ignore]` integration test, crates/omo-worker/tests/series_live.rs, in
  crates/omo-worker/tests/series_live.rs. It sends a worker `series` request for 3 stations
  against the latest complete run and asserts:
  - 109 or 113 valid times per station, consistent with the run's cycle;
  - `distance_m ≤ 7200`;
  - `source_object` is the `s3://` URL and `object_version` is non-empty;
  - the `effective_window.truncated` flag is consistent with the cycle.
- [X] T029 [P] [US1] Extend crates/omo-app/tests/supervisor.rs:
  - a `series` job whose worker is killed mid-reading gives `WorkerCrashed`, generation + 1,
    and its result files are discarded;
  - a `series` job cancelled at interrupt level ends `Cancelled(Interrupt)` with the previous
    result kept.

### Implementation for User Story 1

- [X] T030 [US1] Implement station import in crates/omo-project/src/stations/import.rs and
  the `StationSnapshot`/`StationRow`/`ColumnMapping`/`RowIssue` types in
  crates/omo-project/src/stations/mod.rs (data-model StationSnapshot, research R-04):
  - parse, sniff, propose a mapping, validate, normalize;
  - canonical CSV bytes and `content_sha256`;
  - `write_canonical(dir) -> PathBuf` producing `stations-<sha256>.csv`.

  T026 must pass.
- [X] T031 [US1] Complete Plan G in crates/omo-engine/src/series/plan_g.rs, following the
  "Series processing order" in contracts/worker-protocol-m1.md:
  - `run(conn, stations, entry, run, window, max_distance_m) -> SeriesRun` (stations →
    nearest → read → time join → `StationSeriesRow`s + `StationMatch` list);
  - capture the object ETag before and after the read, and fail with `SourceChanged` if it
    differs.

  In crates/omo-engine/src/series/mod.rs, choose the plan from `entry.grid.kind`. In
  crates/omo-engine/src/series/schema.rs, add Parquet sibling and matches-file writers.
  T027 must pass.
- [X] T032 [US1] Add the `series` request kind in crates/omo-worker/src/server.rs and crates/omo-worker/src/main.rs:
  - crates/omo-worker/src/server.rs: dispatch next to `query`/`export`/`series_plan`, with
    the same ready/active guards;
  - crates/omo-worker/src/main.rs: `engine.series(&body)` resolves the run (T020), emits
    `progress` stages planning → reading → writing, checks interrupt between stages, writes
    the Arrow, Parquet, matches, and plot files under `<session_tmp>/g<gen>/`, and replies
    with the contract result body.

  T028 must pass when run with `--ignored` on Spark.
- [X] T033 [US1] Host submission in crates/omo-app/src/host/runtime.rs and
  crates/omo-app/src/host/supervisor.rs:
  - at submit, build an immutable `SeriesRequest` snapshot: canonical stations CSV via T030
    into `<session_tmp>`, a full `CatalogEntry` copy, `run: "latest_complete"`, window,
    `max_distance_m`;
  - route `progress` stages to the task model and the result to the preview/result store;
  - use the M0 three-level cancel (crates/omo-app/src/host/cancel.rs).

  T029 must pass.
- [X] T034 [US1] Create crates/omo-app/src/ui/import_view.rs:
  - CSV open dialog (`file_dialog` filter `csv`) plus manual path;
  - a preview of the first 20 rows;
  - one `Select` per field (id, lat, lon, name, elevation), preselected from the proposal;
  - an issues list with line numbers;
  - a confirm button, enabled only when the import is valid, that stores the snapshot in the
    document.

  In crates/omo-app/src/ui/workspace.rs, replace the source-panel `choose_stations_csv` flow
  with it. Parsing runs off the UI main thread (Constitution IV).
- [X] T035 [US1] Wire "Run series" in crates/omo-app/src/ui/workspace.rs:
  - the result appears in crates/omo-app/src/ui/table_view.rs, with a per-station series
    chart in crates/omo-app/src/ui/plot_view.rs;
  - a match-report panel lists `NoMatch` stations with reasons;
  - the result badge reads "received, not saved";
  - a truncation label "0–144 h of requested 0–168 h" appears when `effective_window.truncated`;
  - changing stations, catalog selection, or window marks the result out of date (T025 key).
- [X] T036 [US1] (docs/m1/evidence/V1-*.md) Run quickstart V1 on Spark (default flow, 100 stations, timing) and record
  docs/m1/evidence/V1-spark-aarch64-<date>.md. Update docs/m1/acceptance.md.

**Checkpoint**: US1 is the MVP. It is demonstrable on its own with default settings.

---

## Phase 4: User Story 2 — Catalog with readiness (Priority: P1)

**Goal**:
- the catalog lists entries with the default preselected;
- live readiness (data reachable, variable present, latest complete run identified);
- specific not-ready reasons;
- attribution shown.

**Independent Test**: quickstart V1 step 1 and V7 row 3:
- online, the default entry shows **Ready** with a run time;
- offline, it shows `NotReady(RemoteUnavailable)`, and "Run series" is disabled.

### Tests for User Story 2

- [X] T037 [P] [US2] Write crates/omo-engine/tests/catalog_readiness.rs with a fake
  `MetaSource` and a fake grid-info source:
  - `Ready{run}`;
  - `NotReady(RemoteUnavailable)` when `caps.remote = false`;
  - `VariableMissing`;
  - `NoCompleteRun`;
  - `GridMismatch` when the om_grid_info point count ≠ 6,599,680, or a sampled coordinate
    differs from `O1280::point`.

### Implementation for User Story 2

- [X] T038 [US2] In crates/omo-engine/src/catalog/mod.rs, implement `check_readiness(conn, entry)` in
  crates/omo-engine/src/catalog/mod.rs:
  - run resolution (T020);
  - `om_grid_info` cross-check: point count, plus 16 sampled flat indices vs
    crates/omo-engine/src/grid/reduced_gaussian.rs.

  T037 must pass.
- [X] T039 [US2] Add the `catalog_check` and `resolve_run` request kinds (crates/omo-worker/src/server.rs) in
  crates/omo-worker/src/server.rs and crates/omo-worker/src/main.rs, per
  contracts/worker-protocol-m1.md.
- [X] T040 [US2] In crates/omo-app/src/host/runtime.rs:
  - send `catalog_check` for every entry after each worker `ready`, and on user refresh;
  - store readiness per entry;
  - expose an `is_ready(entry_id)` signal source.

  "Run series" (T035) is enabled only when the selected entry is Ready.
- [X] T041 [US2] Create crates/omo-app/src/ui/catalog_view.rs and dock it in
  crates/omo-app/src/ui/workspace.rs. It shows:
  - entries, with the default preselected;
  - a readiness badge and the specific reason;
  - the latest complete run (UTC);
  - provider/model, variable and unit;
  - a time-step summary (hourly to 90 h, 3-hourly to 144 h, 6-hourly after);
  - per-cycle horizons;
  - the attribution strings verbatim.

**Checkpoint**: US1 + US2 cover both P1 stories.

---

## Phase 5: User Story 3 — Single- and multi-station exploration (Priority: P2)

**Goal**: select one or several stations, compare labelled curves, and narrow the lead-time
window within the run horizon.

**Independent Test**: quickstart V3:
- with one station, then five, the chart shows exactly those, labelled, and the legend
  toggles;
- with the window at 0–72 h, only those valid times appear, under a new preview key.

### Tests for User Story 3

- [X] T042 [P] [US3] Add host-level tests in crates/omo-app/tests/preview_queue.rs:
  - narrowing the window inside the received run's horizon produces a filtered view under a
    new key and does not resubmit;
  - widening beyond the horizon clamps, with `truncated = true`.

  Add a plot-sampling test in crates/omo-engine/tests/results.rs: more than 20 selected
  series are capped at 20 and labelled truncated.

### Implementation for User Story 3

- [X] T043 [P] [US3] Create crates/omo-app/src/ui/stations_view.rs:
  - a filterable checkbox list (NSH `Checkbox` + `virtual_list` + `input` filter, research
    R-07) driven by `signal<HashSet<String>>`;
  - select-all and clear;
  - the selection is persisted to `doc.view.selected_station_ids` (an edit increments the
    revision).
- [X] T044 [P] [US3] Create crates/omo-app/src/ui/window_control.rs:
  - an NSH `Slider` with `SliderValue::Range`, plus two `NumberInput`s for start_h/end_h;
  - clamp to the run horizon, with a visible note;
  - write `doc.window`.
- [X] T045 [US3] Extend crates/omo-app/src/ui/plot_view.rs:
  - one curve per selected station, labelled by station name or id;
  - a composed legend (swatch + `Checkbox` toggle per series);
  - the M0 cap of 20 series × 10,000 points, with an explicit truncation label.

  Wire T043 and T044 into crates/omo-app/src/ui/workspace.rs. Narrowing filters through a
  query over the received result; widening beyond the received range resubmits `series`.

**Checkpoint**: US3 works on any received result.

---

## Phase 6: User Story 4 — Export with provenance (Priority: P2)

**Goal**: export the series or SQL result to CSV/Parquet with sidecar
`omo-export-sidecar/2`. The UI shows "saved" only after commit.

**Independent Test**: quickstart V4:
- the sidecar `/2` fields match the result and the data hash verifies;
- `export-kill` steps 1/2/3 give incomplete, incomplete, finished.

### Tests for User Story 4

- [X] T046 [P] [US4] Extend crates/omo-engine/tests/export.rs:
  - a series-result export (CSV and Parquet) writes a sidecar `/2` containing `stations`,
    `catalog`, `run`, `window`, `plan`, a null `query`, and the attribution list from the
    catalog;
  - a SQL export sets `query`;
  - the EXDEV path still errors.
- [X] T047 [P] [US4] Extend crates/omo-app/tests/export_scan.rs: the saved predicate accepts
  both `/1` and `/2` sidecars, and an interrupted `/2` export is classified incomplete.

### Implementation for User Story 4

- [X] T048 [US4] Add `SIDECAR_SCHEMA_V2` (crates/omo-protocol/src/export.rs) and the `/2` field structs in
  crates/omo-protocol/src/export.rs. Implement `/2` sidecar writing in
  crates/omo-engine/src/export.rs:
  - series-result export reads the Parquet sibling via `COPY`;
  - SQL export records the query text;
  - the commit protocol is unchanged.

  T046 must pass.
- [X] T049 [US4] Make crates/omo-app/src/host/export_scan.rs accept `/1` and `/2`. T047 must
  pass.
- [X] T050 [US4] Wire export for series results in crates/omo-app/src/ui/dialogs.rs and
  crates/omo-app/src/ui/workspace.rs:
  - save dialog with `csv`/`parquet` filters and a suggested name;
  - reuse the existing overwrite confirmation;
  - the badge changes to "saved → <path>" only on `export_committed`.
- [X] T051 [US4] Extend `export-kill` in crates/omo-app/src/bin/omo-harness.rs with
  `--source series` (export a received series result). Run steps 1/2/3 on Spark and record
  docs/m1/evidence/V4-spark-aarch64-<date>.md.

**Checkpoint**: US4 is complete. Results can leave the app with provenance.

---

## Phase 7: User Story 5 — Save and reopen a project (Priority: P2)

**Goal**: an `omo-project/1` file with atomic save and open, a dirty indicator, 30 s and
debounced drafts, crash recovery, and refusal of newer versions.

**Independent Test**: quickstart V5:
- round-trip equality, including the stations hash;
- after a kill during an edit, the recovery prompt appears and the project file is
  byte-identical.

### Tests for User Story 5

- [X] T052 [P] [US5] Write crates/omo-project/tests/project_file.rs per
  contracts/project-file.md:
  - save → open round-trip equality of every persisted field;
  - no `.tmp` is left after save;
  - a newer schema major version is refused and the file bytes are unchanged (hash before and
    after);
  - an inline snapshot hash mismatch gives `CorruptSnapshot`;
  - a changed `orig_path` raises the informational notice;
  - opening the committed fixture fixtures/projects/m1-roundtrip.omoproj yields a golden
    stations hash (this covers SC-006 across CI platforms).
- [X] T053 [P] [US5] Write crates/omo-project/tests/draft.rs:
  - staging writes only under the drafts directory, never to the project path;
  - recovery candidates are drafts newer than the project mtime, or unsaved docs;
  - discard deletes the draft;
  - a successful save deletes the draft.

### Implementation for User Story 5

- [X] T054 [US5] Implement the `omo-project/1` model in crates/omo-project/src/doc.rs (all
  data-model ProjectDoc fields). Implement `save_atomic(path)`: tmp, fsync, rename, directory
  fsync, with EXDEV as an error. Implement `open(path)` with the version gate and snapshot
  hash check. Create fixtures/projects/m1-roundtrip.omoproj. T052 must pass.
- [X] T055 [US5] Implement drafts in crates/omo-project/src/draft.rs: `stage(doc, app_data)`,
  `list_recoverable(app_data)`, `discard`, `clear_after_save`. T053 must pass.
- [X] T056 [US5] Create crates/omo-app/src/host/project_io.rs:
  - save/open/draft I/O off the UI main thread;
  - draft scheduling: 30 s `set_interval` while dirty, a 2 s edit debounce, and on
    close-request;
  - a startup recovery scan.
- [X] T057 [US5] In crates/omo-app/src/ui/workspace.rs, add:
  - an application menu via `set_application_menu` with New, Open… (`omoproj` filter),
    Save, and Save As…, with cmd/ctrl shortcuts;
  - a reactive window title "name ●" while dirty;
  - an `on_close_press` guard offering Save / Discard / Cancel;
  - a recovery `alert_dialog` at startup.

  Record whether Cmd-Q/app-quit interception works (research R-07 UNVERIFIED) in
  docs/m1/evidence/V5-spark-aarch64-<date>.md, together with the V5 run.

**Checkpoint**: US5 is complete. Work persists across sessions and crashes.

---

## Phase 8: User Story 6 — SQL over stations and series (Priority: P3)

**Goal**: SQL over the `stations` and `station_series` views. Results show in the table and
chart, and errors keep the last result marked stale.

**Independent Test**: quickstart V6. A daily-max-per-station query matches the raw series for
a sample station.

### Tests for User Story 6

- [X] T058 [P] [US6] Write crates/omo-worker/tests/sql_session.rs, offline, using a small
  committed Parquet fixture crates/omo-worker/tests/data/series_small.parquet and a
  canonical stations CSV:
  - after `sql_session`, both views exist;
  - after a simulated worker restart and a new `sql_session`, they exist again;
  - the daily-max query gives the expected values.

### Implementation for User Story 6

- [X] T059 [US6] Add the `sql_session` request kind in crates/omo-worker/src/server.rs and
  crates/omo-worker/src/main.rs. It runs `CREATE OR REPLACE TEMP VIEW stations AS SELECT * FROM read_csv(<canonical>, columns := {...})`,
  and, when a series Parquet file is given,
  `CREATE OR REPLACE TEMP VIEW station_series AS SELECT * FROM read_parquet(<path>)`.
  T058 must pass.
- [X] T060 [US6] In crates/omo-app/src/host/runtime.rs, send `sql_session` after every
  `ready` and after every new series result. In crates/omo-app/src/ui/sql_view.rs, show query
  results in the table, and in the chart when there is a time column with numeric columns.
  Keep the last good result marked stale on error (existing preview_queue). Make sure Chinese
  IME input works in the SQL editor (it is exercised in T066).

**Checkpoint**: all six stories work independently.

---

## Phase 9: Polish, Acceptance & Cross-Cutting

**Purpose**: SC-003 reference accuracy, CI smoke test, fault injection, IME, performance,
docs, and roadmap.

- [X] T061 [P] Generate the SC-003 reference in fixtures/reference/:
  - run scripts/o1280-nearest-oracle.py and scripts/o1280-value-oracle.py on
    fixtures/stations/m1-oracle.csv for one pinned complete run;
  - commit the outputs to fixtures/reference/o1280-<run>/{nearest.csv,values.csv};
  - add a fixtures/PROVENANCE.md entry with the run, object ETag, script SHA-256 values, and
    the `omfiles` version.
- [X] T062 Write scripts/compare-o1280-oracle.py, comparing an app CSV export for the same
  pinned run (explicit `run`) against T061:
  - flat index and coordinates must match 100%;
  - values must match at 0.05 °C resolution, including null positions.

  Run it and record docs/m1/evidence/SC-003-spark-aarch64-<date>.md.
- [X] T063 Write scripts/ci-verify-series.py: build the worker, send `catalog_check` and a
  3-station `series` against the latest complete run, and assert the T028 invariants. A
  network failure reports `skipped(network)`, never pass. Add the step to
  .github/workflows/ci.yml for the `duckomo_platform` jobs (linux_amd64, linux_arm64,
  osx_arm64), per SC-009.
- [X] T064 (docs/m1/evidence/V7-*.md) Run quickstart V7 fault injection on Spark, with ≥ 10 trials of each:
  - cancel during reading;
  - `kill -9` of the worker during reading;
  - network down before a request;
  - network down during reading.

  Record docs/m1/evidence/V7-spark-aarch64-<date>.md (SC-005).

  **T064 closure (2026-11-25):** Cancel (10/10), `kill -9` (10/10), and network-down-before-request (10/10) passed in [V7-spark-aarch64-20261123.md](../../docs/m1/evidence/V7-spark-aarch64-20261123.md); `network-down-during-reading` was not run on the unprivileged Spark host and was accepted by user decision as a recorded scope boundary ([CI-CROSS-PLATFORM-20261125.md](../../docs/m1/evidence/CI-CROSS-PLATFORM-20261125.md)).
- [X] T065 (docs/m1/evidence/PERF-*.md) Measure SC-001 (interaction time) and SC-002 (100 stations, median of 5, cold
  cache, recorded network conditions) on Spark, and record
  docs/m1/evidence/PERF-spark-aarch64-<date>.md. If SC-002 misses 2 min, report the measured
  value to the user for a decision; do not silently change the target.
- [X] T066 (docs/m1/evidence/IME-*.md) Run the V8 Chinese IME regression on Spark X11 (IBus/libpinyin or Fcitx5) in the
  SQL editor and station filter, and record docs/m1/evidence/IME-spark-aarch64-x11-<date>.md
  (SC-008).

  **T066 closure (2026-11-25):** Closed by user decision on Chinese station-name rendering/processing in [V1-spark-aarch64-20261124.md](../../docs/m1/evidence/V1-spark-aarch64-20261124.md) plus prior M0 X11 IBus composition evidence (`N6`); no separate M1 interactive IME session was rerun ([CI-CROSS-PLATFORM-20261125.md](../../docs/m1/evidence/CI-CROSS-PLATFORM-20261125.md)).
- [X] T067 [P] Update README.md and README.en.md: M1 usage (import, catalog, series, export,
  project), data attribution, and the default product/run behaviour (truncated 06Z/18Z).
- [X] T068 [P] (scripts/check-boundaries.sh, workspace) Run `cargo +1.100.0 fmt --check`, `cargo +1.100.0 clippy --workspace --locked`,
  `cargo +1.100.0 test --workspace --locked`, and `scripts/check-boundaries.sh --self-test`.
  Fix findings in the touched files only.

  **T068 closure (2026-11-25):** On `3affd77` (`v0.1.0-m1`), `clippy --workspace --locked --all-targets` (warnings only), `test --workspace --locked` (97 passed, 0 failed, 1 ignored), and boundary checks/self-test pass; formatting diffs from `3affd77` are deferred to the next code change so `crates/` remains byte-identical to the CI/release-verified `v0.1.0-m1` tree.
- [X] T069 Run the full quickstart.md (S1, V1–V9). Fill docs/m1/acceptance.md with a result
  and an evidence link for every row.
- [X] T070 (.specify/memory/roadmap.md) Only after T069 has full evidence: update the 002 status in
  .specify/memory/roadmap.md to `implemented`, then to `verified` once the user accepts. Mark
  the T0xx boxes in this file. Do not commit or push unless the user asks (constitution
  workflow).

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (T001–T006)**: no dependencies. T001 comes first, because the lockfile change
  precedes `--locked` builds.
- **Foundational (T007–T025)**: depends on Setup.
  - T007/T008 → T009 → T010 → T013 → T014 → **T016 (gate)**; T015 (fixtures) feeds T016.
  - T017 → T019; T018 → T020 → T021.
  - T022 → T023; T024 → T025.
  - T011/T012 are independent and are needed by T061.
- **User stories (Phase 3+)**: all depend on Foundational, including a passed T016.
- **Polish (T061–T070)**: T061–T062 need US1 and US4; T063 needs US1 and US2; T064 needs US1;
  T066 needs US6 and US3; T069–T070 need everything.

### User Story Dependencies

| Story | Depends on | Notes |
|---|---|---|
| US1 (P1) | Foundational | the MVP |
| US2 (P1) | Foundational | independent of US1. T040 gates the US1 "Run series" button once both exist |
| US3 (P2) | US1 (needs a received result) | |
| US4 (P2) | US1 (series export), US6 (SQL export, optional) | the series-export path is testable with US1 alone |
| US5 (P2) | Foundational + T030 (station snapshot type) | otherwise independent of series |
| US6 (P3) | US1 (series Parquet) | can test with the committed Parquet fixture alone (T058) |

### Within Each Story

Tests first (they must fail), then engine/project, then worker, then host, then UI, then the
evidence run.

### Parallel Opportunities

- Setup: T002, T003, T004, T005, T006 in parallel after T001.
- Foundational:
  - T007 ∥ T008 ∥ T011 ∥ T012 ∥ T015 ∥ T017 ∥ T018 ∥ T022 ∥ T024;
  - then T019 ∥ T020 ∥ T023 ∥ T025 alongside the grid chain.
- After the gate, the following can proceed in parallel by different owners, since they touch
  different files:
  - US2 (T037–T041) ∥ US1 (T026–T036);
  - US5 project-crate work (T052–T055) ∥ US1.
- Within stories: all `[P]` test tasks, and T043 ∥ T044.

---

## Parallel Example: User Story 1

```bash
# Tests together (different files):
Task: "T026 stations import tests in crates/omo-project/tests/stations_import.rs"
Task: "T027 Plan G offline tests in crates/omo-engine/tests/plan_g.rs"
Task: "T028 live series test in crates/omo-worker/tests/series_live.rs"
Task: "T029 supervisor series crash/cancel tests in crates/omo-app/tests/supervisor.rs"

# Then crate-separated implementation in parallel:
Task: "T030 station import in crates/omo-project/src/stations/"
Task: "T031 Plan G pipeline in crates/omo-engine/src/series/plan_g.rs"
```

## Parallel Example: User Story 2 alongside User Story 1

```bash
Task: "T037 readiness tests in crates/omo-engine/tests/catalog_readiness.rs"
Task: "T041 catalog view in crates/omo-app/src/ui/catalog_view.rs"   # UI file distinct from US1's import_view.rs
```

---

## Implementation Strategy

### MVP First (User Story 1)

1. Phase 1 Setup.
2. Phase 2 Foundational. **Pass the S1 gate (T016) or stop and ask the user.**
3. Phase 3 US1 → validate with quickstart V1 → demo.

### Incremental Delivery

1. US1, the MVP.
2. US2, which completes P1: readiness and attribution.
3. US4 + US5, which make work durable: export, projects.
4. US3, refined exploration.
5. US6, SQL.
6. Phase 9 acceptance (SC-003 oracle, CI smoke test, fault injection, IME, performance),
   then roadmap update after the user accepts.

### Notes

- `[P]` means different files with no dependency on an incomplete task.
- Every evidence file follows Constitution VI (command, host, date, versions, output path).
- Never substitute another product, mix runs, zero-fill, or fall back silently from signed to
  unsigned.
- No commit or push without the user's explicit request.
