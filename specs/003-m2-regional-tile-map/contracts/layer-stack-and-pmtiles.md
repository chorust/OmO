# Contract: Web Mercator Tile Pyramid, Layer Stack, Offline Coastlines & Local PMTiles

## 1. Spherical Web Mercator (`EPSG:3857`) Tile Math (`FR-001`, `SC-006`)

- **Zoom range**: Integer `z ∈ 1..=12` (`2^z × 2^z` tiles per world, `256 × 256` logical pixels per tile; `world_px = 256 << z`).
- **Latitude domain**: Clamped to `[-85.05112878°, +85.05112878°]` (`±atan(sinh(π))`). Stations with `|lat| > 85.05112878°` are surfaced in the Map view's out-of-projection count/notice (`FR-011`).
- **Seamless global pixel addressing**:
  - For pixel `(u, v)` (`u, v ∈ 0..256`) of tile `(z, x, y)`:
    - `g_x = ((x as u64) * 256 + (u as u64)) & (world_px - 1)`
    - `g_y = (y as u64) * 256 + (v as u64)` (pixel center `g_y as f64 + 0.5`)
    - East-longitude index for O1280 ring lookup: `east_px = (g_x + (world_px >> 1)) & (world_px - 1)`, converting `[-180°, 180°)` to `[0°, 360°)` in exact integer arithmetic before floating-point division so adjacent tiles (including across `x = 2^z - 1 → 0` and `0°` Prime Meridian) receive bit-identical IEEE-754 inputs (`max |ΔRGBA| == 0`).
- **Antimeridian shortest-arc framing (`FR-002`)**:
  - Given `N` in-projection stations, sort normalized longitudes `λ_0 ≤ λ_1 ≤ ... ≤ λ_{N-1}` in `[-180°, 180°)`; find the largest gap `(λ_{(i+1)%N} - λ_i) mod 360°`, and take its complement as the minimal longitudinal span (applying a minimum `±2.0°` span when span `< 4.0°` or `N == 1`).

## 2. Ordered `LayerStack` & Value Provenance (`FR-012`–`FR-018`)

Layers render bottom-to-top according to their order in `MapViewState.layers`:

| `kind` | Rendered By | `value_provenance` (M2) | Opacity & Z-Order Rules |
|---|---|---|---|
| `builtin_coastlines` | UI `canvas(Painter)` (interactive) & Worker `tiny-skia` (export) | `display_only` | Default visible (`opacity = 1.0`); movable below or above `forecast_field` |
| `local_pmtiles` (raster) | Worker `image` → straight RGBA8 `256×256` tiles | `display_only` *(reserves `palette_decoded` post-M2)* | Respects `minzoom..maxzoom` (overzooms `maxzoom` up to `z = 12`; transparent below `minzoom` or outside bounds) |
| `local_pmtiles` (MVT) | Worker `mvt-reader` + `tiny-skia` → unpremultiplied RGBA8 `256×256` tiles | `display_only` | Deterministic built-in geometry palette (see §4) |
| `forecast_field` | Worker `TrapezoidBilinear` / 2D bilinear + 4096-entry LUT → straight RGBA8 `256×256` tiles | `native_numeric` | Default `opacity = 0.78` when coastlines/basemap sit underneath; `NaN` cells emit `[0,0,0,0]` |
| `station_overlay` | UI markers / `canvas(Painter)` (interactive) & Worker `tiny-skia` (export) | `display_only` *(click selects station)* | Pinned top data layer (`opacity` configurable) |

## 3. Built-In Offline Coastline & Boundary Asset (`OCB1`, `FR-014`)

- **Path**: `fixtures/geodata/ne_50m_coast_borders.ocb1` (Public Domain Natural Earth `1:50m` coastlines + land admin-0 boundary lines; provenance & SHA-256 recorded in `fixtures/PROVENANCE.md`).
- **Binary layout (`OCB1`, little-endian)**:
  - `magic`: `b"OCB1"` (4 B), `version`: `1u16`, `flags`: `0u16`, `line_count`: `u32`, `vertex_count`: `u32` (16 B header).
  - `line_index`: `line_count × 32 B` SoA/fixed records (`min_lon_e4: i32`, `min_lat_e4: i32`, `max_lon_e4: i32`, `max_lat_e4: i32`, `first_lon_e4: i32`, `first_lat_e4: i32`, `delta_offset: u32`, `vertex_len: u16`, `feature_kind: u8` `[0 = coastline, 1 = admin0_border]`, `pad: u8`).
  - `deltas`: `(vertex_count - line_count) × 4 B` (`dlon_e4: i16`, `dlat_e4: i16` in `10^-4` degrees, max spherical quantization error `≤ 7.7 m`).

## 4. Local `.pmtiles` v3 Validation & Deterministic MVT Theme (`FR-015`, `FR-016`)

- **Supported archive**: Local filesystem `.pmtiles` version 3 only (no `http://` / `https://` / `s3://` URLs in M2).
- **Compression**: Internal header/directory compression `None | Gzip | Brotli`; tile payload compression `None | Gzip | Brotli | Zstd` (decoded in pure Rust via `flate2`/`zlib-rs`, `brotli`, and `ruzstd`).
- **Supported tile types**:
  1. Raster: `Png`, `Jpeg`, `Webp` → decoded to straight RGBA8 `256×256` (rescaled to `256×256` if `512×512`).
  2. Vector: `Mvt` → decoded via `mvt-reader` and rasterized onto `256×256` `tiny-skia::Pixmap` (demultiplied to straight RGBA8) using the deterministic built-in geometry style:
     - Polygons: `water` / `ocean` / `waterway` (muted slate-blue fill), `earth` / `land` / `landcover` / `park` (neutral land/park tint), `building` (subtle neutral fill at `z >= 11`).
     - Polylines: `boundary` / `admin` (dashed/solid border stroke by `admin_level`), `transportation` / `road` (width scaled by `z`), `waterway` (thin blue stroke).
     - **Explicitly ignored in M2**: Point label placement, external MapLibre Style JSON, remote glyphs/fonts, and sprite sheets.
- **Error isolation**: Missing file, invalid magic/version, unsupported Zstd internal directory, or corrupt tile payload yields a structured layer diagnostic (`InvalidLayer` / `layer_errors`) without crashing the worker or blocking other layers.
