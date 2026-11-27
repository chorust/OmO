# Feature Specification: M2 Regional Tile Map & Layer Stack (M2 区域瓦片地图与多图层栈)

**Feature Branch**: `003-m2-regional-tile-map` (spec directory only; no git branch was created)

**Created**: 2026-11-25

**Status**: Draft

**Input**: User decisions (2026-11-25): upgrade M2 (`003`) from a single static image to a standard Web Mercator (`EPSG:3857`) `z/x/y` slippy tile map backed primarily by Open-Meteo `data_spatial` single-timestep objects (with automatic slow-path fallback and notice to `data_run` for runs older than `data_spatial` retention); auto-fit imported station bounds on open while allowing free pan/zoom across wider regions; structure the map around an ordered multi-layer stack with built-in offline coastlines/borders plus user-mounted local `.pmtiles` archives and a value-provenance seam ready for future `radiust`-style color-to-value inversion; evaluate upgrading NSH from `=0.5.1` to `=0.7.0` as an M2 pre-gate under Constitution Principle II.

Sources of truth:
- omobench design v0.2 (2026-11-15) §7.5, with §4.2–§4.3, §5.1, §5.4, and §9 as referenced;
- constitution v1.3.0;
- roadmap 1.0.6, entry 003;
- M2 pre-spec primary-source research ([research-nsh-geospatial.md](../../docs/m2/research-nsh-geospatial.md));
- verified M0/M1 decisions:
  - R4 spatial pre-provisioning before configuration lock ([r4.md](../../docs/m0/decisions/r4.md));
  - channel policy ([channel.md](../../docs/m0/decisions/channel.md));
  - M1 O1280 grid definition and run-completeness contract ([002 spec](../002-m1-station-time-series/spec.md)).

## Context

M1 delivered the end-to-end station time-series workflow: users import a station CSV, inspect catalog readiness, compute 0–7 day nearest-grid series on the default 9 km ECMWF IFS HRES (`ecmwf_ifs`, reduced Gaussian O1280) product, query via SQL, export CSV/Parquet with provenance, and persist work in `.omoproj` project files. However, users currently have no spatial view of how the weather field behaves *around* and *between* their stations.

Design v0.2 §7.5 originally scoped M2 conservatively as a single static PNG around the station bounding box because pure-Rust NSH capabilities had not yet been audited. Pre-spec source research ([research-nsh-geospatial.md](../../docs/m2/research-nsh-geospatial.md)) confirmed two key facts:
1. Pure-Rust NSH (without QuickJS or WebView) supports zero-copy in-memory RGBA tile textures, bounded LRU texture caching, 2D vector canvas overlays, clipped absolute tile positioning, cross-boundary drag panning, and cursor-anchored wheel zooming.
2. Open-Meteo publishes `data_spatial/<domain>/<YYYY>/<MM>/<DD>/<HH>00Z/<valid_time>.om` objects that store one valid time per object, reducing regional `ecmwf_ifs` 2 m temperature transfer size by roughly two orders of magnitude compared with `data_run` (`~422 KB` vs `~117 MB` for a `30°×40°` region, and `4.48 MB` compressed globally).

On 2026-11-25 the user selected **Direction C**: build M2 on a standard **Web Mercator (`EPSG:3857`) `z/x/y` tile pyramid and ordered multi-layer stack** so that forecast fields pan and zoom smoothly from worker-cached numeric fields today, and external tile layers (including post-M2 raster layers with `radiust`-style color-to-value inversion) can plug into the exact same coordinate and layer model later.

### Source facts (observed 2026-11-25, public `s3://openmeteo` & upstream NSH)

- `s3://openmeteo/data_spatial/ecmwf_ifs/`:
  - Stores one `.om` object per valid time (`{YYYY}/{MM}/{DD}/{HH}00Z/{YYYY}-{MM}-{DD}T{HH}00.om`), bundling 42 variables hierarchically (~128 MB per valid-time object).
  - Binding a single variable explicitly (`variable := 'temperature_2m'`) issues only 4 metadata requests and reads `4,692,652 B` (`4.48 MB`) compressed for all `6,599,680` global O1280 points, or `432,527 B` (`422 KB`) for a `30°×40°` region; omitting `variable := ...` binds all 42 variables (`94` metadata requests) and must be avoided.
  - Retention is rolling (~7 days of runs), shorter than `data_run`'s ~3-month retention.
  - Even small `2°×3°` S3 reads incur multi-second HTTP round-trip latency (~7.6 s single-connection), so interactive tile panning/zooming cannot query remote storage per tile; the worker must prefetch/cache the numeric field for the selected valid time and render `z/x/y` tiles locally in memory.
- Upstream NSH releases (`0.5.1` → `0.7.0`, published 2026-11-25):
  - Both `=0.5.1` (currently pinned by Constitution Principle II) and `=0.7.0` have no built-in `Map` widget and support pure-Rust `AppSource::Rust` without QuickJS.
  - `=0.7.0` adds explicit HiDPI logical sizing on raw RGBA buffers, zero-copy opaque `CGImage` backing on macOS, and desktop trackpad pinch events. Evaluating `=0.7.0` is gated on evidence and a formal Constitution Principle II pin amendment during M2 planning/setup; M2 functional requirements do not depend on `=0.7.0`-only capabilities if `=0.5.1` is retained.

Technology names in this spec (NSH, duckomo, Open-Meteo OM, Web Mercator `EPSG:3857`, `.pmtiles`) reflect locked product constraints and user-selected interoperability standards, not internal implementation details.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Interactive forecast tile map around my stations (Priority: P1)

A user with imported stations opens the **Map** view, sees the map automatically framed around their stations (with a comfortable margin), and views a color-shaded Web Mercator (`EPSG:3857`) tile map of the selected catalog variable for a chosen valid time of the resolved forecast run, overlaid with crisp coastlines/borders, their station markers, and a color scale legend. They can drag to pan, scroll to zoom around the cursor, step to another valid time within the run, and hover or click anywhere on the map to read the exact coordinates and underlying forecast grid value. If they open a saved project whose pinned run is older than `data_spatial`'s ~7-day window, the workbench warns that fast spatial snapshots have expired, automatically falls back to reading from `data_run`, and renders the exact same valid time without silently changing the run.

**Why this priority**: Spatial context around user stations is the core value of M2, and backing it with a cached numeric field + `z/x/y` tile pyramid delivers smooth map navigation without per-tile remote network stalls.

**Independent Test**: Import the 100-station M1 fixture, switch to the Map tab, verify automatic station framing, pan/zoom smoothly across cached and newly exposed areas, inspect cursor values against station series values at the same valid time, and test both `data_spatial` and `data_run`-fallback paths plus cancel/crash/offline recovery.

**Acceptance Scenarios**:

1. **Given** a project with confirmed stations and a ready catalog entry, **When** the user opens the Map view and runs/loads the map, **Then** the viewport frames the bounding box of all in-projection stations plus margin, renders `256×256` forecast tiles for the default valid time (lead `0 h` of the resolved run, or the currently selected valid time), draws station markers and a color scale legend with unit, and displays the status `"received, not saved"`.
2. **Given** a loaded forecast map for a valid time, **When** the user drags to pan or scrolls the wheel to zoom within the cached field extent, **Then** previously visited `(z, x, y)` tiles reappear immediately from cache without blank-frame flicker, newly exposed tiles render locally without remote S3 reads, and station markers and vector boundaries stay aligned with the raster tiles at every zoom level.
3. **Given** a loaded regional map, **When** the user pans or zooms into an area outside the currently cached field extent, **Then** debounced background fetching extends or reloads the numeric field for the new extent, keeping existing tiles visible until the new tiles arrive.
4. **Given** a loaded forecast map, **When** the user moves the cursor over the map (or clicks a location/station), **Then** the map readout shows the cursor `(latitude, longitude)`, the sampled forecast value and unit from the underlying numeric grid (never reverse-read from display RGB pixels), the matched grid point distance, and the value provenance badge `"native numeric"` (or `"no data"` where the source value is missing/null).
5. **Given** a project pinned to a historical run whose valid-time object is absent (404) in `data_spatial` but present in `data_run`, **When** the map loads, **Then** the UI displays a clear notice that `data_spatial` retention has expired and slower `data_run` archive reading is in progress, produces the identical valid-time field from that pinned run without substituting another run, and records the actual `data_run` source object in provenance.
6. **Given** a running map load, **When** the user cancels it or the worker process is killed, **Then** cancellation/restart follows the M0/M1 supervision rules, no partial/corrupt field replaces a valid display, and any previously displayed map remains visible with a stale marker.
7. **Given** a displayed map, **When** the user changes the station set, catalog product/variable, run, valid time, or color scale parameters, **Then** the displayed map is immediately marked out of date (`stale`) until the new revision finishes loading, and if the new request fails (including offline), the previous map remains visible as stale alongside the error.

---

### User Story 2 - Multi-layer stack, offline coastlines, and local PMTiles layers (Priority: P1)

A user inspects and controls an ordered **Layer Stack** on the Map view: toggling visibility, adjusting opacity (`0–100%`), and reordering overlay/basemap layers. Out of the box—even with zero network access—the map displays a built-in offline coastline and country-border vector layer. Furthermore, the user can mount one or more local `.pmtiles` tile archives from disk (either pre-rendered raster tile archives or Mapbox Vector Tile basemap archives) into the same `EPSG:3857` `z/x/y` stack, verifying that omobench's tile engine cleanly hosts heterogeneous external layers and distinguishes numeric-capable layers from display-only layers.

**Why this priority**: The user explicitly chose the tile architecture so omobench can host additional tile layers today and support `radiust`-style color-decoded analytical raster layers in future milestones without redesigning the map stack.

**Independent Test**: Disconnect network access after loading a field (or with local fixtures), toggle/reorder the built-in coastline layer, station layer, forecast raster layer, and a mounted local raster `.pmtiles` plus an MVT `.pmtiles` file; verify opacity blending, invalid-archive error handling, attribution display, and `"display only"` readout provenance on non-numeric layers.

**Acceptance Scenarios**:

1. **Given** the Map view (online or offline), **When** no external basemap is configured, **Then** the built-in offline coastline and country-boundary layer renders crisply across zoom levels with zero network requests.
2. **Given** the Layer Stack panel, **When** the user toggles a layer's visibility, adjusts its opacity slider, or reorders custom/overlay layers, **Then** the map updates without re-fetching remote forecast data, and hiding/staling the forecast layer suppresses or marks stale its numeric cursor readout so users never misread hidden/stale values as current.
3. **Given** a valid local `.pmtiles` archive on disk containing `EPSG:3857` PNG, JPEG, or WebP raster tiles, **When** the user adds it as a layer, **Then** its `(z, x, y)` tiles align with the forecast and coastline layers across supported zoom levels, clamping cleanly beyond the archive's `minzoom`/`maxzoom`, and any attribution string in the archive metadata is surfaced in the UI.
4. **Given** a valid local `.pmtiles` archive containing `EPSG:3857` Mapbox Vector Tiles (MVT), **When** the user adds it as a basemap layer, **Then** polygon and line geometries render deterministically using a built-in basemap palette (water, land, boundaries, transportation lines) without requiring external style sheets, fonts, or network access.
5. **Given** a corrupt, non-existent, or unsupported-projection `.pmtiles` file, **When** the user attempts to add it (or reopens a project referencing a missing `.pmtiles` file), **Then** the workbench reports a clear per-layer error without crashing the worker or blocking the rest of the map layers.
6. **Given** cursor inspection when both the forecast layer (`native numeric`) and a raster `.pmtiles` layer are active, **When** the user inspects a point, **Then** the layer readout explicitly labels the forecast layer's value as `"native numeric"` and the raster `.pmtiles` layer as `"display only"` (no numeric value claimed).

---

### User Story 3 - Atomic composite map export and project persistence (Priority: P2)

A user who has framed a meaningful regional weather map exports it to a PNG file. The workbench waits until all visible layers for the current view and revision are completely rendered, composites them with the station markers, color scale legend, run/valid-time title, and required data/basemap attributions into a single PNG, and commits it atomically alongside its `.png.omo.json` provenance sidecar. Saving and reopening the `.omoproj` project restores the map viewport, selected valid time, color scale settings, and layer stack configuration.

**Why this priority**: Consistent with Constitution Principle IV and VI, every visual output must preserve the `received ≠ saved` boundary, full scientific provenance, and crash-safe atomic export.

**Independent Test**: Export a rendered multi-layer map to PNG, inspect the PNG visually and verify its `.png.omo.json` sidecar fields and SHA-256 hash; run `export-kill` fault injection at steps 1–3 with a map source and verify startup recovery classification; save and reopen the project and confirm map state round-trips identically.

**Acceptance Scenarios**:

1. **Given** a settled `"received, not saved"` map view, **When** the user exports to `<name>.png` (confirming overwrite if `<name>.png` or `<name>.png.omo.json` exists), **Then** the worker writes a single composite PNG containing the visible layers in z-order with configured opacities, station markers, color scale bar (palette, min/max, unit), run & valid-time header, and attribution footer, and commits `<name>.png.omo.json` atomically as the single commit point, updating the status to `"saved → <path>"`.
2. **Given** a map whose current revision is still loading or has missing in-flight tiles, **When** the user triggers export, **Then** export either waits for the current revision's visible tiles to settle or is disabled until settled—never committing a partially loaded or mixed-revision tile mosaic.
3. **Given** an export interrupted before sidecar rename, after data write, or after sidecar rename (`step1` / `step2` / `step3`), **When** the startup export scan runs, **Then** it classifies `step1` and `step2` as `incomplete` (`saved = false`, staging preserved) and `step3` as `finished` (`saved = true`), matching M0/M1 semantics.
4. **Given** a project with customized map viewport (`center`, `zoom`), selected valid time, color scale settings, and layer stack (including local `.pmtiles` references, visibility, opacity, and order), **When** the user saves and reopens the project, **Then** all map settings restore faithfully and canonical project hashing remains deterministic.

---

### Edge Cases

- **Zero stations imported**: Opening the Map view before importing stations displays a default continental/global overview (`zoom` 2–3) with the built-in coastline layer (and forecast/PMTiles layers if requested); the `"Fit to stations"` action is disabled with an explanatory tooltip until stations are confirmed.
- **Single station or coincident stations**: When all confirmed stations share identical coordinates (zero-area bbox), initial framing applies a minimum geographic span (default `±2.0°` latitude/longitude around the station) rather than zooming to an infinite/degenerate level.
- **Stations or panning across the antimeridian (`±180°` longitude)**:
  - Initial station bbox framing computes the **shortest longitudinal arc** (e.g., stations at `+178°` and `-178°` frame a `4°` Pacific window, not a `356°` world-spanning box).
  - Panning and forecast-field fetching across `±180°` split antimeridian queries into non-wrapping conjunctions so neither tiles nor station markers disappear at the seam.
- **Polar stations beyond Web Mercator limits (`|latitude| > 85.0511°`)**: Web Mercator tiles clamp at `±85.051129°`; stations beyond that latitude are never silently dropped—they are listed in a visible `"N polar station(s) outside Web Mercator bounds"` notice on the Map view while valid series workflows in M1 remain unaffected.
- **Both `data_spatial` and `data_run` unavailable for a requested run/valid time**: After at most one fallback attempt from `data_spatial` to `data_run`, the map task fails with a specific error (`run/valid time unavailable` or `remote data unavailable`), leaving any prior valid map visible as `stale`.
- **Rapid continuous pan/zoom**: Viewport-driven background field fetches are coalesced/debounced and supersede in-flight fetches so fast dragging never triggers an unbounded storm of remote requests.
- **Out-of-range valid time on project reopen or run switch**: If a saved valid-time offset exceeds a newly selected 06Z/18Z run's `+144 h` horizon, the Map view clamps or flags the valid time visibly rather than silently displaying a mismatched timestamp.

## Requirements *(mandatory)*

### Functional Requirements

**Coordinate system & tile pyramid**

- **FR-001**: The Map view MUST use the standard spherical Web Mercator (`EPSG:3857`) slippy tile pyramid (`2^z × 2^z` tiles of `256×256` logical pixels, integer zoom levels at least `z = 1..12`) so internal forecast layers, vector overlays, and external `.pmtiles` archives share one deterministic coordinate reference.
- **FR-002**: Initial Map view framing MUST automatically fit the shortest-longitude bounding box of confirmed in-projection stations plus padding when stations are present (using a minimum `±2.0°` span for a single/degenerate point, and a default overview when zero stations are present), while allowing the user to freely pan and zoom beyond the initial station bounding box, plus a `"Fit to stations"` control to return at any time.
- **FR-003**: Map navigation MUST support left-button drag panning (preserving drag tracking even if the pointer leaves the map bounds), cursor-anchored scroll-wheel zooming, double-click zoom-in, and explicit `+` / `-` / `"Fit to stations"` UI controls. Trackpad pinch-to-zoom MAY be enabled as progressive enhancement when supported by the pinned shell release.
- **FR-004**: Panning and zooming across cached `(z, x, y)` tiles MUST keep already-decoded visible tiles mounted without blank-frame flicker while newly exposed tiles render, and unmounted tiles MUST be bounded by an LRU memory budget so prolonged navigation does not leak memory.

**Forecast field loading (`data_spatial` primary + `data_run` fallback)**

- **FR-005**: The user MUST be able to select a single forecast **valid time** (UTC) from the valid times published by the currently resolved forecast run (defaulting to the run's initial valid time `lead 0 h`, or syncing with a user-chosen valid time).
- **FR-006**: To load a forecast map field for `(catalog entry, resolved run, valid time, variable)`, the engine MUST prefer Open-Meteo's single-timestep `data_spatial` layout (explicitly binding only the requested variable) when the catalog entry declares a `data_spatial` source and the object exists.
- **FR-007**: When the requested run/valid-time object is unavailable (404) in `data_spatial` (such as a pinned project run older than `data_spatial`'s ~7-day retention), the engine MUST notify the UI that fast spatial snapshots are unavailable for that run and automatically fall back once to the corresponding `data_run` object for the **exact same run, variable, unit, and valid time**—never substituting a different run or time step. If `data_run` is also unavailable, the task MUST fail with a clear reason.
- **FR-008**: All forecast tile rendering MUST run against a worker-cached numeric scalar field for the active `(run, valid time, variable)` so panning, zooming, opacity changes, and color-scale adjustments within the cached extent render locally in the worker without issuing per-tile remote S3 requests. When the user pans/zooms outside a regionally cached extent, background field extension requests MUST be debounced and cancellable.
- **FR-009**: Forecast tile sampling MUST support both reduced Gaussian O1280 grids (default `ecmwf_ifs` 9 km) and regular lon/lat grids, applying sufficient cross-tile sampling margin (gutter) when interpolating so adjacent `256×256` tiles show zero visible boundary seams, while preserving missing/null source cells as transparent/uncolored pixels.

**Color scale, station overlay & numeric point inspection**

- **FR-010**: The Map view MUST render a visible color scale legend showing the active palette name, numeric domain (`min..max`, auto-ranged once from the loaded field and shared uniformly across every `(z, x, y)` tile of that revision—never normalized per tile—with optional user min/max lock), and published physical unit (`°C` for the default `temperature_2m`).
- **FR-011**: Confirmed stations within Web Mercator bounds MUST render as distinct markers over the raster layers (reflecting current station selection/filter state from the workbench), and any stations with `|latitude| > 85.0511°` MUST be surfaced in an explicit out-of-projection count/notice rather than silently omitted.
- **FR-012**: Hovering or clicking on the map MUST display a coordinate and layer readout showing `(latitude, longitude)` in WGS 84 decimal degrees and, when the forecast layer is visible and current, the nearest-grid numeric value, unit, matched grid-point distance, and an explicit value-provenance label `"native numeric"` sampled directly from the worker's numeric field (never decoded from rendered RGB pixels). When the forecast layer is hidden or `stale`, the readout MUST suppress or explicitly flag the value as stale.

**Multi-layer stack, built-in coastlines & local `.pmtiles`**

- **FR-013**: The Map view MUST maintain an ordered **Layer Stack** supporting per-layer visibility toggle, opacity (`0%–100%`), status/error indicator, and deterministic z-ordering across:
  1. base/reference layers (built-in offline coastlines/borders and optional user-mounted `.pmtiles`);
  2. forecast scalar tile layer;
  3. station marker & inspection overlay.

  Users MUST be able to reorder multiple mounted `.pmtiles` / reference layers and toggle whether boundary lines draw beneath or above the translucent forecast raster layer.
- **FR-014**: The workbench MUST ship a built-in offline coastline and admin-0 boundary dataset derived from Public Domain data (Natural Earth `1:50m` regional scale) that renders cleanly across `z = 1..12` (providing smooth regional reference through `z = 8`, with local `.pmtiles` available for higher-zoom detail) with zero network access.
- **FR-015**: Users MUST be able to add and remove local `.pmtiles` v3 archives from disk (`EPSG:3857` Web Mercator):
  - **Raster `.pmtiles`** (`png`, `jpeg`, `webp` tiles): decoded and composited into the `(z, x, y)` stack, respecting the archive's `minzoom`, `maxzoom`, and bounds.
  - **Vector `.pmtiles`** (Mapbox Vector Tiles, `.mvt`, raw or gzip/zstd-compressed per PMTiles v3): rendered deterministically in pure Rust using a fixed built-in basemap geometry style (land/water polygons, boundary lines, transportation lines) without external style JSON, remote fonts, or custom sprite sheets.
- **FR-016**: Corrupt, unreadable, missing, or non-Web-Mercator `.pmtiles` files MUST be rejected or marked errored per layer with a specific diagnostic message without crashing the worker or preventing other layers from rendering.
- **FR-017**: Every layer in the Layer Stack MUST declare its **value provenance capability**: `"native numeric"` for numeric forecast fields and `"display only"` for built-in vector boundaries and M2 `.pmtiles` layers (reserving `"palette decoded"` in the data model/protocol for post-M2 color-to-value inversion layers without implementing color inversion in M2).
- **FR-018**: Required attributions—Open-Meteo (CC BY 4.0), upstream model provider (ECMWF), omobench modification notice, and any attribution metadata embedded in active `.pmtiles` archives—MUST be visible on the Map view and recorded in every map export provenance sidecar. No online third-party XYZ tile servers (such as `tile.openstreetmap.org`) may be contacted.

**Tasks, supervision, export & project persistence**

- **FR-019**: Field loads, tile batch renders, and map exports MUST execute in the supervised child worker process under existing generation, preview-revision, and 3-level cancellation (`Interrupt` → `Drain` → `Kill`) contracts; stale-generation artifacts (including temporary map/tile files) MUST be cleaned up automatically on generation rollover and exit.
- **FR-020**: Map previews MUST bind to project revision, station snapshot hash, catalog version, resolved run, selected valid time, variable, color scale, active layer stack, and worker generation, marking the displayed map `stale` immediately on any relevant input change and retaining the last valid map on failure.
- **FR-021**: Users MUST be able to export the current settled map view to a single composite PNG file accompanied by an atomic provenance sidecar (`<target>.png.omo.json`). Submitting an export MUST capture an immutable snapshot of `(revision, viewport, layer stack, color scale)` so concurrent pan/zoom or layer edits cannot desynchronize the PNG from its sidecar, and export MUST NOT commit a partially loaded or mixed-revision mosaic; existing targets require overwrite confirmation, the sidecar rename is the single commit point, and startup scan MUST classify interrupted map exports identically to M0/M1 (`incomplete` vs `finished`).
- **FR-022**: The map export provenance sidecar MUST record:
  - station snapshot hash and count (when stations are present);
  - catalog entry, catalog version, resolved run reference time, and selected valid time (UTC);
  - actual forecast source layout (`data_spatial` vs `data_run`), source object URL, and observed object version/ETag;
  - viewport geographic bounds, `EPSG:3857` zoom level, output pixel dimensions, and color scale (`palette`, `min`, `max`, `unit`);
  - active layer stack manifest (including each local `.pmtiles` path, SHA-256 or size/mtime fingerprint, opacity, and metadata attribution);
  - engine/extension versions, distribution channel, PNG SHA-256 hash, and full attribution list.
- **FR-023**: `.omoproj` project save/open and crash-recovery drafts MUST persist and restore the Map view state (viewport `center`/`zoom`, selected valid time or lead offset, color scale settings, and layer stack definitions including local `.pmtiles` paths, visibility, opacity, and order) with backward-compatible opening of M1 project files (initializing default M2 map settings when absent).

### Key Entities

- **Forecast Field Cache**: A worker-resident, generation- and revision-scoped numeric scalar representation of one `(catalog entry, run, valid time, variable)` loaded from `data_spatial` (or `data_run` fallback), recording source object URL, ETag/version token, geographic coverage, min/max statistics, and supporting fast local `(z, x, y)` tile rasterization plus exact nearest-grid point inspection.
- **Layer Stack & Map Layer**: An ordered collection of spatial layers rendered in `EPSG:3857` `(z, x, y)` tile coordinates. Each layer carries an identifier, kind (`forecast_field`, `builtin_coastlines`, `local_pmtiles_raster`, `local_pmtiles_mvt`, `station_overlay`), visibility, opacity (`0.0..=1.0`), value-provenance capability (`native_numeric` | `display_only`, reserving `palette_decoded` for future milestones), readiness/error state, and attribution strings.
- **Local PMTiles Source**: A user-selected local `.pmtiles` v3 file validated for header integrity, tile type (raster `png`/`jpeg`/`webp` or `mvt`), zoom range (`minzoom..maxzoom`), geographic bounds, and embedded attribution metadata.
- **Point Readout**: The result of inspecting a `(latitude, longitude)` location on the map, pairing coordinates with per-layer value provenance (`native_numeric` forecast value + unit + matched grid point distance, or `display_only` / `stale` / `no data`).
- **Composite Map Export**: A settled single-file PNG rendering of the active viewport, layers, station markers, color scale legend, timestamp header, and attribution footer, paired with its atomic `.png.omo.json` provenance sidecar.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On the Spark reference host with recorded network conditions, loading the default regional forecast map (`ecmwf_ifs` 9 km · `temperature_2m`, 100-station regional `30°×40°` M2 fixture extent) from `data_spatial` reaches a settled tile display in a median of **under 30 seconds** (relaxed from 15 s by user decision after Gate `S1-M2` measured a 24.07 s median; see `docs/m2/evidence/S1-spark-aarch64-20261126.md`) across 5 cold-field runs, with bytes read and HTTP request counts recorded via scan metrics.
- **SC-002**: Once a forecast field is cached in the worker for the active valid time, panning and zooming within the cached extent renders newly exposed `256×256` tiles in **under 100 ms** per visible viewport batch (p95 on the reference host) with **0 remote network requests** and **0 blank-frame flashes** on already-mounted tiles.
- **SC-003**: For a reference set of inspection coordinates (including exact station locations, off-grid points, antimeridian-adjacent points, and null/masked cells) at a pinned run and valid time, **100%** of Map point-readout `"native numeric"` values and matched grid coordinates equal the M1 `station_series` / independent O1280 oracle values (`max |Δ| == 0.0`).
- **SC-004**: For a pinned run available in both `data_spatial` and `data_run`, forcing the `data_run` fallback path produces **bit-identical** numeric field values across the test region (`max |Δ| == 0.0`) while surfacing the fallback notice and recording the `data_run` object URL in provenance.
- **SC-005**: With network access disabled (`bwrap --unshare-net` or degraded `httpfs`), **100%** of local map operations—built-in coastline rendering, mounting and panning/zooming local raster and MVT `.pmtiles` fixtures, layer toggling/opacity adjustments, inspecting an already-cached field, and exporting a settled map—succeed without error while new remote forecast loads fail cleanly with `"remote data unavailable"` and retain stale state.
- **SC-006**: Adjacent `256×256` forecast tiles across interior boundaries and across the `±180°` antimeridian exhibit **0 seam discontinuities** (verified by comparing tiled output pixel-for-pixel against a single-pass reference render of the same `EPSG:3857` viewport within ±1 LSB rounding).
- **SC-007**: Under fault injection on the reference host (≥ 10 trials each of cancel during field read, `kill -9` of worker during field read or tile render, offline before request, and `export-kill` at steps 1–3 with a map source), **100%** of trials preserve state-boundary invariants (`received ≠ saved`, stale map retained on failure, startup export scan classifies `incomplete`/`incomplete`/`finished`, and zero leaked stale-generation tile files after rollover).
- **SC-008**: Saving and reopening `.omoproj` projects (both newly saved M2 projects with custom viewport/valid-time/color-scale/PMTiles layer stacks and existing M1 project files) succeeds in **100%** of round-trip tests across supported platforms with deterministic canonical hashes.
- **SC-009**: Interactive GUI map acceptance (framing, pan/zoom, layer stack, `.pmtiles` mount, readout, export) is verified with recorded visual evidence on the Spark reference host (`linux_arm64`), and `linux_amd64` and `osx_arm64` pass GitHub Actions CI and release workflows including an automated M2 worker map/tile smoke test (Constitution 1.3.0).

## Assumptions

- **Shell version pre-gate (`NSH =0.5.1` vs `=0.7.0`, Constitution Principle II)**:
  - Constitution Principle II currently pins `nsh-* =0.5.1` and requires recorded evidence plus a ratified amendment for any pin upgrade.
  - Per user decision (2026-11-25), `/speckit.plan` includes an explicit pre-implementation gate (`S0-NSH-070`) to evaluate upgrading all `nsh-*` crates from `=0.5.1` to official release `=0.7.0` (keeping `default-features = false`, zero QuickJS/WebView linkage, verified by `check-boundaries.sh`, M0/M1 regression tests, and Spark GUI smoke).
  - If `S0-NSH-070` passes and the user ratifies the Constitution Principle II pin amendment, M2 ships on `=0.7.0` (gaining HiDPI `ImageSrc::size`, macOS zero-copy opaque `CGImage`, and progressive-enhancement trackpad pinch zoom). If any blocker appears, M2 proceeds on `=0.5.1` without changing any mandatory FR or SC in this spec.
- **Forecast source & retention**:
  - Open-Meteo `data_spatial` is the primary single-timestep map source; `data_run` is the automatic archive fallback when `data_spatial` returns 404 for an older pinned run.
  - Only single-timestep scalar map rendering is in M2; stitched multi-run fields are prohibited just as in M1.
- **PMTiles v3 scope in M2**:
  - Local file paths only (`.pmtiles` v3 on local filesystem); HTTP range-hosted remote `.pmtiles` URLs are excluded in M2 to preserve strict capability/offline boundaries.
  - MVT vector `.pmtiles` rendering uses a deterministic built-in geometry style (land/water fills, boundary/road strokes) suitable for geographic reference; MapLibre Style JSON parsing, text label placement/collision engines, and custom icon sprite sheets are explicitly out of scope for M2.
- **Extensibility seam for post-M2 analytical layers**:
  - M2 establishes the `LayerStack` and `value_provenance` (`native_numeric` | `display_only`, reserving `palette_decoded`) contracts so post-M2 milestones can mount external scientific/radar raster tile layers with `radiust`-style validated palette/gray-code color-to-value inversion without breaking M2's UI, worker protocol, or project schema.
- **Out of scope for M2**:
  - Implementing color-to-value inversion (`palette_decoded`) on external raster tiles (post-M2);
  - Scraping online third-party XYZ raster/vector tile servers (`tile.openstreetmap.org`, commercial map APIs);
  - MapLibre Style JSON, MVT text label collision/shaping, 3D terrain/globe projections, or wind-particle animations;
  - Multi-timestep map animation loops or video/GIF export;
  - Declarative or signed TSX dynamic reports (L1 `004` / L2 `006`);
  - Dynamical Icechunk data sources (M3 `005`);
  - Any modification to neighbouring repositories (`Duckling`, `duckomo`, `radiust`).
