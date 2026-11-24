# Research: M1 Stations × Forecast Series

**Feature**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Date**: 2026-11-23

Inputs:
- four read-only subagent reports (2026-11-23):
  - duckomo v0.4.0 tag recon;
  - Open-Meteo bucket/source research;
  - OmO M0 code map;
  - NSH 0.5.1 capability scan;
- direct reads of public `s3://openmeteo` metadata (2026-11-23 13:05–15:07 +08:00).

Per Constitution VI, every fact below is either cited or marked **UNVERIFIED**. None of the
facts has yet been exercised on the locked stack (DuckDB v2.0.0 + duckomo v0.4.0 via
`omo-worker`). Spike S1 (R-01) is the gate that does that.

## R-01 Nearest-point sampling on O1280 (FR-010, SC-003)

**Facts**
- duckomo v0.4.0 `read_om_regions(method := 'nearest')` rejects the O1280 grid.
  - `src/grid/nearest_sampling.cpp:55-60` throws
    `[sampling_capability] nearest requires a spherical earth definition` and
    `... regular longitude/latitude grids only`.
  - The `ecmwf_ifs` earth is `wgs84_source`.
  - README.en.md:180 says "Nearest requires an eligible regular spherical grid".
- `read_om(path, domain := 'ecmwf_ifs')` is supported.
  - Output carries `lat`/`lon` columns derived from the row table.
  - `lat`/`lon` WHERE predicates are pushed down.
  - `docs/grid-domains.md:26` rates it `remote-benefit-validated`, but only for frozen HSURF
    and chunk_817 objects on Linux AArch64 with DuckDB 1.5.4–1.5.6. **DuckDB 2.0: UNVERIFIED.**
- `read_om_regions(method := 'points')` is experimental. Its Gaussian real-data behaviour is
  **UNVERIFIED**.
- `read_om_series` is compiled into the binary but documented as development-only. It rejects
  `data_run/` prefixes, so it cannot read per-run objects.
- The O1280 row table follows the Open-Meteo producer rule (`GaussianGrid.swift`; duckomo
  `scripts/generate-hres-o1280.py`):
  - N = 1280; rows y = 0…2559 run north to south.
  - Points per row: `nx(y) = 20 + 4·min(y, 2N−y−1)`.
  - `dy = 180/(2N+0.5)`; `lat_y = (N−y−1)·dy + dy/2`. These are equally spaced approximate
    latitudes, not Legendre roots.
  - `lon = x·360/nx(y)`, starting at 0° and wrapped to [−180, 180).
  - Flat index = Σ nx(<y) + x. Total 6,599,680 points.
  - The producer evaluates the rule in Float32. Our implementation MUST match it bit for bit
    for coordinates.
- Largest possible nearest distance, as a half-cell diagonal bound computed from the rule:
  **7.12 km** (row spacing 7.82 km).

**Decision — Plan G (reduced-Gaussian nearest)**
1. The worker computes each station's nearest O1280 point in pure Rust
   (`omo-engine::grid::reduced_gaussian`):
   - check rows within ±2 of the station's latitude row;
   - in each row, check the two longitude neighbours, using wrap-around;
   - choose the point with the minimum haversine distance (`sphere_haversine_f64_v1`, mean
     earth radius 6,371,008.8 m, the same as M0 Plan A);
   - break ties by the smaller flat index.
2. Read only the selected points with `read_om(<run object>, domain := 'ecmwf_ifs')`, using a
   predicate that is the OR of tight `lat`/`lon` boxes (±1e-4°) around each selected point. Join
   to stations in SQL on the exact point coordinates.
3. Cross-check on every run:
   - the coordinates duckomo returns MUST equal the Rust-computed coordinates (Float32
     equality);
   - exactly one returned point MUST match each selected point.

   A mismatch fails the request with `GridMismatch` and never falls back silently.
4. Default `max_distance_m = 7200`, the bound rounded up. It is recorded per request. Stations
   further away are reported as `no_match{reason: beyond_max_distance}`.
5. Plan B is kept for eligible regular grids. Plan G is used when the catalog entry's
   `grid.kind = reduced_gaussian_o1280`.

**Rationale**: Only the selection, which is cheap and host-computable, depends on the grid
class. The read path uses a function that is already validated for O1280. The data read is
proportional to the number of stations: ~100 chunks of 7 points × nTime (R-02).

**Alternatives considered**
- Plan B nearest: rejected by duckomo on O1280.
- `read_om_regions` points mode with tiny polygons: fallback if S1 shows that lat/lon OR
  predicates are not pushed down selectively. Its Gaussian behaviour is UNVERIFIED.
- Full-grid read plus SQL nearest: 578 MB per run; rejected for SC-002.
- `ecmwf_ifs025` substitution: rejected by the user (2026-11-23). If S1 fails, stop and ask.

**Spike S1 (gate, first implementation tasks)** — run on the locked stack through
`omo-worker` on Spark and record the outcome in `docs/m1/evidence/S1-*.md`:
- (a) the real dimension names and order of a full `data_run/ecmwf_ifs/…/temperature_2m.om`;
- (b) for 1 and 100 points, check that `read_om(domain)` with OR boxes returns exactly the
  selected points, and record `duckomo_last_scan_metrics` (body bytes, chunks decoded);
- (c) the Rust row table matches duckomo `lat`/`lon` for ≥ 10,000 sampled flat indices,
  including the poles and the antimeridian;
- (d) the 100-station × 113-step wall time from a cold cache (input to SC-002).

## S1 outcome (2026-11-23, Spark `aarch64-unknown-linux-gnu`, DuckDB v2.0.0, duckomo v0.4.0)

- **Real object columns (`read_om`, S1a)**: `value::FLOAT`, `lat::DOUBLE`, `lon::DOUBLE`,
  `valid_time::TIMESTAMP` (note: no raw `time` index column; Plan G matches `valid_time`
  against the run's `meta.json` `valid_times`).
- **Pushdown findings (S1b)**:
  - Single-station `lat BETWEEN p±1e-4 AND lon BETWEEN p±1e-4` pushes down cleanly to 1
    chunk (`data_bytes = 817`, `bytes_fetched = 2,123`, `read_requests = 17`, ~10.3 s).
  - Multi-station OR-of-boxes, `IN (...)`, global bounding box + join, and
    `read_om_regions(method := 'points')` all fall back to scanning/evaluating the full
    6.6M-point coordinate table (> 180 s).
- **Coordinate verification (S1c)**: `lon_f32` matches duckomo bit-for-bit using
  `dlon = 360.0f32 / nx; raw = x * dlon`. Sampled cross-check across the grid including
  polar rows (`0..19`, `6599660..6599679`) and antimeridian rows passed 100%.
- **100-station performance (S1d)**: Running parallel single-station `read_om` queries
  across 20 worker connections (`ParallelBox`) fetched 14,500 rows for 100 stations in
  **59.0 s** (`sum_bytes_fetched = 195,491 B`, ~1.95 KB/station, 17 HTTP requests/station),
  well under the 120 s SC-002 target.
- **Decision**: Proceed with **parallel per-station Plan G** (`single-box` pushdown across
  up to 20 worker connections).


## R-02 Open-Meteo object layout and run completeness (FR-005, FR-008, FR-012, FR-013)

**Facts**
- Bucket layout:
  - `data_run/ecmwf_ifs/YYYY/MM/DD/HHMMZ/<var>.om` holds one run per object. The open-data
    README describes it as "granular access to individual model runs". Runs are kept 3 months
    (`GenericVariableHandle.swift`). Runs from 2026-08-20 to 2026-11-23 were observed.
  - `data/ecmwf_ifs/<var>/chunk_N.om` is a rolling hourly series that mixes runs (past hours
    from earlier runs, future hours from the newest run). It cannot satisfy FR-012.
- Per-run object `[1, 6599680, nTime]`, chunk `[1, max(1, 1024/nTime), nTime]`, codec
  `pfor_delta2d_int16`. A full 00Z run has nTime = 145, so each chunk holds 7 points × 145
  steps. The 2026-11-22T00Z `temperature_2m.om` is 577,719,048 bytes. (Open-Meteo writer
  source; S3 listing; confirmed by HTTP HEAD on 2026-11-23: `Content-Length: 577719048`,
  `Last-Modified: 2026-11-22T06:48:38Z`, while that run's `meta.json` has `created_at`
  06:31:49Z.) Our dimension names are **UNVERIFIED** until S1(a).
- Cycles: 00Z/12Z reach +360 h (hourly 0–90, 3-hourly 93–144, 6-hourly 150–360); 06Z/18Z reach
  +144 h. Observed counts: 145 and 109 valid times (meta.json, 2026-11-22).
- Completeness:
  - `meta.json` has `reference_time`, `valid_times`, `variables`, `created_at`, and
    `temporal_resolution_seconds`, but **no completed flag**.
  - **Early partial uploads exist.** At about 07:00Z on 2026-11-23, `latest.json` pointed at
    23/0000Z with valid times only to +72 h (downloader `--short-cut-off-hour`). The early
    object can be overwritten later.
  - `data_spatial/ecmwf_ifs/latest.json` has `completed: true`, but it describes the spatial
    layout, not `data_run`.
- `temperature_2m` is in °C with scale factor 20 (0.05 K resolution).

**Decision — run resolution**
- The catalog entry declares the cycle horizons `{00: 360, 06: 144, 12: 360, 18: 144}` h.
- A run is **completely published** when its `meta.json` lists `variable ∈ variables` and
  `valid_times[-1] == reference_time + horizon(cycle)`.
- Resolver steps:
  1. Read `data_run/ecmwf_ifs/latest.json`.
  2. Test that run, then step back 6 h at a time, up to 8 candidates (48 h). Stop at the first
     complete run.
  3. If none is found, fail with `NoCompleteRun`.
- The default window is lead 0–168 h, so:
  - a 00Z/12Z run gives 113 valid times;
  - a 06Z/18Z run gives 0–144 h (109 valid times), labelled truncated (FR-013).
- The time index is mapped to `valid_time` with the run's `meta.json` `valid_times`, because
  the steps are non-uniform. The worker builds a small `(time_index, valid_time)` table and
  joins it.
- `source_object` = full `s3://` object URL. `object_version` = the object's ETag observed at
  read time. duckomo enforces ETag stability during a scan (`docs/official-httpfs.md`).

**Alternatives considered**
- `data/` chunks: they mix runs.
- `data_spatial/`: one object per time step, so 113 objects × full-grid tiles; kept only 7 days.
- Trusting `latest.json` alone: it can point at an early partial upload.

## R-03 Network access location (FR-007, FR-029; Constitution I, III)

**Decision**: All remote reads happen in the worker through the bootstrapped connection's
httpfs:
- OM objects;
- `latest.json` and `meta.json` via `read_text('s3://openmeteo/…')` / `read_json`.

`omo-app` gets no HTTP client. When httpfs is degraded (`caps.remote = false`), catalog checks
and series requests fail with the existing `CapabilityUnavailable(remote)`.

**Rationale**: This reuses M0's capability and degrade semantics, keeps all I/O off the UI
process, and adds no dependency. **UNVERIFIED** until S1: anonymous `s3://openmeteo` access
from the locked httpfs. duckomo README pitfall: an `s3://`-scoped secret causes 404s.

**Alternatives considered**: a host-side HTTP client (new dependency, a second network stack,
host I/O policy).

## R-04 Station CSV import (FR-001–FR-004)

**Decision**:
- Parse in `omo-project` with the `csv` crate. It stays UI-free and needs no DuckDB in the
  app.
- Accept UTF-8 with or without BOM. Sniff the delimiter from the header line (`,` or `;`).
  Handle quoted fields.
- Non-UTF-8 input fails with `ImportError::Encoding{byte_offset}`.
- Propose a mapping from case-insensitive header aliases:
  - id: `station_id`, `id`, `code`, `name_id`;
  - latitude: `lat`, `latitude`, `y`;
  - longitude: `lon`, `lng`, `longitude`, `x`;
  - name: `name`, `station_name`;
  - elevation: `elevation`, `elev`, `alt`, `altitude`, `height`.
- Validation reports every issue at once: empty input, duplicate ids, lat outside −90..90,
  lon outside −180..360. Longitudes in 0..360 are normalized to [−180, 180) (M0 convention).
- The snapshot is the canonical, normalized rows. The content hash is SHA-256 over canonical
  CSV bytes (fixed column order, LF, `ryu` float formatting).

**Rationale**: The mapping preview needs a synchronous, local parse with no worker round trip.
Canonical bytes make `stations_hash` stable across platforms (SC-006).

**Alternatives**: DuckDB `read_csv` sniffing in the worker (requires a round trip and is
nondeterministic across sniffer versions); M0 `read_stations` (strict 3-column; kept for
spikes).

## R-05 Project file and drafts (FR-024–FR-027)

**Decision**:
- Single JSON file `*.omoproj`, schema `omo-project/1`, containing:
  - the station snapshot inline (≤ 1,000 rows is small);
  - mapping and source provenance;
  - catalog selection (entry id + catalog version);
  - window (lead hours);
  - `max_distance_m`;
  - SQL tabs;
  - view settings (selected stations, chart options).
- Save writes `.<name>.omoproj.tmp`, fsyncs it, renames it over the target, then fsyncs the
  directory.
- Drafts:
  - written to `<app_data>/drafts/<doc_id>.json` every 30 s while dirty (NSH `set_interval`),
    and on close-request;
  - never written to the project path;
  - on start, any draft newer than its project's mtime (or for an unsaved doc) triggers a
    recovery prompt (NSH `alert_dialog`).
- A newer `schema` major version is refused read-only (FR-027).

**Alternatives**:
- SQLite or a directory bundle: more moving parts; results are not stored anyway.
- TOML: no benefit, and float round-trip is less predictable.

## R-06 SQL over stations and series (FR-015)

**Decision**:
- The worker writes each series result twice:
  - an Arrow IPC file (the existing UI paging path);
  - a sibling `.parquet` file.
- Before each SQL task the worker re-creates two temp views:
  - `stations` over the session's canonical stations CSV (`read_csv`, explicit schema);
  - `station_series` over the current result Parquet (`read_parquet`).
- After a worker restart, the views are re-created from those files.

**Rationale**: No extra DuckDB extension is needed, because Parquet and CSV are built in
(already used by M0 export). The views survive worker restarts.

**Alternative**: worker-only temp tables (lost on crash); Arrow scanning (would need a
non-locked extension).

## R-07 UI building blocks (FR-014, FR-025, FR-030) — NSH 0.5.1

Per the NSH 0.5.1 source scan:

| Need | NSH 0.5.1 | Evidence (nsh crates) |
|---|---|---|
| Multi-station selection | composition: `Checkbox` + `virtual_list`/`listbox` + `input` filter (`data_table` selects only one cell) | `checkbox.rs:83`, `data_table/mod.rs:187` |
| Multi-series chart + legend | the line plot exists; **no legend**, so it is built from swatches + `Checkbox` toggles | `plot/` |
| Menus / shortcuts | `set_application_menu`, `MenuItem::shortcut("cmd-s")`, `on_key` | `element/desktop.rs:1067-1217`, `ambient.rs:1532` |
| Dirty title | window `.title(reactive)` | `element/desktop.rs:525` |
| Draft timer | `set_interval(Duration, Fn)` | `ambient.rs:1282` |
| Unsaved-close guard | `WindowRef::on_close_press` with `close_on_press(false)` | `element/desktop.rs:346-351` |
| Dialogs | `file_dialog` filters/suggested name; `alert_dialog` | `file_dialog.rs:55-250`, `alert_dialog.rs:115` |
| Mapping form | `Select` per column | `select.rs:444` |
| Window range | `Slider` with `SliderValue::Range`, `NumberInput` | `slider.rs:18-48`, `number_input.rs:357` |

**UNVERIFIED**:
- whether `run_app`/`AppOptions` exposes a reactive main-window title and close interception;
- Cmd-Q / app-quit interception.

Mitigation: if quit cannot be intercepted, drafts are staged on every edit debounce (2 s) as
well as on the 30 s timer, so a quit loses at most 2 s of edits.

## R-08 Attribution (FR-009, FR-022)

**Decision**: The catalog entry carries these texts, which are shown in the catalog panel and
copied verbatim into every export sidecar:
- `"Weather data by Open-Meteo.com (CC BY 4.0, https://open-meteo.com/en/licence)"`;
- `"Copyright © <run year> European Centre for Medium-Range Weather Forecasts (ECMWF). Source: www.ecmwf.int. Licence: CC BY 4.0. ECMWF does not accept any liability whatsoever for any error or omission in the data, their availability, or for any loss or damage arising from their use."`;
- `"Modified: nearest-point sampling to user stations by omobench"`.

Sources: Open-Meteo licence page; ECMWF Terms of Use. Both are cited in the researcher report.

## R-09 Independent reference for SC-003

**Decision**: Build a dev-only oracle under `scripts/`, outside the app and outside duckomo:
- `scripts/o1280-nearest-oracle.py`:
  - pure Python;
  - brute-force haversine over **all points in rows ±3** of each station;
  - its own implementation of the producer row rule, double-checked by recomputing the point
    count (6,599,680);
  - emits a station → flat index / coordinates / distance table.
- `scripts/o1280-value-oracle.py`:
  - reads the same run object with Open-Meteo's official `omfiles` Python reader at those flat
    indices;
  - emits reference values. `omfiles` is a dev-only dependency, pinned in
    `scripts/requirements-oracle.txt`.
- Commit the reference outputs as fixtures with run time, object ETag, script hashes, and
  tool versions in `fixtures/PROVENANCE.md`.
- The station set (≥ 50) covers:
  - poles (|lat| > 89.9);
  - the antimeridian (lon ±180);
  - 0°/360° wrap;
  - points midway between rows;
  - exact grid-point coordinates;
  - the M0 station set.

**Rationale**: The oracle uses a different language, a different algorithm (brute force over
a wider window), and a different reader (official omfiles instead of duckomo). Tie-breaking
follows our contract (smaller flat index). Differences from Open-Meteo's own API point
selection are expected and not a failure.

## R-10 Performance targets (SC-002, station-set size)

**Estimate**:
- 100 stations ≈ 100 chunk reads of ~0.6 KB average compressed, plus trailer, metadata, and
  a LUT slice;
- the LUT is ≈ 943k chunks; its encoding and the size of the slices read are UNVERIFIED.

The provisional SC-002 target (< 2 min, median of 5, cold cache) is kept until S1(d) measures
it. 1,000 stations stays provisional, with a warning above it.

## Resolved Technical Context unknowns

| Unknown | Resolution |
|---|---|
| Sampling plan for O1280 | R-01 Plan G (gated by S1) |
| Which objects to read / run completeness | R-02 `data_run`, completeness by cycle horizon |
| Where network I/O lives | R-03 worker httpfs only |
| Station parsing location / format | R-04 `omo-project` + `csv` crate |
| Project format / drafts | R-05 JSON `omo-project/1`, atomic save, app-data drafts |
| SQL over results | R-06 Parquet sibling + temp views |
| UI feasibility | R-07 composition on NSH 0.5.1 |
| Attribution | R-08 |
| SC-003 reference | R-09 |
| Performance | R-10 (measured in S1) |

No `NEEDS CLARIFICATION` remains. Remaining **UNVERIFIED** items are each covered by S1 or a
named mitigation.
