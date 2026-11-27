# Research: M2 Regional Tile Map & Layer Stack

**Date**: 2026-11-25 | **Feature**: `003-m2-regional-tile-map` | **Spec**: [spec.md](spec.md)

Primary empirical evidence:
- [research-nsh-geospatial.md](../../docs/m2/research-nsh-geospatial.md) (pre-spec NSH `0.5.1`/`0.7.0`, crate ecosystem, and live `data_spatial` probe)
- [PHASE0-SPIKES-spark-aarch64-20261125.md](../../docs/m2/evidence/PHASE0-SPIKES-spark-aarch64-20261125.md) (`S0-NSH-070`, `S1-M2`, and `S2-M2` isolated Phase 0 spike runs on `lizhan-spark`)

---

## R-01 Shell Pin Gate: `nsh-* =0.5.1` → `=0.7.0` (`FR-003`, `FR-004`, Constitution II)

- **Decision**: Include a gated Task Phase 1 step (`S0-NSH-070`) that upgrades `nsh-core`, `nsh-base`, `nsh-platform-linux`, and `nsh-platform-macos` from `=0.5.1` to official release **`=0.7.0`** (`default-features = false`, explicit `["editor", "highlight", "plot"]` on platform crates), updates `scripts/check-boundaries.sh` pin assertions to `"=0.7.0"`, runs an interactive Spark X11 GUI regression smoke, and proposes the matching Constitution Principle II pin amendment (`1.3.0 → 1.3.1` PATCH/MINOR) for user ratification. Mandatory M2 features use APIs available on both `=0.5.1` and `=0.7.0`, with `on_pinch` and `ImageSrc::size` wired when `=0.7.0` is active.
- **Rationale**:
  - Isolated spike `S0-NSH-070` proved **0 breaking API changes** across all 11 files in `crates/omo-app/src/ui/*.rs`, **97/97** workspace tests passing, **`check-boundaries.sh` + `--self-test` passing** (zero `nsh-js`, `quickjs`, `webkit6`, `gpui`, or `duckdb` linkage), and a **713-line `Cargo.lock` reduction** (dropping unused `gstreamer*`, `tokio-tungstenite`, `rustls`, and `jni*` transitive crates).
  - `=0.7.0` adds `ImageSrc::rgba(...).size(Size<Points>)` (HiDPI logical tile sizing), zero-copy opaque `CGImage` backing on macOS, cached macOS canvas text lines, and desktop trackpad `on_pinch` (`PinchEvent`).
- **Alternatives considered**:
  - Staying on `=0.5.1` unconditionally: viable fallback if interactive X11/macOS CI smoke exposes any subtle regression, which is why mandatory FRs/SCs avoid hard-requiring `on_pinch`.
  - Tracking `main` git commits of NSH: prohibited by Constitution Principle II.

---

## R-02 Slippy Tile Composition & Anti-Flicker Strategy in Pure-Rust NSH (`FR-001`–`FR-004`, `FR-011`)

- **Decision**: Compose the Map view in `crates/omo-app/src/ui/map_view.rs` as a clipped `.relative().size_full().overflow_hidden()` container with:
  1. **Keyed `256×256` raster tile views**: `each(visible_tile_keys, |k| *k, ...)` keyed by `(layer_id, layer_revision, z, x, y)` using `image(ImageSrc::rgba(256, 256, bytes))` positioned via `.absolute().w(px(256.0)).h(px(256.0)).translate_x(...).translate_y(...)`;
  2. **Pure-Rust 2D vector `canvas(Painter)` overlays**: stacked `.absolute().inset_0().pointer_events(PointerEvents::None)` layers for crisp coastlines/admin-0 boundaries (or worker-composited vector tiles), gridlines, crosshair, and drag/zoom feedback;
  3. **Interactive station marker & HUD overlays**: `.absolute().inset_0().pointer_events(PointerEvents::BoxNone)` for clickable/hoverable station markers and legend/attribution HUD;
  4. **Touch/mouse responder panning & cursor-anchored wheel zoom**: `.claim_touches(TouchClaim::on_start()).keep_touches(true)` + `on_responder_grant`/`move`/`release` (tracks mouse left-drag even outside widget bounds), `.on_wheel` anchored at `e.position`, `.on_mouse_move` (120 Hz coordinate/readout updates), and `.on_pinch` (on `0.7.0`).
- **Rationale**:
  - Primary-source audit of `nsh-platform-{linux,macos}/src/image.rs` found that mutating `src` in-place on a pooled `Image` slot immediately clears its texture (`None`) on L4 cache misses, causing blank-frame flashes. Keying `each(...)` nodes by `(layer_id, layer_revision, z, x, y)` keeps already-loaded tiles mounted without flicker while unmounted tiles release their `Lease` into NSH's bounded 256 MiB LRU `MemoryCache`.
  - `CanvasView` has no `putImageData` raw pixel upload, whereas `ImageSrc::rgba` uploads straight RGBA8 directly to `gdk::MemoryTexture` (Linux) and `CGImage` (macOS) with zero PNG encode/decode overhead.
- **Alternatives considered**:
  - Mutating a single viewport-sized `Image` on every pan pixel: discards cached pixels on back-and-forth panning and cannot directly host multi-layer `z/x/y` tile sources (`.pmtiles` / future external tile layers).
  - Embedding a WebView (`Leaflet` / `MapLibre GL JS`): prohibited in M0–M2 by Constitution Principles II & V (`no QuickJS/WebView` in main shell).

---

## R-03 Forecast Field Source: `data_spatial` Primary, Pre-Lock `duckomo` Tuning & `data_run` Fallback (`FR-005`–`FR-008`, `SC-001`, `SC-004`)

- **Decision**:
  1. Extend `CatalogEntry` (`omo-catalog/1`) with an optional `spatial_layout` (`kind: "data_spatial"`, `base: "s3://openmeteo/data_spatial/ecmwf_ifs/"`, `object: "{YYYY}/{MM}/{DD}/{HH}00Z/{valid_YYYY}-{valid_MM}-{valid_DD}T{valid_HH}{valid_mm}.om"`). Run resolution continues to use `entry.layout` (`data_run` `latest.json` + `meta.json`) so partial uploads are rejected identically to M1.
  2. Always pass `variable := '<var>'` explicitly to `read_om(...)` on `data_spatial` (reducing metadata bind requests from `94` to `4`).
  3. Configure `duckomo_remote_max_request = 4194304` (`4 MiB`) and `duckomo_remote_merge_gap = 65536` (`64 KiB`) inside `BootstrapConfig` / `open_bootstrapped` **before** `SET lock_configuration = true;`, and fetch regional latitude bands in parallel across a bounded reader pool (`12–16` `BootstrappedConn` readers sharing `SeriesCancel`, matching M1 Plan G's pattern).
  4. **Fallback rule**: If `data_spatial` fails with `[object_not_found]` + `(HTTP 404)` (verified deterministic `duckomo` error signature in `S1-M2`), emit a progress/status notice (`fallback_reason: "data_spatial_expired"`) and automatically fall back **once** to `entry.layout` (`data_run`) filtered to the exact same `reference_time_utc` and `valid_time_utc`, verifying `object_version` ETag stability. Any non-404 error (`RemoteUnavailable`, `Cancelled`, schema mismatch) fails immediately without fallback.
- **Rationale**:
  - `S1-M2` proved **bit-exact float32 parity** (`447/447` points, `0` bit mismatches, `max |Δ| == 0.0`) between `data_spatial` and `data_run` for `ecmwf_ifs` `temperature_2m`, while cutting regional compressed payload size by ~`270×` (`427 KB` vs `~117 MB` for `30°×40°`).
  - `S1-M2` discovered that `duckomo_remote_max_request` defaults to `64 KiB` and **cannot** be changed via `SET` after `open_bootstrapped` locks DuckDB configuration (`Cannot change configuration option ... - the configuration has been locked`). Setting `4 MiB` max request + `64 KiB` merge gap pre-lock alongside `12–16` parallel latitude-band readers comfortably meets `SC-001` (`< 15 s` median).
- **Alternatives considered**:
  - Querying remote S3 per `(z, x, y)` tile: even a `21.5 KB` `2°×3°` tile took `7.6 s` of S3 HTTP RTTs and collides with `omo-worker`'s single-active-request loop.
  - Refusing runs older than 7 days: rejected by user decision (2026-11-25) in favor of automatic slow-path `data_run` fallback with visible user notice.

---

## R-04 Seamless O1280 & Regular-Grid `EPSG:3857` Tile Rasterization (`FR-009`, `FR-010`, `SC-002`, `SC-006`)

- **Decision**:
  - Store the loaded 1-timestep forecast field in `EngineBackend` as a worker-resident `CachedField` (sparse row-segment or dense flat `Vec<f32>` up to `26.4 MB` for full global O1280, keyed by `(entry_id, reference_time_utc, valid_time_utc, extent)`), computing a single field-wide `(auto_min, auto_max)` range on load so all `(z, x, y)` tiles of a revision share an identical color normalization (`FR-010`).
  - Render `256×256` Web Mercator (`EPSG:3857`) forecast tiles on CPU in `omo-engine::map::raster` using **scanline-separable `TrapezoidBilinear` interpolation** (4 bracketing points across the north/south O1280 Gaussian rows, or standard 2D bilinear on regular lon/lat grids) with exact `u64` global pixel wrapping (`east_px = (g_x + half_world) & (world_px - 1)`) and a pre-baked `4096 × [u8; 4]` LUT from `colorgrad 0.9.0`. Missing (`NaN`) cells emit `[0, 0, 0, 0]`.
  - Write rendered tile batches to generation-scoped raw RGBA files `results/g{gen}/tile_{rev}_{layer}_{z}_{x}_{y}.rgba` (`262,144 B` each, read via `mmap`/file in `omo-app::host` and passed to `ImageSrc::rgba` in `omo-app::ui`), avoiding 16 MiB JSON frame bloat and PNG encode/decode overhead during interactive pan/zoom.
- **Rationale**:
  - Spike `S2-M2` proved **`0 / 262,144` differing pixels (`max |ΔRGBA| == 0`)** between tiled `256×256` rendering and single-pass `512×512` rendering across interior seams, the `0°` Prime Meridian, and the `±180°` antimeridian (`SC-006`), reduced off-grid RMS error by **`317×`** vs nearest-cell coloring, and rendered a **16-tile (`1024×1024`) batch in `1.11 ms` (16T) / `3.78 ms` (1T)** on Spark `aarch64` (`SC-002` target `< 100 ms`).
- **Alternatives considered**:
  - Nearest-haversine per pixel: `83×` slower (`92.4 ms` 1T) and produces blocky Voronoi facets at `z >= 6` while point inspection (`FR-012`) still reports exact nearest-grid values separately.
  - Inline base64 PNG tiles in JSON frames: wastes CPU on PNG compression + base64 encoding and risks hitting `MAX_FRAME_LEN = 16 MiB` (`crates/omo-protocol/src/frame.rs:5`).

---

## R-05 Point Readout & Future Color-to-Value Inversion Seam (`FR-012`, `FR-017`, `SC-003`)

- **Decision**:
  - Define `ValueProvenance` in `omo-protocol` as `NativeNumeric | PaletteDecoded | DisplayOnly` (with only `NativeNumeric` and `DisplayOnly` produced by M2 layers, reserving `PaletteDecoded` so external scientific raster tile layers with `radiust`-style validated palette/gray-code inversion plug in cleanly post-M2).
  - For `OmWeatherLayer` (`NativeNumeric`), cursor hover/click inspection never reads rendered tile RGB bytes:
    - To achieve **zero-IPC-latency** 60–120 Hz hover readouts while guaranteeing **100% bit-exact agreement with M1 (`SC-003`)**, `omo-engine` writes a compact `FieldReadoutIndex` artifact alongside the field load (or answers fast `map_inspect`), or `omo-app::host` queries the worker's in-memory `CachedField` via `O1280::nearest` / regular-grid nearest! Even simpler and strictly respecting `omo-app::host` boundaries: a lightweight worker `map_inspect` on an idle worker takes `< 0.2 ms` over stdio IPC when throttled to ~30–60 Hz (when no `map_load_field` is active), **and** for instant offline/zero-IPC hover even while tiles are rendering, the worker also writes a generation-scoped `field_{id}.f32bin` (`mmap`-able flat/row-band `f32` array + header) so `omo-app::host` can compute exact `(grid_lat, grid_lon, distance_m, f32_value)` in `< 5 µs` locally without touching DuckDB!
    - Let's verify boundary compliance: `omo-app::host` is UI-free Rust (only `nsh_*` and `duckdb` are forbidden in `omo-app::host`). Having `omo-engine` provide a `map_inspect` worker handler (authoritative for tests & `SC-003`) plus an optional `mmap` readout buffer gives both authoritative testability and instant hover UX!
- **Rationale**: Guarantees `max |Δ| == 0.0` against M1 `station_series` (`O1280::nearest` + exact `f32` cell value, never interpolated color), suppresses/flags stale values when the forecast layer is hidden or stale, and establishes the multi-layer readout contract for post-M2 `PaletteDecoded` layers.
- **Alternatives considered**:
  - Reading back pixel RGB from rendered `256×256` forecast tiles: lossy (quantized to 8-bit color ramp + bilinear blend across 4 cells) and violates Principle VI when exact numeric grid data is already present.

---

## R-06 Built-In Offline Coastlines & Pure-Rust Local `.pmtiles` Stack (`FR-013`–`FR-016`, `FR-018`, `SC-005`)

- **Decision**:
  - **Built-in offline basemap**: Bundle Natural Earth `1:50m` coastlines (`ne_50m_coastline`) + land admin-0 borders (`ne_50m_admin_0_boundary_lines_land`) in the compact **`OCB1` binary format** validated in `S2-M2` (`363.5 KiB` raw / `303.2 KiB` compressed under `fixtures/geodata/ne_50m_coast_borders.ocb1`, with `fixtures/PROVENANCE.md` recording Public Domain source commit & SHA-256). Render via `nsh_core::element::canvas(Painter)` in the UI view (with Cohen–Sutherland viewport segment clipping for instant crispness at any zoom) **and** via `tiny-skia` in `omo-engine` when compositing PNG exports!
  - **Local `.pmtiles` v3 archives**: Add pure-Rust crates to `omo-engine` only:
    - `pmtiles = { version = "0.28.1", default-features = false, features = ["brotli"] }` over a `memmap2::Mmap` `SyncBackend`;
    - `ruzstd = "0.9.1"` (pure-Rust Zstandard tile payload decompression, avoiding C `zstd-sys`);
    - `mvt-reader = "2.5.0"` (pre-generated `prost`, no `protoc` build dependency);
    - `tiny-skia = "0.12.0"` + `png = { version = "0.18.1", features = ["zlib-rs"] }` + `image = { version = "0.25.10", default-features = false, features = ["png", "jpeg", "webp"] }` (for decoding raster JPEG/WebP `.pmtiles` tiles to RGBA8).
  - **Deterministic MVT basemap theme**: Geometry-only polygons and polylines matched by standard MVT layer names (`water`/`ocean`, `earth`/`land`/`landcover`/`landuse`, `boundary`/`admin`, `transportation`/`roads`, `building`) with fixed dark/light cartographic colors; no external Style JSON, text label collision engine, or custom sprites in M2.
- **Rationale**:
  - `S2-M2` verified **zero C/system dependencies** (`0` `cc`/`cmake`/`zstd-sys`/`libz-sys`/`tokio`/`reqwest`), keeping cross-platform Linux x86_64/arm64 and macOS arm64/Intel CI builds completely self-contained and fast.
  - Natural Earth is Public Domain (zero license conflict with AGPL-3.0 or exported user PNGs) and `303 KiB` adds negligible binary/repo size while satisfying 100% offline map viewing (`SC-005`).
- **Alternatives considered**:
  - Enabling `pmtiles/zstd` or `pmtiles/http-async`: pulls C `zstd-sys` and `tokio`+`reqwest`, adding C toolchain surface and forbidden/unneeded HTTP client stacks.
  - Online OSM tile fetching (`tile.openstreetmap.org`): violates `FR-018`, offline requirements, and OSM's Tile Usage Policy.

---

## R-07 Worker Concurrency, Stale-Generation Reaping & Atomic Export (`FR-019`–`FR-022`, `SC-007`)

- **Decision**:
  1. **Serialized host map queue**: Because `omo-worker` enforces a single active request (`crates/omo-worker/src/server.rs:417-424`), `omo-app::host::map_host` maintains a dedicated `MapRequestScheduler` (separate from SQL's `PreviewQueue` so map and SQL previews never overwrite each other's `doc_id` slot!). It serializes `map_load_field` → `map_render_tiles` (batched visible `(z, x, y)` list per request!) → `map_inspect` / `export`, coalescing rapid pan/zoom events and issuing soft `interrupt` (`SeriesCancel`) when a newer field revision supersedes an in-flight `map_load_field`.
  2. **Supervisor tracking & stale-generation directory reaping**:
     - Add `"map_load_field"`, `"map_render_tiles"`, `"map_inspect"`, and `"map_layer_probe"` to `is_tracked_request` in `crates/omo-app/src/host/supervisor.rs:607`.
     - Extend `remove_result_if_owned` (`supervisor.rs:118-132`) to allow `.rgba`, `.png`, and `.f32bin` extensions alongside `.arrow` and `.parquet`, delete tile file arrays on `GenerationDisposition::Stale`, **and** recursively reap `result_root/g{old_gen}/` on worker crash/restart and app shutdown so zero stale-generation tile files leak (`SC-007`).
  3. **Atomic composite PNG export & `omo-export-sidecar/3`**:
     - Reuse the existing `"export"` worker request kind with `format: "png"` and a `map_export` snapshot payload (`viewport`, `valid_time_utc`, `color_scale`, `layers`) so `runtime.rs` `active_exports`, `recent-export-dirs.json`, `"export_committed"`, and `FailKind::Export` work unchanged.
     - Introduce `SIDECAR_SCHEMA_V3 = "omo-export-sidecar/3"` in `omo-protocol::export` (`is_supported_sidecar_schema` accepts `/1`, `/2`, and `/3`), keeping top-level `"schema"` and `"/data/sha256"` (`{"file": "<name>.png", "sha256": "<hex>", "format": "png", "width": W, "height": H}`) so `export_scan::is_saved` and `scan_export_dir` (`crates/omo-app/src/host/export_scan.rs:25-118`) require zero structural changes.
     - Extend `omo-harness export-kill --source map` (`crates/omo-app/src/bin/omo-harness.rs`) for `SC-007`.
- **Rationale**: Reuses every battle-tested M0/M1 supervision and atomic-export invariant instead of inventing a parallel export or crash-recovery state machine.
- **Alternatives considered**:
  - Creating a separate `"export_map"` request kind or directory-mode `.omo/` export: duplicates `runtime.rs` / `server.rs` / `export_scan.rs` export tracking when a single composite PNG + `.png.omo.json` sidecar already satisfies `FR-021`–`FR-022`.

---

## R-08 Backward-Compatible `.omoproj` Persistence & Deterministic Hashing (`FR-023`, `SC-008`)

- **Decision**:
  - Keep `PROJECT_SCHEMA_V1 = "omo-project/1"` (`*.omoproj`) and add `#[serde(default)] pub map: MapViewState` on `ProjectDoc` (`crates/omo-project/src/doc.rs`).
  - Also peek top-level `"schema"` major version *before* full `ProjectDoc` struct deserialization in `open_from_path` so future `omo-project/2` files with altered field types always return `ProjectFileError::UnsupportedSchema` (`FR-027` hardening).
  - Normalize `-0.0` → `0.0` on all `f64` fields in `MapViewState` (`center_lat`, `center_lon`, `color_min`, `color_max`, `opacity`) and use `Vec<MapLayerConfig>` (never `HashMap`) so `serde_json::to_vec_pretty` (`float_roundtrip`) produces byte-identical `.omoproj` files across Linux and macOS (`SC-008`).
  - Wire map signals into `ProjectController::{fingerprint, snapshot_doc, apply_doc}` (`crates/omo-app/src/ui/project_ctl.rs`) and `PreviewKey` (`crates/omo-project/src/preview_key.rs` / `MapPreviewKey`) so unsaved edits (`●`), 2 s / 30 s auto-draft staging, and crash recovery cover all map settings.
- **Rationale**: Existing M1 `.omoproj` files (`fixtures/projects/m1-roundtrip.omoproj`) open with zero migration friction via `#[serde(default)]`, while M2 map state round-trips deterministically.
- **Alternatives considered**:
  - Bumping `.omoproj` to `omo-project/2`: would break opening M2-saved projects in M1 readers unnecessarily when all M2 additions are strictly optional/additive.
