# Data Model: M0 Skeleton & Usable Engine

The types below are **contracts** for the four UI-free core crates. Field names are normative.
Rust spelling is indicative.

## omo-engine

### BootstrapConfig
| Field | Type | Rule |
|---|---|---|
| `data_dir` | abs path | must exist and be writable |
| `ext_cache_dir` | abs path | becomes DuckDB `extension_directory` |
| `duckomo_artifact` | `ArtifactLockEntry` | selected by (channel, platform) from the lock |
| `platform` | enum `linux_amd64 \| linux_arm64 \| osx_arm64` | must equal `PRAGMA platform` |
| `threads`, `memory_limit` | optional | passed to `duckdb::Config` at open |
| `httpfs_timeout` | Duration, default 15 s | |

No field is ever derived from another app's data directory (FR-006).

### ArtifactLockEntry (`artifacts/duckomo.lock.json`, see `contracts/artifact-lock.md`)
`channel` (`community-signed | github-release-unsigned`), `extension` = `duckomo`,
`version` (`0.4.0`), `duckdb_version` (`v2.0.0`), `platform`, `sha256`, `size`,
`source {url, pr?, ref?, release_tag?, downloaded_at}`, `golden {fixture, query_id, result_sha256}`.

### BootstrappedConn
`conn` (private), `caps {remote: bool, spatial: bool}`, `probe_report: ProbeReport`,
`generation: u64`. It can be constructed **only** by `open_bootstrapped`. All OM-query APIs
take `&BootstrappedConn` (FR-009).

### ProbeReport / BootstrapReport
`duckdb_version`, `platform`, `duckomo_version`, `channel`, `functions_present: [..]`,
`httpfs: Loaded | Installed | Unavailable(reason)`, `spatial: NotRequested | Loaded | Failed(reason)`,
`golden: {expected, actual, ok}`, `timings_ms {open, httpfs, duckomo, lock, probes, golden}`,
`r4: {unsigned_close: Ok|Err|NotApplicable, spatial_after_lock: Ok|Err|NotRun}`.

### BootstrapError
`ArtifactMismatch{field}`, `Duckomo(reason)`, `ProbeFailed{probe, expected, actual}`,
`GoldenMismatch`, `ConfigLock(reason)`. `CapabilityUnavailable(remote|spatial)` is a separate
task-level error.

### StationSeriesRow (stable schema, C-10)
| Column | Type |
|---|---|
| `station_id` | VARCHAR |
| `variable` | VARCHAR |
| `valid_time` | TIMESTAMPTZ (UTC) |
| `value` | FLOAT / DOUBLE (missing → NULL) |
| `grid_lat`, `grid_lon` | DOUBLE (lon normalised to [-180, 180)) |
| `distance_m` | DOUBLE |
| `source_object` | VARCHAR |
| `object_version` | VARCHAR |
Uniqueness: (`station_id`, `variable`, `valid_time`) after chunk de-duplication.

### SeriesPlan
`PlanA {grid_index per station}` | `PlanB {chunk objects, buffer_m}`. The spike report covers
the semantic checks (lon/0-360, CRS, distance algorithm, missing values, chunk overlap/dedup,
UTC), plus `bytes_read`, `duration_ms`, and `matches_reference`.

## omo-protocol

### Envelope
`{ v: 1, worker_generation: u64, job_id: u64, request_id: u64, kind, body }`. See
`contracts/worker-protocol.md`.

### ResultRef
`{ id, generation, path, rows: u64, schema_hash: sha256, truncated: bool, limits {rows, bytes} }`.
Its lifetime is the session temp dir. It is deleted when its generation is stale or when it is
discarded on exit.

### JobState
`Queued → Bootstrapping → Planning → Reading → Writing → Committing → Saved`, plus
`Failed(kind)` and `Cancelled`.
`FailKind = WorkerCrashed | Bootstrap(..) | Capability(..) | Query(..) | Cancelled(level) | Export(..) | Timeout`.
Retry is allowed from `Failed`.

### CancelLevel
`Interrupt` → `Drain(≤10 s)` → `Kill` (restart + re-bootstrap). The level that ended the
task is recorded.

## omo-worker / supervisor (omo-app::host)

### WorkerHandle
`generation`, `kind: Interactive | Job`, `pid`, `last_heartbeat`, `state: Starting |
Bootstrapping | Ready | Busy | Draining | Dead`. Heartbeats arrive every 1 s. 5 s of silence
counts as hung, which triggers Kill (initial values, tuned in M0).

## omo-project (M0 subset)

### ProjectDoc (minimal)
`doc_id`, `revision: u64`, `saved_revision: Option<u64>`, `stations_source?
{orig_path, snapshot_path, sha256, mapping}`, `sql_tabs: [text]`. Derived:
`dirty = saved_revision != Some(revision)`. M0 needs only revision/dirty semantics for
PreviewKey. Full persistence is M1.

### PreviewKey
`{doc_id, revision, view_id, query_hash, stations_hash, catalog_version, worker_generation}`.

### Completion states
`edited`, `draft_staged`, `received`, `saved`, which are distinct and never conflated (M0
exercises `received` vs `saved` via export).

## Export

### ExportBundle / Sidecar (`<name>.omo.json` beside data; see `contracts/export-protocol.md`)
`schema: "omo-export-sidecar/1"`, `job {job_id, revision, sql, stations_hash, catalog_version}`,
`bootstrap_report`, `duckomo {version, channel, sha256}`, `duckdb_version`,
`data {file, sha256, rows}`, `attribution: "Open-Meteo, CC BY 4.0"`, `created_at`.

## Evidence

### AcceptanceRecord (`docs/m0/evidence/*.md` + `docs/m0/acceptance.md` table)
`item` (N1..N9 | EXIT-1..EXIT-6 | SPIKE-1..5), `platform`, `session` (`wayland|x11|n/a`),
`result: pass | fail | not-run(reason)`, `measurements {..}`, `command`, `versions`,
`date`, `evidence_path`.

### ProvenanceRecord (`vendor-provenance/DUCKLING.md`)
`duckling_commit`, `mapping [{from, to, trimmed}]`, `cherry_picks [{commit, files, date}]`.
Its content depends on the FR-005 decision.
