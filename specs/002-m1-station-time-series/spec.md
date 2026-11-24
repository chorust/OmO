# Feature Specification: M1 Stations × Forecast Series (M1 站点 × 预报时序)

**Feature Branch**: `002-m1-station-time-series` (spec directory only; no git branch was created)

**Created**: 2026-11-23

**Status**: Draft

**Input**: User description: "下一步 M1：真实目录接入、站点 CSV 导入、单/多站时间序列工作流与工程持久化；默认用 ECMWF HRES 2m temp 未来 0～7 天。"

Sources of truth:
- omobench design v0.2 (2026-11-15) §7.4, with §4.2–§4.5 and §5.4 as referenced;
- constitution v1.3.0;
- roadmap 1.0.5, entry 002, including the OQ-4 and OQ-6 resolutions of 2026-11-23;
- the verified feature 001 decisions:
  - Plan B nearest sampling ([series-plan.md](../../docs/m0/decisions/series-plan.md));
  - channel policy ([channel.md](../../docs/m0/decisions/channel.md));
  - NSH 0.5.1 accepted ([shell.md](../../docs/m0/decisions/shell.md)).

## Context

M0 delivered a verified engine, a supervised worker, an NSH workbench shell, and decisions on
the nearest-series plan, the distribution channel, and the shell. It shipped no end-user
forecast workflow: the catalog is an empty `omo-catalog/0` placeholder, station import is a
file picker without mapping or snapshotting, and there is no project file.

M1 is the first milestone with an end-user outcome. A forecaster or analyst imports their own
station table and gets nearest-grid forecast time series for those stations. They can browse,
chart, query, and export the series with provenance, save the work as a project, and reopen
it later. At no point can they mistake an unsaved result for a saved one.

The default forecast is **ECMWF IFS HRES (deterministic), 2 m air temperature, forecast
horizon 0–7 days** (OQ-6, user decision 2026-11-23).

### Source facts (observed 2026-11-23, public `s3://openmeteo` metadata)

- `ecmwf_ifs` (IFS HRES 9 km):
  - Grid: **reduced Gaussian O1280**, not a regular lon/lat grid.
  - Time step: hourly to +90 h, 3-hourly to +144 h, 6-hourly after that.
- `ecmwf_ifs025` (IFS HRES on a regular 0.25° WGS 84 grid):
  - Time step: 3-hourly to +144 h, 6-hourly after that.
- Run horizons, for both products:
  - 00Z and 12Z runs reach +360 h;
  - 06Z and 18Z runs reach only **+144 h (6 days)**.
- The M0-selected Plan B (`read_om_regions` nearest) is documented by duckomo for eligible
  **regular spherical** grids only. Its applicability to O1280 has not been demonstrated.
- duckomo registers the `ecmwf_ifs` domain for O1280. Its coordinates and values are recorded
  as validated against Open-Meteo's published objects (duckomo `docs/grid-domains.md`). Reading
  O1280 is therefore supported; nearest-point selection on it is the open part.

Technology names in this spec (NSH, duckomo, Open-Meteo OM) are **locked product constraints**
(constitution I–III, roadmap C-02..C-10), not implementation choices this spec makes.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Default forecast series for my stations (Priority: P1)

A user opens omobench, imports a CSV of their stations, confirms which columns hold the
identifier, latitude, and longitude, and gets a 2 m temperature forecast for every station
from the latest available ECMWF IFS HRES (9 km) run, covering 7 days of lead time (6 days,
labelled as truncated, when the latest run is a 06Z or 18Z run). The result appears as
a table and a line chart, and the user can tell which grid cell each station was matched to
and how far away that cell is.

**Why this priority**: This is the product's core promise. Every other M1 capability (export,
projects, SQL) operates on this result. On its own it is a viable MVP: a user can answer "what
is the forecast at my stations?".

**Independent Test**: Start from a fresh install with a 100-station CSV and network access.
Import the CSV, confirm the mapping, and request the default series without changing anything
else. Verify that a table and chart appear for all stations, with valid times, values, and
the matched grid location and distance for each station.

**Acceptance Scenarios**:

1. **Given** a CSV with columns `id,name,lat,lon` and 100 rows, **When** the user imports it,
   **Then** the system proposes a mapping (id→identifier, lat→latitude, lon→longitude), shows a
   preview of the first rows, and waits for the user to confirm before creating the station
   set.
2. **Given** a confirmed station set and the default catalog selection, **When** the user runs
   the series, **Then** for every station the result contains the forecast values for each
   valid time in the 0–7 day window of the selected run, the matched grid latitude/longitude,
   and the station-to-cell distance.
3. **Given** a running series request, **When** it completes, **Then** the result is marked
   "received, not saved" until it is exported.
4. **Given** a station whose coordinates fall outside the product's grid coverage or beyond
   the maximum matching distance, **When** the series runs, **Then** that station is reported
   as "no match" with the reason, and all other stations still receive their series.
5. **Given** a running series request, **When** the user cancels it, **Then** the request ends
   as cancelled, no partial result is shown as current, and the previous result (if any) stays
   visible.
6. **Given** a running series request, **When** the worker process is killed, **Then** the
   request is reported as failed, the worker restarts automatically, no result from the old
   worker generation is displayed, and the user can retry.
7. **Given** a displayed result, **When** the user changes the stations, catalog selection, or
   window, **Then** the displayed result is marked out of date until a new request completes.
8. **Given** no network access, **When** the user runs the series, **Then** the request fails
   with "remote data unavailable", and the last successful result stays visible with a stale
   marker.
9. **Given** the latest completely published run is a 06Z or 18Z run (horizon +144 h), **When**
   the user runs the default series, **Then** the result covers lead times 0–144 h from that
   run and is labelled "0–144 h of requested 0–168 h". No values from another run are added.

---

### User Story 2 - Catalog with readiness (Priority: P1)

A user opens the catalog and sees the available products and variables, with the default
(ECMWF IFS HRES · 2 m temperature) preselected. Each entry shows whether it is ready to query
right now: data reachable, variable present, and the latest run identified. Entries that are
not ready say why.

**Why this priority**: The series workflow depends on a correct, verifiable product
definition. M0 left the catalog empty, and without readiness checks users would get opaque
failures.

**Independent Test**: Open the catalog with network access. Confirm that the default entry
reports ready with an identified latest run time. Disable network access and confirm that the
same entry reports "not ready: remote data unavailable" rather than failing silently.

**Acceptance Scenarios**:

1. **Given** network access, **When** the catalog is opened, **Then** the default product and
   variable are listed, preselected, and marked ready, and the latest available run
   initialization time is shown.
2. **Given** a catalog entry whose data or variable cannot be found, **When** readiness is
   checked, **Then** the entry is marked not ready with a specific reason, and it cannot be
   used to start a series request.
3. **Given** a catalog entry, **When** the user inspects it, **Then** they see the product
   name, provider attribution (Open-Meteo, CC BY 4.0, and the upstream model provider), the
   variable and unit, the time step, and the available forecast horizon.

---

### User Story 3 - Single- and multi-station exploration (Priority: P2)

A user narrows the view to one station or a chosen set of stations, compares their curves in
the chart, and changes the time window within the run's available horizon (for example, only
days 0–3).

**Why this priority**: The design calls for both single- and multi-station workflows. They
refine the P1 result rather than produce a new kind of output.

**Independent Test**: With a received 100-station result, select one station and then five
stations. Confirm that the table and chart update to exactly those stations. Change the window
to days 0–3 and confirm that only valid times in that range are shown.

**Acceptance Scenarios**:

1. **Given** a received multi-station result, **When** the user selects a single station,
   **Then** the table and chart show only that station's series.
2. **Given** a received result, **When** the user selects several stations, **Then** the chart
   shows one labelled curve per station, up to the M0 plot cap (20 series × 10,000 points).
   Anything beyond the cap is explicitly labelled as truncated or sampled.
3. **Given** a received result, **When** the user changes the relative time window, **Then**
   the result is narrowed (or re-requested) so that it covers exactly the new window, and the
   preview is bound to the new window.

---

### User Story 4 - Export with provenance (Priority: P2)

A user exports the current series to CSV or Parquet. The export includes a provenance record
covering the stations snapshot, catalog entry, run time, window, engine and extension
versions, distribution channel, data hash, and attribution. The UI marks the result "saved"
only after the export has fully committed.

**Why this priority**: Export is how results leave the app. M0 proved the atomic commit
protocol; M1 applies it to real series.

**Independent Test**: Export a received result to CSV. Confirm that the data file and its
provenance record exist, that the recorded hash matches the file, and that the UI changed from
"received" to "saved → path" only after both existed. Repeat with a forced interruption and
confirm that the export is reported as incomplete.

**Acceptance Scenarios**:

1. **Given** a received result, **When** the user exports to CSV or Parquet, **Then** the data
   file and its provenance record are written, and the result is shown as saved with the
   target path.
2. **Given** an export target that already exists, **When** the user exports, **Then** they are
   asked to confirm overwriting before anything changes.
3. **Given** an export interrupted at any commit step, **When** the app next starts, **Then**
   the interrupted export is reported as incomplete (or cleaned up, if it had committed), and
   is never shown as saved.

---

### User Story 5 - Save and reopen a project (Priority: P2)

A user saves their work as a project file that holds the station snapshot and mapping, the
catalog selection, the time window, the SQL text, and the view settings. Later they reopen it
on the same or another supported machine and continue from the same state. Re-running the
series reproduces the result for the same run.

**Why this priority**: Without persistence every session starts from scratch. Persistence
also anchors the "edited / draft staged / received / saved" state model.

**Independent Test**: Import stations, select a window, write a SQL query, and save the
project. Close the app, reopen the project, and confirm that stations, mapping, selection,
window, and SQL are restored. Edit something without saving and confirm the unsaved-changes
indicator. Kill the app and confirm that the staged draft can be recovered without overwriting
the project file.

**Acceptance Scenarios**:

1. **Given** a project with imported stations and settings, **When** the user saves and later
   reopens it, **Then** the stations (identical content hash), mapping, catalog selection,
   window, SQL text, and view settings are restored.
2. **Given** an opened project, **When** the user edits anything, **Then** the title shows
   unsaved changes until the next save.
3. **Given** unsaved edits and an app crash, **When** the app restarts, **Then** the user is
   offered the staged draft, and the project file on disk is unchanged.
4. **Given** a reopened project whose station source file has since changed on disk, **When**
   the project loads, **Then** the project still uses its own stored snapshot and tells the
   user that the original file differs.

---

### User Story 6 - SQL over stations and series (Priority: P3)

A user writes SQL that joins their station table with the forecast series, for example the
daily maximum temperature per station, or only stations above a given elevation. The result
appears in the same table and chart views.

**Why this priority**: SQL serves power users and enables ad-hoc analysis, but the core
workflow is complete without it.

**Independent Test**: With a received result, run a query joining stations and series that
aggregates to the daily maximum per station. Confirm that the result is correct for a sample
station against the raw series.

**Acceptance Scenarios**:

1. **Given** a received result, **When** the user runs a query over the `stations` and
   `station_series` tables, **Then** the query result is shown in the table view, and in the
   chart when it has a time column and numeric columns.
2. **Given** a SQL query that fails, **When** it runs, **Then** the error is shown, and the last
   successful result stays visible with a "stale" marker.
3. **Given** the SQL editor or station filter, **When** the user types Chinese text through the
   platform input method (composition, candidate selection, commit), **Then** the committed
   text appears correctly and the query runs with it.

---

### Edge Cases

- **Station CSV problems**:
  - CSV with a header but no rows → import is refused with a clear message.
  - Duplicate station identifiers → import is refused, and the duplicates are listed.
  - Latitude outside −90..90 or longitude outside −180..360 → the invalid rows are listed and
    the user can fix the file or exclude them.
  - Longitudes in 0..360 are normalized to −180..180, matching the M0 convention.
  - Non-UTF-8 encoding, a semicolon delimiter, or quoted fields → handled, or a specific
    error is raised. Malformed input is never silently misread.
- **Large station sets**: Beyond the supported size (see Assumptions), the user is warned
  before the request runs.
- **Run availability**: A run still being published is not used. The latest completely
  published run is used, whatever its cycle. A 06Z/18Z run gives 0–144 h, labelled as
  truncated. Data is never zero-filled or padded across runs.
- **Missing values**: Missing values stay null in the series. They are never replaced by 0 or
  interpolated.
- **Offline**: Network loss before or during a request → the request fails with
  "remote data unavailable", and the last successful result stays visible with a stale
  marker. The app does not crash.
- **Worker failures**: A worker crash during a request → the request fails, the worker
  restarts, no stale-generation result is shown, and the user can retry.
- **Cancellation**: A user cancel during a long request → the request ends through the M0
  cancellation levels, and the UI returns to an idle, consistent state.
- **Project portability**: A project saved with a newer project format version is refused
  with a clear message, without data loss.
- **Channel failure**: Signed-channel bootstrap failure → reported as a failure. The user may
  explicitly switch to the unsigned channel; the system never falls back silently (OQ-4).

## Requirements *(mandatory)*

### Functional Requirements

**Station import**

- **FR-001**: Users MUST be able to import a station table from a CSV file through the file
  dialog or a manual path.
- **FR-002**: The system MUST propose a column mapping for identifier, latitude, and longitude
  (plus optional name and elevation), show a preview, and require explicit user confirmation
  before creating the station set.
- **FR-003**: On confirmation, the system MUST store an immutable snapshot of the station rows
  together with a content hash, the original file path, and the confirmed mapping. Later
  changes to the source file MUST NOT change the snapshot.
- **FR-004**: The system MUST validate stations (unique identifiers, coordinate ranges,
  non-empty input) and report every invalid row before the snapshot is created.

**Catalog**

- **FR-005**: The system MUST ship a versioned product catalog that replaces the
  `omo-catalog/0` placeholder. It MUST contain at least the default entry: ECMWF IFS HRES,
  2 m air temperature, time step and horizon as published, grid description, and attribution.
- **FR-006**: The default selection on a new project MUST be ECMWF IFS HRES · 2 m temperature
  · forecast window 0–7 days of the selected run.
- **FR-007**: For each catalog entry, the system MUST check and display readiness:
  - data reachable;
  - variable present with the expected unit;
  - latest available run identified.

  Not-ready entries MUST show a specific reason and MUST NOT be usable to start a request.
- **FR-008**: The run used for a request MUST be resolved once when the request is submitted
  and recorded with the result, so that the result can be reproduced for that run.
- **FR-009**: Catalog entries MUST carry the attribution required by Open-Meteo (CC BY 4.0)
  and the upstream model provider. The attribution MUST appear in the UI and in every export
  provenance record.

**Station series**

- **FR-010**: The system MUST produce nearest-grid forecast series for every station in the
  snapshot, emitting the stable `station_series` schema (C-10) with no additional
  consumer-visible columns. The default product is the 9 km IFS HRES product (`ecmwf_ifs`, reduced
  Gaussian O1280; user decision 2026-11-23). M1 MUST therefore provide nearest-point sampling
  on this grid:
  - each station is matched to the grid point at the smallest great-circle distance;
  - ties are broken by the source point index;
  - the matched point's latitude/longitude and distance are reported.

  The M0 Plan B remains the plan for eligible regular grids. Choosing the O1280 sampling plan
  is planning work; the plan MUST be validated against an independent reference before it is
  used for the default (see SC-003).
- **FR-011**: Stations without a valid match (out of coverage, or beyond the maximum matching
  distance) MUST be reported individually with a reason, and MUST NOT block the other
  stations' results.
- **FR-012**: Valid times MUST be explicit UTC instants. Missing values MUST remain null.
  Results from different runs MUST NOT be mixed in one series.
- **FR-013**: The time window MUST be expressed relative to the selected run as a closed
  interval of lead times, default 0–168 h inclusive, adjustable within the run's available
  horizon. The default run is the **latest completely published run of any cycle** (user
  decision 2026-11-23). When that run's horizon is shorter than the requested window (06Z/18Z
  runs reach only +144 h), the result MUST cover only the available lead times and be visibly
  labelled as truncated, for example "0–144 h of requested 0–168 h". It MUST NOT be extended
  with data from another run. Requests outside the horizon MUST be clamped with the same
  visible note.

**Views**

- **FR-014**: The system MUST show series results in a paged table and a line chart, with
  station selection (single and multiple) and the M0 plot cap and its truncation labelling.
- **FR-015**: The system MUST provide a SQL view over the `stations` and `station_series`
  tables of the current project, with results shown in the same table and chart views.
- **FR-016**: Previews MUST be bound to the project revision, station snapshot hash, catalog
  version, selected run, window, query, and worker generation. When any of these change, the
  displayed preview MUST be marked out of date. Only the latest request's result may be
  displayed.
- **FR-017**: When a request fails, the last successful result MUST remain visible with a
  stale marker and the error.

**Tasks**

- **FR-018**: Series requests and exports MUST run as tasks that take an immutable snapshot of
  the project state when submitted, and show stage-based progress:
  queued → bootstrapping → planning → reading → writing → committing → saved/failed/cancelled.
- **FR-019**: Users MUST be able to cancel and retry tasks. Cancellation MUST use the M0
  three-level mechanism.
- **FR-020**: The task panel MUST show scan metrics (bytes read, duration) when the engine
  provides them.

**Export**

- **FR-021**: Users MUST be able to export the current series or SQL result to CSV or Parquet.
  The export uses the M0 atomic commit protocol: the provenance record is the single commit
  point, and existing targets require overwrite confirmation.
- **FR-022**: The export provenance record MUST include:
  - station snapshot hash;
  - catalog entry and version;
  - resolved run time and window;
  - query text, when applicable;
  - engine and extension versions;
  - distribution channel;
  - data file hash;
  - attribution.
- **FR-023**: A result MUST be shown as saved only after its export has committed and its
  hash has been verified. The startup scan MUST classify interrupted exports, as in M0.

**Project persistence**

- **FR-024**: Users MUST be able to create, save, save-as, and open a project file that
  contains:
  - the station snapshot and mapping;
  - the catalog selection;
  - the window;
  - SQL text;
  - view settings;
  - a project format version.

  Results are not stored in the project; they are reproducible from the recorded run.
- **FR-025**: The system MUST track edited/saved state per project, show unsaved changes, and
  stage drafts periodically to the app data area. Drafts MUST NEVER overwrite the project
  file.
- **FR-026**: After a crash, the system MUST offer to recover the latest staged draft.
- **FR-027**: Opening a project written in a newer, unsupported format version MUST be refused
  with a clear message and no modification of the file.

**Engine & channel (carried from M0)**

- **FR-028**: Every connection MUST use the M0 locked bootstrap. The distribution channel MUST
  follow the OQ-4 policy:
  - `community-signed` is preferred;
  - `github-release-unsigned` is used only by explicit user selection;
  - there is no silent fallback.

  The channel in use MUST be visible in the task panel and recorded in exports.
- **FR-029**: Offline or remote-unavailable conditions MUST degrade to a clear "remote data
  unavailable" failure for affected requests, while local operations (viewing the last result,
  SQL over received results, saving the project) keep working.

**Input**

- **FR-030**: Text inputs (station names in mapping and filtering, SQL editor) MUST accept
  Chinese input through the platform input method. At least one Chinese IME regression pass
  is recorded on the reference host.

### Key Entities

- **Station snapshot**: An immutable set of stations created from an imported CSV.
  - Attributes: identifier, latitude, longitude, optional name and elevation.
  - Provenance: content hash, source path, mapping, import time.
- **Catalog entry**: A queryable forecast product and variable.
  - Attributes: product identity (ECMWF IFS HRES by default), variable and unit, time step,
    horizon, grid description, attribution, readiness state, latest run time.
  - Versioned as part of the catalog.
- **Forecast run**: A specific model initialization time of a catalog product, resolved when a
  request is submitted.
- **Series request**: An immutable task snapshot.
  - Contents: station snapshot hash, catalog entry and version, resolved run, relative window,
    channel, worker generation.
- **Station series result**: Rows in the stable `station_series` schema.
  - Columns: station identifier, variable, valid time, value, grid latitude/longitude,
    distance, source object, object version.
  - Plus a per-station match report.
  - State: received or saved.
- **Project**: A user-saved document.
  - Contents: station snapshot, mapping, catalog selection, window, SQL, view settings,
    format version.
  - Tracked by revision and saved revision.
- **Export**: A committed data file plus its provenance record.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On the reference host, a user can go from a 100-station CSV to a displayed 7-day
  2 m temperature forecast for all stations, using only default settings, in under 3 minutes
  of interaction plus engine time.
- **SC-002** *(provisional target, to be confirmed in planning)*: For 100 stations × 7 days ×
  1 variable, the default series request completes on the reference host in under 2 minutes.
  Conditions: cold extension cache, the remote public bucket, and the network conditions
  recorded with the run. The median of 5 runs is reported, together with duration and bytes
  read.
- **SC-003**: For a reference station set on the default O1280 grid, 100% of matched points
  equal an independently computed nearest point, and 100% of returned values match an
  independent reference (official reader / reference oracle) for the same run, point, and
  valid times, including null positions. The station set includes stations near the poles,
  near the antimeridian, and near the boundaries between Gaussian rows.
- **SC-004**: 100% of stations outside coverage or beyond the matching distance are reported
  as "no match" with a reason. 0 are silently dropped.
- **SC-005**: Under fault injection, the system behaves correctly in 100% of trials (at least
  10 per kind), and no stale or unsaved result is ever shown as current or saved:
  - cancel during reading;
  - worker kill during reading;
  - network loss before and during a request.
- **SC-006**: Saving and reopening a project restores the station snapshot (identical content
  hash), mapping, catalog selection, window, SQL, and view settings in 100% of round-trip
  tests, on the same and on a different supported platform.
- **SC-007**: In 100% of export interruption trials at each commit step, the startup scan
  gives the correct saved/incomplete verdict.
- **SC-008**: One Chinese IME regression pass (composition, candidate selection, commit) is
  recorded on the reference host's X11 session for the SQL editor and station filter input.
- **SC-009**: Interactive acceptance runs on the reference host (`linux_arm64`). `linux_amd64`
  and `osx_arm64` pass the CI and release workflows, including a series smoke test against the
  default catalog entry where the network allows (Constitution 1.3.0).

## Assumptions

- **Users**: Single local user per installation. No accounts, sharing, or multi-user
  projects.
- **Station CSV**:
  - UTF-8 (with or without BOM), comma or semicolon delimiter, header row required.
  - Coordinates in decimal degrees, WGS 84.
- **Station set size** *(provisional)*: The supported size for M1 is up to 1,000 stations.
  Acceptance is measured at 100. Larger sets are allowed after a warning, without a
  performance guarantee.
- **Data access**: Forecast data is read remotely from Open-Meteo's public OM data bucket
  through the locked extension. Local mirrors or bulk downloads are out of scope for M1.
  Offline:
  - Reopening a project restores all inputs (stations, mapping, selection, window, SQL, views).
  - Received results stay viewable and queryable for the rest of the session.
  - Exported files remain on disk.
  - New series requests and readiness checks require network access.
- **Window anchoring**: "0–7 days" means lead times 0–168 h inclusive from the
  initialization time of the selected run, never "now + 7 days" stitched across runs.
  - A run counts as published when the provider's run metadata marks it complete and lists
    valid times through the requested lead time.
  - Delayed or partial runs are not used for the default.
  - Expected valid-time counts for the default 9 km product:
    - 113 for 0–168 h from a 00Z/12Z run (91 hourly + 18 three-hourly + 4 six-hourly);
    - 109 for the truncated 0–144 h from a 06Z/18Z run.
- **Product identifiers**: The default is the Open-Meteo `ecmwf_ifs` dataset, variable
  `temperature_2m`. Which object layout to read (per-run objects or time-series chunks) is
  decided in planning. Whatever layout is chosen, FR-012's single-run rule applies. If O1280
  nearest sampling cannot be validated during planning, planning stops and asks the user. It
  does not silently substitute another product such as `ecmwf_ifs025`.
- **Matching distance**: The default maximum station-to-point distance is the largest possible
  nearest-point distance on the product grid. For O1280 it is 7,200 m: the 7.12 km half-cell
  diagonal bound computed in research R-01, rounded up. It is recorded per request. On a
  global grid every valid coordinate matches. "No match" arises only from invalid coordinates,
  a user-tightened distance, or missing source data.
- **Catalog contents**: The catalog may contain additional products or variables. Only the
  default entry is required to be ready and measured for M1 acceptance.
- **Result storage**: Results live in temporary storage while received and are discarded on
  exit unless exported. Project files store inputs, not results.
- **Out of scope**:
  - static regional maps (M2);
  - declarative or TSX reports (L1/L2);
  - Dynamical/Icechunk sources (M3);
  - ensemble products;
  - unit conversion beyond displaying the published unit;
  - Windows, Wasm, and Intel Mac runtime acceptance;
  - any change to Duckling or duckomo.
