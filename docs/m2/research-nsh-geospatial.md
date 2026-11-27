# M2 Pre-Spec Research: NSH Geospatial, Raster & Tile-Map Support

- **Date**: 2026-11-25 UTC
- **Scope**: Primary-source audit of locally pinned `nsh-* =0.5.1`, upstream `longbridge/nsh` releases (`0.6.0`–`0.7.0`), pure-Rust tile/raster crates, and `OmO` × `duckomo` data/protocol constraints for `003 — M2 Static Regional Map` vs. interactive tile-based maps.
- **Method**: Three parallel read-only subagent lanes (`nsh-local-source`, `nsh-upstream-ecosystem`, `omo-duckomo-fit`) verified against Cargo registry sources, upstream repositories/APIs, and `OmO` source files.

---

## 1. Executive Summary

1. **NSH (`0.5.1` pure Rust, `default-features = false`, no QuickJS/WebView) is far more capable than `design.md` §5.1 conservatively assumed**:
   - It has **no built-in `Map` widget** (neither in `0.5.1` nor in upstream `0.7.0`), **but** it already exposes all four building blocks needed for either an interactive slippy tile map or an interactive raster+vector viewport in pure Rust:
     1. **Zero-copy in-memory RGBA textures**: `ImageSrc::rgba(width, height, pixels)` (`nsh-core-0.5.1/src/element/asset.rs:161-168`) backed by a 256 MiB process-wide LRU `MemoryCache` with active-view `Lease` pins (`nsh-core-0.5.1/src/image/memory.rs:30-140, 450-560`).
     2. **Pure-Rust 2D vector `canvas(painter)` (`Painter`)**: Cairo (`nsh-platform-linux-0.5.1/src/canvas.rs`) and CoreGraphics (`nsh-platform-macos-0.5.1/src/canvas.rs`) vector paths, text, clipping, partial-damage caching (`DamageScan`), and synchronous `path_contains` hit-testing (`nsh-core-0.5.1/src/element/host.rs:185-760, 1680-2465`).
     3. **Clipped absolute/transform composition**: `.relative().overflow_hidden()` viewport + keyed `.absolute().w(px(256.0)).h(px(256.0)).translate_x(...).translate_y(...)` tile nodes + `z_index` vector/marker/HUD layers (`PointerEvents::None` / `PointerEvents::BoxNone`, `nsh-core-0.5.1/src/element/style.rs`).
     4. **Full pan/zoom/hover events**: `.claim_touches(TouchClaim::on_start()).keep_touches(true)` + `on_responder_move` (captures mouse left-drag and touch even outside widget bounds, `nsh-core-0.5.1/src/element/responder.rs:1-285`), `.on_wheel` with element-local cursor `e.position` (`nsh-core-0.5.1/src/ops.rs:560-625`), `.on_mouse_move` (120 Hz), and `.on_layout` bounds reporting.
2. **The real architectural constraint for a tile map in `OmO` is the weather grid I/O & worker protocol, not NSH**:
   - Calling remote S3 via `duckomo` per `(z, x, y)` tile would be prohibitively slow on `ecmwf_ifs` O1280 (`577.7 MB` `data_run` object, `[1, 7, 145]` chunks) and would collide with `omo-worker`'s single-active-request loop (`crates/omo-worker/src/server.rs:417-424`).
   - **Key principle**: Decouple **1-timestep Field Fetch & Worker Cache** (`omo-worker` fetches the scalar field once per `(product, run, valid_time, variable)` and caches ~`2–26.4 MB` in memory) from **Local Tile / Viewport Rasterization** (`omo-worker` renders `256×256` `(z, x, y)` tiles in batches or renders the current viewport in ~`5–25 ms` from memory without touching the network).
3. **Basemap & Attribution**:
   - Online XYZ raster basemaps (e.g., `tile.openstreetmap.org`) conflict with `omo-app`'s no-network rule (`scripts/check-boundaries.sh`), offline operation, and OSM's Tile Usage Policy.
   - Bundled **Natural Earth** vector coastlines/borders (`1:50m` / `1:10m`, **Public Domain**) or an offline local **PMTiles** archive (`pmtiles` crate) enables crisp multi-zoom geographic context 100% offline with zero third-party tile scraping.

---

## 2. Primary-Source Findings: Pinned `nsh-* 0.5.1` (Local Cargo Cache)

Crate root audited: `/home/blizhan/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`.

### 2.1 Image, Pixel Buffers, Caching & Flicker Rules
- **Pure-Rust `ImageSrc` surface** (`nsh-core-0.5.1/src/element/asset.rs:83-189`, `builder.rs:2321-2474`):
  - Accepts `file://` / absolute paths (`Location::File`), `asset:/` URIs, `http(s)://` URLs, `data:image/...;base64,...` URIs, `ImageSrc::svg_markup(&str)`, and **raw straight (unpremultiplied) 8-bit RGBA buffers** via `ImageSrc::rgba(width: u32, height: u32, pixels: impl AsRef<[u8]> + Send + Sync + 'static)`.
  - Both Linux (`nsh-platform-linux-0.5.1/src/image.rs:378-381`, `gdk::MemoryTexture`) and macOS (`nsh-platform-macos-0.5.1/src/image.rs:771-789`, `CGImage`) upload `ImageSrc::rgba` pixels without PNG encode/decode.
  - **Hard limitation**: `ImageSrc` fields are `pub(crate)`; there is no `ImageSrc::from_png_bytes(Vec<u8>)` constructor. Worker-produced tiles should either be passed via `mmap`/file paths or raw RGBA bytes (`ImageSrc::rgba`).
- **LRU Texture Cache & Anti-Flicker Rule**:
  - Decoded textures live in a process-wide `MemoryCache` capped at `1/16` system RAM up to **256 MiB** (`nsh-core-0.5.1/src/image/memory.rs:30-140`), with active mounted views pinned by `Lease` and unmounted tiles evicted LRU (with `libc::malloc_trim(0)` on Linux GNU after 64 MiB evicted).
  - **Critical flicker gotcha**: Mutating `src` in-place on an existing `Image` element immediately clears its displayed texture (`None`) when the new key misses L4 cache (`nsh-platform-linux-0.5.1/src/image.rs:843-858`, `nsh-platform-macos-0.5.1/src/image.rs:1115-1122`).
  - **Mitigation**: For tile grids, render tiles with `each(visible_tile_keys, |k| *k, ...)` **keyed by `(revision, z, x, y)`** (so existing tiles stay mounted and never blank while newly exposed tiles load); for single-viewport frames, double-buffer two stacked `Image` layers or swap only after `on_load` fires.

### 2.2 Pure-Rust 2D `canvas(painter)` & Overlays
- **`canvas(painter)` (`nsh-core-0.5.1/src/element/host.rs:185-760, 1680-2465`)**:
  - Records vector `Drawing` ops (`move_to`, `line_to`, `bezier_curve_to`, `arc`, `rect`, `fill_svg`/`stroke_svg`, dashes, linear/radial/conic gradients, `fill_text`, clipping, `is_point_in_path` / `path_contains`) and replays on Cairo (`ImageSurface` → `gdk::Texture`) on Linux and CoreGraphics (`CGContext` / `CGLayer`) on macOS.
  - Supports `DamageScan` static-prefix caching (`nsh-core-0.5.1/src/canvas/damage.rs`).
  - **Limitation**: `CanvasView` has no `putImageData` raw pixel blit (`draw_image` only takes URI strings) and `ShaderView` rejects custom `@binding` textures. Thus **raster weather pixels belong in `Image` (`ImageSrc::rgba` or PNG path), while vector coastlines, gridlines, crosshairs, station rings, and selection boxes belong in a sibling `canvas(painter)` layered above it**.
- **`nsh_base::plot` internals** (`nsh-base-0.5.1/src/plot/mod.rs:1290-1765`):
  - `PlotContext` and `AxisMap` are module-private (geospatial layers cannot plug into `Plot` as custom series), but `plot/mod.rs` proves the exact pure-Rust pattern: `view().relative().overflow_hidden().on_layout(...)` + `canvas(Painter)` + `.absolute()` marker/HUD children + `Interactive` mouse/responder handlers.

### 2.3 Pointer, Pan & Wheel Gesture Surface
- **Panning (`nsh-core-0.5.1/src/element/responder.rs:1-285`)**:
  - `on_mouse_move` stops firing if the cursor leaves widget bounds during a fast drag.
  - `.claim_touches(TouchClaim::on_start()).keep_touches(true)` + `.on_responder_grant` / `.on_responder_move` / `.on_responder_release` captures both mouse left-button drags and touch drags continuously outside element bounds, delivering `gesture.delta` and velocity per frame.
- **Zooming & Hover (`nsh-core-0.5.1/src/ops.rs:500-630`, `builder.rs:1390-1980`)**:
  - `.on_wheel(|e: &WheelEvent| ...)` provides `WheelDelta::Lines` / `Points` plus `e.position: Point<Points>` (element-local cursor coordinates for zoom-around-cursor).
  - `.on_mouse_move(|e: &MouseEvent| ...)` provides 120 Hz cursor `e.position` for real-time `(lat, lon)` + cached grid-value readout without round-tripping to DuckDB.

---

## 3. Upstream NSH (`0.5.1` → `0.7.0`) & Pure-Rust Geospatial Ecosystem

### 3.1 Upstream `longbridge/nsh` Releases
Verified via crates.io API and `github.com/longbridge/nsh` releases:
- Versions after `0.5.1` (`2026-11-14`): `0.6.0` (`2026-11-18`), `0.6.1` (`2026-11-20`), and **`0.7.0` (`2026-11-25`)** — all `Apache-2.0`, Edition `2024`, MSRV `1.97`.
- Notable geospatial-relevant additions in `0.6.0`–`0.7.0` (if/when `OmO` evaluates a Constitution Principle II version bump):
  - `ImageSrc::size(Size<Points>)` (`0.7.0`) for explicit 2× HiDPI logical sizing of `ImageSrc::rgba` buffers, plus zero-copy opaque sRGB `CGImage` wrapping on macOS.
  - Desktop touchpad `PinchEvent` (`.on_pinch`, `0.6.0+`) and smoother desktop wheel handling.
  - Cached macOS canvas text lines (`#3133`) and font ascent/descent metrics (`#2619`) in `0.7.0`.
  - Still **no built-in `Map` widget** (`nsh-extra 0.7.0` only adds geolocation, camera, photos, push, speech).
- **Important**: `nsh-* =0.5.1` already suffices to build either static raster, interactive cached-viewport, or slippy tile maps; upgrading NSH is optional, not a blocker.

### 3.2 Pure-Rust Geospatial & Raster Crate Evaluation

| Category | Crate | Version & License | Verdict for `OmO` (`omo-engine` / `omo-worker`) |
|---|---|---|---|
| **Existing Map Engines** | `walkers` `0.66.0` | `MIT` | **Skip**: hard-coupled to `egui 0.37` + `reqwest` + `tokio`. |
| | `galileo` `0.2.1` | `MIT OR Apache-2.0` | **Skip**: rendering requires `wgpu`; heavy ~30-crate core graph. |
| | `maplibre` / `mln-rs` `0.0.15` | `MIT/Apache-2.0` / `BSD-2-Clause` | **Skip**: coupled to `wgpu`/Metal/GL; `mln-rs` is experimental (`0.0.15`, MSRV `1.98`). |
| **Tile Pyramid Math** | Internal (~100 LOC) or `utiles-core` `0.9.0` / `tile-grid` `0.6.2` | `MIT OR Apache-2.0` / `MIT` | **Recommended**: Web Mercator (`EPSG:3857`) & Equirectangular (`EPSG:4326`) `z/x/y ↔ (lat, lon)` math is ~100 LOC of pure `f64` (zero deps), or `tile-grid` (`projtransform` off) if OGC TMS structs are desired. (**Avoid** `slippy-map-tiles`: `GPL-3.0` & unmaintained). |
| **CPU 2D Raster / Vector** | `tiny-skia` `0.12.0` | `BSD-3-Clause` | **Recommended**: minimal pure-Rust SIMD 2D painter (already in `Cargo.lock` as `0.11.4` via NSH `resvg`). Note: `Pixmap` is premultiplied RGBA8 — keep opaque (`alpha = 255`) or unpremultiply before `ImageSrc::rgba`. |
| **PNG Codec / Buffers** | `image` `0.25.10` or `png` `0.17`/`0.18` | `MIT OR Apache-2.0` | **Recommended**: `png` is already in `Cargo.lock`; encodes export PNGs and straight RGBA8 buffers cleanly. |
| **Colormaps** | `colorgrad` `0.9.0` (or internal LUT) | `MIT OR Apache-2.0` | **Recommended**: pre-bake a 256-entry `[u8; 4]` LUT (`grad.colors(256)`) for single-nanosecond per-pixel scalar coloring (Viridis, Turbo, RdBu, etc.). |
| **Vector Basemap & Optional Archive** | `geo` `0.33.1` + Natural Earth (`1:50m`) or `pmtiles` `0.28.1` + `mvt-reader` `2.5.0` | `MIT OR Apache-2.0` + Public Domain | **Recommended**: ship compact simplified Natural Earth coastlines/admin-0 borders (Public Domain, offline, zero license burden), or optional local `.pmtiles` vector basemap. |

---

## 4. `OmO` × `duckomo` Data Path & Architecture Constraints

### 4.1 O1280 (`ecmwf_ifs` 9 km) vs Regular Grids for Regional / Tile Reads
- **Regular spherical grids**: `read_om` supports 2D dimension slicing, and `read_om_regions` works after `BootstrappedConn::ensure_spatial()` (`crates/omo-engine/src/conn.rs:76-97`, `docs/m0/decisions/r4.md`).
- **O1280 reduced Gaussian (`crates/omo-engine/src/grid/reduced_gaussian.rs`)**:
  - `data_run/ecmwf_ifs` stores a **1D `point` axis** of `6,599,680` points (`2,560` latitude rows) with chunk shape **`[1, 7, 145]`** across a **577.7 MB** `.om` file (`specs/002-m1-station-time-series/research.md:122-127`).
  - `read_om_regions(method := 'nearest')` is rejected on O1280, and `method := 'points'` degraded to a >180 s full scan in M1 S1 (`docs/m1/evidence/S1-spark-aarch64-20261123.md:16`).
  - In 1D `flat_index` space, `lat BETWEEN lat_min AND lat_max` is one contiguous range (`325,748` points / ~`28.5 MB` for a `10°×10°` mid-latitude band; `1,344,196` points / ~`117.7 MB` for a `30°×40°` East Asia band because every `data_run` chunk carries all 145 time steps), while `lon` within each row is `142–427` disjoint short segments.
  - Meanwhile, Open-Meteo's `data_spatial/ecmwf_ifs` layout (`s3://openmeteo/data_spatial/ecmwf_ifs/{YYYY}/{MM}/{DD}/{HH}00Z/{YYYY}-{MM}-{DD}T{HH}00.om`, ~128 MB hierarchical `.om` bundling 42 variables for a single valid time, retained ~7 days) avoids the 145× time-step read amplification!
  - **Live Spark probe on `data_spatial/ecmwf_ifs/2026/11/25/0000Z/2026-11-25T0100.om` (2026-11-25)**:
    - Passing `variable := 'temperature_2m'` is mandatory: omitting it binds all 42 child variables (`94` S3 HTTP requests, `~33 s` overhead), whereas `read_om(url, variable := 'temperature_2m', domain := 'ecmwf_ifs')` cuts `bind_requests` to **`4`**!
    - Compressed `/temperature_2m` payload sizes in `data_spatial`:
      - `2°×3°` tile (`669` pts): **`21,967 B` (`21.5 KB`)**, `28` data requests (`7.6 s` single-connection due to S3 RTT);
      - `30°×40°` East Asia bbox (`148,957` pts): **`432,527 B` (`422 KB`)**, `464` per-row range requests (`34.7 s` single-connection; parallelizable across latitude sub-bands just like Plan G's 20 reader connections to ~`2–3 s`!);
      - Full global O1280 (`6,599,680` pts): **`4,692,652 B` (`4.48 MB`)** total (`17,723` chunks).
    - Because even a `21.5 KB` tile takes `~7.6 s` of S3 HTTP RTTs if queried cold from S3, **both Option 2 and Option 3 should prefetch/cache the `data_spatial` field in `omo-worker` memory (parallelized across latitude bands) and cut viewport/tiles locally in milliseconds**, falling back to `data_run` only when viewing archived runs older than `data_spatial`'s ~7-day retention window.

### 4.2 Worker Protocol & State Boundaries (`Constitution I, II, IV`)
- `omo-worker` processes **one active request at a time** (`crates/omo-worker/src/server.rs:417-424`), and `omo-app/src/host/supervisor.rs:79-128` reaps generation-scoped files under `results/g{generation}/` (must extend whitelist from `.arrow`/`.parquet` to include `.png`/`.rgba` map artifacts).
- **Why per-tile S3 requests fail, and how Tile Mode works cleanly**:
  - If UI mounted 20 tiles and sent 20 separate IPC requests that each hit S3, 19 would get `busy` or queue serially for minutes.
  - Instead, **if Tile Mode (`z/x/y`) is chosen**:
    1. Request 1 (`map_load_field`): Worker fetches and caches the 1-timestep `CachedField` (`f32` array, `≤ 26.4 MB` even globally!) in `EngineBackend` RAM, tied to `PreviewKey` + `object_version_token`.
    2. Request 2 (`map_render_tiles { revision, z, tiles: [(x,y), ...] }`): Single batched IPC request! Worker renders all requested `256×256` tiles in parallel on CPU (`rayon`/threads) from the in-memory `CachedField` in **~5–15 ms total**, writes `results/g{gen}/tile_{rev}_{z}_{x}_{y}.rgba` (or `.png`), and returns paths/metadata in one response.
    3. Export (`export_map`): Worker renders the current viewport `(bbox, width, height)` + Natural Earth coastlines + station markers + colorbar + CC BY 4.0 / ECMWF attribution into **one composite PNG** and commits it atomically with `<target>.png.omo.json` via existing `commit_staged` (`crates/omo-engine/src/export.rs:55`).

---

## 5. Comparison of Three Candidate M2 Architectures

| Dimension | Option 1: Static Composite PNG (`design.md` §7.5 baseline) | Option 2: Cached-Field Interactive Viewport (`Image` + `Canvas` overlay) | Option 3: Local Worker Slippy Tile Pyramid (`z/x/y` `256×256` tiles + `Canvas` overlay) |
|---|---|---|---|
| **User Experience** | Fixed bbox around stations; switch time/variable/margin triggers new PNG; no pan/zoom or hover value | Smooth drag-pan, wheel-zoom, hover `(lat, lon, value)` readout, crisp dynamic station/coastline `Canvas` overlay | Slippy-map pan/zoom with cached `256×256` raster tiles; previously visited tiles stay instant when panning back |
| **UI (`omo-app::ui`)** | Single `image(png_path)` | Double-buffered `image(ImageSrc::rgba)` + transparent `canvas(Painter)` overlay (coastlines, stations, crosshair, legend) | Clipped `each(visible_tiles)` keyed `(rev,z,x,y)` `image(...)` + `canvas(Painter)` vector/station overlay + HUD |
| **Worker (`omo-engine`)** | Fetch bbox → rasterize 1 PNG → return `MapRef` | `map_load_field` caches 1-timestep grid in RAM (`≤ 26.4 MB`) → `map_render_view` renders viewport RGBA in ~`10–25 ms` | `map_load_field` caches 1-timestep grid in RAM → batched `map_render_tiles` renders missing `256×256` Web Mercator tiles in ~`5–15 ms` |
| **Projection** | Equirectangular (`EPSG:4326`) or Web Mercator | Equirectangular or Web Mercator (`EPSG:3857`) | Web Mercator (`EPSG:3857`, `2^z × 2^z`) or `EPSG:4326` TMS |
| **Basemap** | Natural Earth coastlines stroked into PNG | Natural Earth coastlines stroked in `Canvas` (or worker) — sharp at every zoom | Natural Earth coastlines stroked in `Canvas` / tile compositor (or local `.pmtiles` vector basemap) |
| **Export + Sidecar** | Direct copy/stage of the PNG + sidecar | Worker composites current viewport + overlays + legend into 1 PNG + sidecar | Worker composites current viewport + overlays + legend into 1 PNG + sidecar |
| **Complexity** | Low | Medium (best UX-to-complexity ratio for meteorology) | Medium-High (tile cache invalidation per revision/colormap, tile seam interpolation for O1280) |

---

## 6. Primary References

- Local Cargo cache (`nsh-* =0.5.1`):
  - `nsh-core-0.5.1/src/element/{asset.rs,builder.rs,host.rs,style.rs,responder.rs}`
  - `nsh-core-0.5.1/src/{image.rs,canvas.rs,ops.rs}` & `src/image/{loader.rs,decoded.rs,memory.rs}`
  - `nsh-platform-linux-0.5.1/src/{image.rs,canvas.rs}` & `nsh-platform-macos-0.5.1/src/{image.rs,canvas.rs}`
  - `nsh-base-0.5.1/src/plot/mod.rs`
- Upstream NSH: `https://github.com/longbridge/nsh` (`v0.5.1`–`v0.7.0`, `docs/dev/{canvas,images,elements,linux,macos}.md`)
- `OmO` codebase & specs:
  - `crates/omo-engine/src/grid/reduced_gaussian.rs`, `crates/omo-engine/src/series/{plan_b.rs,plan_g.rs}`, `crates/omo-engine/src/export.rs`
  - `crates/omo-worker/src/server.rs`, `crates/omo-app/src/host/{supervisor.rs,preview_queue.rs,export_scan.rs}`
  - `specs/002-m1-station-time-series/research.md`, `docs/m0/decisions/r4.md`, `docs/m1/evidence/S1-spark-aarch64-20261123.md`
