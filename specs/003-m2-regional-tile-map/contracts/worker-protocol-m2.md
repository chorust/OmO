# Contract: Worker Protocol Extensions (M2 Map & Layer Operations)

Extends [M0 worker protocol](../../001-m0-skeleton-engine/contracts/worker-protocol.md) and [M1 worker protocol](../../002-m1-station-time-series/contracts/worker-protocol-m1.md).
`PROTOCOL_VERSION` stays `1`, framing remains `u32` little-endian length-prefixed UTF-8 JSON (`MAX_FRAME_LEN = 16 MiB`), and the single-active-request rule (`crates/omo-worker/src/server.rs:417-424`) stays in force.

## Concurrency & Host Serialization Rule

Because `omo-worker` rejects concurrent requests with `FailKind::Query("busy: worker already has an active request")`:
- `omo-app::host::map_host` MUST serialize worker map calls through a dedicated `MapRequestScheduler` (independent of SQL's `PreviewQueue`):
  `map_load_field` → `map_render_tiles` (batched visible `(z, x, y)` list) → `map_inspect` / `export`.
- Rapid pan/zoom events coalesce pending `map_render_tiles` batches to the latest visible viewport set; superseding `map_load_field` revisions issue soft `interrupt` first.
- All new request kinds below MUST be registered in `SupervisorCore::is_tracked_request` (`crates/omo-app/src/host/supervisor.rs:607`) so worker crashes emit synthetic `WorkerCrashed` failures and increment generation.

## New Request Kinds (App → Worker)

| `kind` | `body` | Response event `kind` | `body` |
|---|---|---|---|
| `map_load_field` | `{ field_id, catalog_entry, catalog_version, run: "latest_complete" \| "<rfc3339>", valid_time_utc?: "<rfc3339>", bbox: { lat_min, lat_max, lon_spans: [[lon_min, lon_max], ...] }, force_data_run?: bool }` | `result` | `{ request_id, field_id, run: ForecastRunSummary, valid_time_utc, source_layout: "data_spatial" \| "data_run", fallback_notice: Option<String>, source_object, object_version, stats: { valid_count, min, max, unit }, coverage_bbox, scan_metrics, readout_path?: String }` |
| `map_render_tiles` | `{ result_id, field_id?: String, revision: u64, color_scale: { palette, min: f64, max: f64 }, layers: Vec<RenderLayerRequest>, tiles: Vec<{ z: u8, x: u32, y: u32 }> }` | `result` | `{ request_id, result_id, revision, tiles: Vec<{ layer_id, z, x, y, width: 256, height: 256, rgba_path, empty: bool }>, layer_errors: Vec<{ layer_id, message }> }` |
| `map_inspect` | `{ field_id, latitude: f64, longitude: f64, max_distance_m: u32 }` | `result` | `{ request_id, latitude, longitude, sample: ForecastSampleResult }` |
| `map_layer_probe` | `{ layer_id, kind: "local_pmtiles", path: String }` | `result` | `{ request_id, layer_id, info: PmtilesLayerInfo }` |
| `export` *(extended)* | `{ target, format: "png", overwrite_confirmed, map_export: { revision, field_id?, viewport: { bbox, center_lat, center_lon, zoom, width, height }, stations_ref?, stations_sha256?, station_count, selected_station_ids, catalog_entry, catalog_version, valid_time_utc, color_scale, layers, attribution } }` | `export_committed` | `{ request_id, target, sidecar_path, sha256, bytes }` |

## `map_load_field` Semantics & `data_spatial → data_run` Fallback

1. Emit `progress{stage:"planning"}`: resolve `run` (if `"latest_complete"`) via `entry.layout` (`data_run` `meta.json`), verify `valid_time_utc` belongs to `run.valid_times` (defaulting to `run.valid_times[0]` when omitted; failing with `ValidTimeOutOfRange` if outside the run's published steps).
2. Emit `progress{stage:"reading"}`:
   - Unless `force_data_run == true` or `entry.spatial_layout` is `None`, build `spatial_url` from `entry.spatial_layout` and fetch the requested `bbox` across parallel latitude-band `BootstrappedConn` readers (capped at `≤ 16` readers) with `variable := '<var>'` and pre-lock `duckomo_remote_max_request = 4194304` / `duckomo_remote_merge_gap = 65536`. When stations span more than a `60° × 80°` synoptic window (such as the near-global `m1-100.csv` fixture), the host clips the initial `bbox` request to the active visible viewport (+ 1-tile gutter) or loads coarse/viewport bands first so wide-area datasets never stall on an unrequested 5.3M-point scan.
   - **Exact fallback trigger**: If `data_spatial` fails with `[object_not_found]` and `(HTTP 404)`, emit `progress{stage:"reading", detail:"data_spatial expired; falling back to data_run archive"}` and execute **at most one** fallback read against `entry.layout` (`data_run`) for the exact same `(reference_time_utc, valid_time_utc, variable)`, setting `source_layout = "data_run"` and `fallback_notice = Some(...)`.
   - Verify `object_version` (ETag) consistency across all parallel band readers (`FailKind::SourceChanged` on mid-read ETag mutation).
3. Emit `progress{stage:"writing"}`: store `CachedField` in `EngineBackend` (bounded LRU of 2 fields), optionally write `results/g{gen}/field_{field_id}.f32bin` for zero-IPC host hover readouts, and emit `result`.

## Stale-Generation Artifact Reaping (`supervisor.rs`)

1. All worker-written tile and field files MUST reside under `<result_root>/g<worker_generation>/` with extensions `.rgba`, `.png`, or `.f32bin` (alongside M0/M1 `.arrow` and `.parquet`).
2. `SupervisorCore::remove_result_if_owned` MUST allow `arrow | parquet | rgba | png | f32bin` inside canonical `result_root`.
3. On `GenerationDisposition::Stale`, `SupervisorCore` deletes any `tiles[].rgba_path` and `readout_path` referenced in the discarded message.
4. On worker crash/restart (`generation` rollover `g -> g+1`) and clean shutdown, `SupervisorCore` recursively removes `<result_root>/g<old_gen>/` after verifying canonical prefix containment (`SC-007`).

## Additive `FailKind` Variants

| `FailKind` | Meaning |
|---|---|
| `ValidTimeUnavailable(String)` | Requested `valid_time_utc` is outside the run's `valid_times`, or both `data_spatial` and `data_run` returned 404 |
| `InvalidLayer(String)` | Direct `map_layer_probe` failed due to missing, unreadable, corrupt, or non-`EPSG:3857` `.pmtiles` archive |

*(Note: During `map_render_tiles`, an errored `.pmtiles` layer reports its diagnostic in `layer_errors` and renders transparent for that layer without failing healthy layers, per `FR-016`.)*
