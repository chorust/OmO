# Implementation Plan: M2 Regional Tile Map & Layer Stack

**Branch**: `003-m2-regional-tile-map` (spec directory only; no git branch created) | **Date**: 2026-11-25 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/003-m2-regional-tile-map/spec.md`

## Summary

M2 adds spatial weather context around user stations using a standard **Web Mercator (`EPSG:3857`) `256×256` `z/x/y` slippy tile pyramid and ordered multi-layer stack**. Rather than rendering a single static image or making remote S3 calls per tile, `omo-worker` loads and caches a single-timestep numeric forecast field primarily from Open-Meteo's `data_spatial` objects (falling back automatically once to `data_run` with a visible slow-path notice when historical pinned runs exceed `data_spatial`'s ~7-day retention), then rasterizes `z/x/y` tiles locally in milliseconds (`1.11 ms` per 16-tile viewport batch measured in Phase 0 spike `S2-M2`).

Technical approach (see [research.md](research.md) and [PHASE0-SPIKES-spark-aarch64-20261125.md](../../docs/m2/evidence/PHASE0-SPIKES-spark-aarch64-20261125.md)):
- **Shell pre-gate (`S0-NSH-070`)**: Upgrade `nsh-*` from `=0.5.1` to official release `=0.7.0` (`default-features = false`; verified `0` UI breaking changes, `-713` lines in `Cargo.lock`, all 97 tests and 18 boundary checks green in isolated Phase 0 spike), followed by Spark X11 GUI smoke and user ratification of the Constitution Principle II pin amendment before M2 UI tasks (retaining `=0.5.1` seamlessly if any GUI regression appears).
- **Pre-lock `duckomo` tuning & parallel `data_spatial` fetch (`S1-M2`)**: Configure `duckomo_remote_max_request = 4194304` (`4 MiB`) and `duckomo_remote_merge_gap = 65536` (`64 KiB`) inside `open_bootstrapped` *before* `SET lock_configuration = true;`, and read `data_spatial` (`variable := '<var>'`) across a bounded latitude-band reader pool (`12–16` `BootstrappedConn` connections sharing `SeriesCancel`).
- **Zero-seam O1280 & regular-grid tile rasterization**: Scanline-separable `TrapezoidBilinear` interpolation across bracketing O1280 Gaussian rows with exact `u64` integer longitude shift (`max |ΔRGBA| == 0` proven across interior and `±180°` antimeridian seams) + uniform field-wide color normalization (`4096`-entry `colorgrad` LUT).
- **Extensible `LayerStack` & value-provenance seam**: Built-in `303 KiB` (`363.5 KiB` raw) `OCB1` Natural Earth `1:50m` offline coastline/border asset + zero-C-dependency pure-Rust local `.pmtiles` v3 reader (`pmtiles` + `ruzstd` + `mvt-reader` + `tiny-skia` + `image`), paired with `ValueProvenance` (`native_numeric` | `display_only`, reserving `palette_decoded` for post-M2 `radiust`-style color-to-value inversion).
- **Atomic composite PNG export & `.omoproj` persistence**: Reuse `"export"` (`format: "png"`) + `omo-export-sidecar/3` (`commit_staged`) and additive `ProjectDoc.map` in `omo-project/1`.

## Technical Context

**Language/Version**: Rust `1.100.0` (`rust-toolchain.toml`), workspace MSRV `1.97`. Python 3 for dev/CI verification and fault-injection scripts.

**Primary Dependencies**:
- **Locked / Gated**:
  - `nsh-core`, `nsh-base`, `nsh-platform-linux`, `nsh-platform-macos`: currently `=0.5.1` (`=0.7.0` evaluated and build-verified in `S0-NSH-070`, gated on X11 smoke + Constitution II amendment in Phase 1 setup; `omo-app` only, `default-features = false`).
  - `DuckDB v2.0.0` / `duckdb =1.20000.1` (`omo-engine` / `omo-worker` only) + locked `duckomo v0.4.0` (`community-signed` preferred, `github-release-unsigned` explicit).
- **New UI-free pure-Rust crates (`omo-engine` only; verified 0 C/system/-sys/async-HTTP dependencies in `S2-M2`)**:
  - `pmtiles = { version = "0.28.1", default-features = false, features = ["brotli"] }` + `ruzstd = "0.9.1"` + `flate2 = { version = "1.1.11", default-features = false, features = ["zlib-rs"] }` (local `.pmtiles` v3 sync `mmap` reader);
  - `mvt-reader = "2.5.0"` (pure-Rust MVT v1/v2 geometry decoder, `prost`-backed, no `protoc`);
  - `tiny-skia = "0.12.0"` (CPU 2D vector MVT & composite PNG painter);
  - `colorgrad = { version = "0.9.0", features = ["preset"] }` (scalar colormaps → 4096-entry RGBA8 LUT);
  - `png = { version = "0.18.1", features = ["zlib-rs"] }` + `image = { version = "0.25.10", default-features = false, features = ["png", "jpeg", "webp"] }` (composite PNG encoder & raster `.pmtiles` tile decoder);
  - `memmap2 = "0.9.12"` (already in workspace / thin `mmap` wrapper for `.pmtiles` and `.rgba` tile buffers).

**Storage**:
- Project files: `*.omoproj` (`omo-project/1` with additive `#[serde(default)] map: MapViewState`);
- Crash-recovery drafts: `<app_data>/drafts/<doc_id>.json`;
- Session map artifacts: `<session_tmp>/g<gen>/tile_*.rgba` (`256×256` straight RGBA8) and `field_*.f32bin`, reaped on stale-generation discard and generation rollover;
- Built-in offline geodata: `fixtures/geodata/ne_50m_coast_borders.ocb1` (`363.5 KiB` Public Domain Natural Earth `1:50m` coastlines + admin-0 borders);
- Map exports: `<target>.png` + atomic sidecar `<target>.png.omo.json` (`omo-export-sidecar/3`);
- Catalog: `catalog/products.json` (`omo-catalog/1` with optional `spatial_layout`).

**Testing**:
- `cargo +1.100.0 test --workspace --locked`: unit & contract tests for `EPSG:3857` tile math, antimeridian shortest-arc framing, O1280 `TrapezoidBilinear` 0-seam equivalence (`SC-006`), `OCB1` coastline clipping, `.pmtiles` raster/MVT/corrupt archives (`SC-005`), `omo-catalog/1` `spatial_layout`, `omo-project/1` M1→M2 `.omoproj` round-trip (`SC-008`), `MapPreviewKey`, `omo-export-sidecar/3`, and `SupervisorCore` stale-generation directory reaping (`SC-007`);
- `omo-harness export-kill --at step1|step2|step3 --source map` (`SC-007`);
- `scripts/fault-inject-map.py` (cancel, `kill -9`, offline; ≥ 10 trials each) and `scripts/perf-map.py` (`SC-001`, `SC-002`);
- `scripts/automate_m2_gui.py` (isolated Xvfb X11 GUI verification on Spark) and `scripts/ci-verify-map.py` in `.github/workflows/ci.yml` (`SC-009`).

**Target Platform**:
- Reference interactive host: `lizhan-spark` (`aarch64-unknown-linux-gnu`, X11);
- GitHub Actions CI & release (Constitution 1.3.0): `linux_amd64`, `linux_arm64`, `osx_arm64`, plus build-only `macos-15-intel`.

**Project Type**: Native pure-Rust desktop workbench (`omo-app`, NSH) supervising an out-of-process analytical engine worker (`omo-worker`).

**Performance Goals**:
- Cold regional `data_spatial` field load (`30°×40°` 100-station M2 fixture): median **< 30 s** across 5 runs (relaxed from 15 s) (`SC-001`);
- Cached pan/zoom `256×256` tile viewport batch render: **< 100 ms** p95 (`~1–5 ms` measured in `S2-M2`), `0` remote requests, `0` blank-frame flashes (`SC-002`).

**Constraints**:
- Zero UI crates in `omo-engine`, `omo-protocol`, `omo-worker`, `omo-project`; zero `duckdb`, QuickJS, WebView, or HTTP clients in `omo-app`; zero `nsh_*` tokens in `omo-app::host` (`scripts/check-boundaries.sh`);
- Single active request per `omo-worker` (`MapRequestScheduler` serializes/coalesces requests on host);
- Worker `CachedField` LRU capped at 2 fields (`≤ 53 MB` RSS); NSH texture `MemoryCache` bounded at `≤ 256 MiB`;
- Zero contacts to online third-party XYZ tile servers (`FR-018`).

**Scale/Scope**:
- Up to 1,000 stations (acceptance measured at 100 regional stations + 100 global M1 stations), zoom `z = 1..12`, global O1280 (`6,599,680` points) and regular spherical grids, 1 valid time per map load, multiple ordered layers (`.pmtiles` raster/MVT + built-in coastlines + forecast + stations).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Pre-Research Check | Post-Design Check | Notes |
|---|---|---|---|
| **I. UI-Free Core Crates** | **PASS** | **PASS** | All tile math, `CachedField`, `OCB1`, `.pmtiles`, and `tiny-skia` compositing live in `omo-engine`; wire/catalog/sidecar types in `omo-protocol`; persisted state in `omo-project`; scheduler in `omo-app::host`; NSH `image`/`canvas` strictly confined to `omo-app::ui`. |
| **II. One Native Shell: NSH Pure Rust, Exactly Pinned** | **PASS (with Amendment Gate)** | **PASS (with Amendment Gate)** | Pure-Rust NSH only (`default-features = false`, zero QuickJS/WebView/GPUI). Upgrading `=0.5.1 → =0.7.0` passed build/test/boundary checks in `S0-NSH-070` and requires user ratification of a Constitution II pin amendment in Phase 1 setup before landing; mandatory M2 FRs/SCs also work on `=0.5.1`. |
| **III. Pinned Engine, Per-Connection Verified Bootstrap** | **PASS** | **PASS** | `DuckDB v2.0.0` (`=1.20000.1`) and `duckomo v0.4.0` unchanged. Pre-lock `duckomo_remote_max_request` / `duckomo_remote_merge_gap` settings apply inside `open_bootstrapped` before `SET lock_configuration = true;`. All remote reads stay in worker `httpfs`. |
| **IV. Supervised Engine Process & Explicit State Boundaries** | **PASS** | **PASS** | Generation-tagged `results/g{gen}/*.rgba` artifacts, `MapPreviewKey` revision/stale retention, `SeriesCancel` multi-connection interrupt, recursive stale-generation directory reaping, and single-commit-point `omo-export-sidecar/3`. |
| **V. Staged, Gated Dynamic Reports** | **PASS** | **PASS** | L0 maintained: no TSX, QuickJS, or dynamic report execution in M2. |
| **VI. Evidence Before Claims** | **PASS** | **PASS** | Phase 0 unknowns resolved with recorded measurements on Spark (`docs/m2/evidence/PHASE0-SPIKES-spark-aarch64-20261125.md`); `S0`/`S1` and `V1`–`V8` gates defined in `quickstart.md`. |
| **VII. Hands Off Neighbouring Repositories** | **PASS** | **PASS** | `Duckling`, `duckomo`, and `radiust` are untouched. Natural Earth `1:50m` Public Domain provenance recorded in `fixtures/PROVENANCE.md`. |

## Project Structure

### Documentation (this feature)

```text
specs/003-m2-regional-tile-map/
├── spec.md                         # Feature specification
├── plan.md                         # This implementation plan
├── research.md                     # Phase 0 research & spike synthesis
├── data-model.md                   # Phase 1 entity & state definitions
├── quickstart.md                   # Phase 1 end-to-end validation guide
├── checklists/
│   └── requirements.md             # Spec quality checklist
├── contracts/
│   ├── catalog-m2.md               # omo-catalog/1 spatial_layout contract
│   ├── worker-protocol-m2.md       # Map load/tile/inspect/probe/export IPC
│   ├── layer-stack-and-pmtiles.md  # EPSG:3857 math, LayerStack, OCB1, PMTiles/MVT
│   ├── export-sidecar-v3.md        # omo-export-sidecar/3 PNG sidecar contract
│   └── project-file-m2.md          # omo-project/1 .omoproj MapViewState contract
└── tasks.md                        # Phase 2 output (/speckit.tasks - NOT created here)
```

### Source Code (repository root)

```text
catalog/products.json                         # EXTEND: add spatial_layout for ecmwf_ifs.temperature_2m
crates/
├── omo-protocol/src/
│   ├── map.rs                                # NEW: MapLoadField*, MapRenderTiles*, MapInspect*,
│   │                                         #      PmtilesLayerInfo, ValueProvenance, TileCoord
│   ├── series.rs                             # EXTEND: SpatialLayout on CatalogEntry, validation
│   ├── export.rs                             # EXTEND: SIDECAR_SCHEMA_V3 ("omo-export-sidecar/3")
│   └── state.rs                              # EXTEND: ValidTimeUnavailable, InvalidLayer FailKinds
├── omo-project/src/
│   ├── map.rs                                # NEW: MapViewState, MapViewport, ColorScaleConfig,
│   │                                         #      MapLayerConfig, -0.0 -> 0.0 canonicalization
│   ├── doc.rs                                # EXTEND: #[serde(default)] map: MapViewState +
│   │                                         #         pre-deserialize schema major check
│   └── preview_key.rs                        # EXTEND: MapPreviewKey
├── omo-engine/src/
│   ├── bootstrap.rs / config.rs              # EXTEND: pre-lock duckomo_remote_max_request &
│   │                                         #         duckomo_remote_merge_gap configuration
│   ├── catalog/mod.rs                        # EXTEND: spatial_object_url template builder
│   ├── map/
│   │   ├── mod.rs                            # NEW: CachedField LRU, PointReadout inspector
│   │   ├── fetch.rs                          # NEW: parallel latitude-band data_spatial reader +
│   │   │                                     #      [object_not_found] (HTTP 404) -> data_run fallback
│   │   ├── mercator.rs                       # NEW: EPSG:3857 math, shortest-arc antimeridian bbox
│   │   ├── raster.rs                         # NEW: O1280 TrapezoidBilinear & regular 2D tile
│   │   │                                     #      rasterizer + 4096-entry colorgrad LUT
│   │   ├── coastlines.rs                     # NEW: OCB1 offline coastline/border reader & clipper
│   │   ├── pmtiles_layer.rs                  # NEW: sync mmap PMTiles v3 raster & deterministic MVT
│   │   └── compose_export.rs                 # NEW: settled multi-layer + legend + stations PNG
│   └── export.rs                             # EXTEND: stage & commit composite PNG + sidecar /3
├── omo-worker/src/
│   ├── server.rs                             # EXTEND: dispatch map_load_field, map_render_tiles,
│   │                                         #         map_inspect, map_layer_probe, PNG export
│   └── main.rs                               # EXTEND: EngineBackend CachedField + SeriesCancel
└── omo-app/src/
    ├── bin/omo-harness.rs                    # EXTEND: export-kill --source map
    ├── host/
    │   ├── map_host.rs                       # NEW: MapRequestScheduler, RGBA tile loader,
    │   │                                     #      f32bin hover reader, sidecar /3 builder
    │   ├── supervisor.rs                     # EXTEND: track map kinds + allow .rgba/.png/.f32bin +
    │   │                                     #         recursive g{old_gen}/ directory reaper
    │   └── runtime.rs                        # EXTEND: route map requests & PNG export tracking
    └── ui/
        ├── workspace.rs                      # EXTEND: center "Map" tab + File -> Export map PNG...
        ├── map_view.rs                       # NEW: keyed ImageSrc::rgba tile grid + canvas(Painter)
        │                                     #      coastlines/markers/HUD + responder pan/wheel/pinch
        ├── map_controls.rs                   # NEW: valid-time stepper, LayerStack, palette/range,
        │                                     #      Fit-to-stations, PMTiles file picker
        └── project_ctl.rs                    # EXTEND: sync MapViewState into fingerprint/save/open/draft
fixtures/
├── geodata/ne_50m_coast_borders.ocb1         # NEW: 363.5 KiB Public Domain Natural Earth 1:50m
├── stations/m2-regional-100.csv              # NEW: 100-station East Asia synoptic fixture (SC-001)
├── pmtiles/{sample-raster,sample-mvt,corrupt}.pmtiles  # NEW: deterministic offline test archives
└── PROVENANCE.md                             # EXTEND: Natural Earth & PMTiles fixture provenance
scripts/
├── build-ne50m-ocb1.py                       # NEW (dev): reproducible OCB1 builder from Natural Earth
├── perf-map.py                               # NEW: SC-001 / SC-002 benchmark runner
├── fault-inject-map.py                       # NEW: SC-007 10-trial cancel/kill9/offline runner
├── automate_m2_gui.py                        # NEW: isolated Xvfb GUI verification driver
└── ci-verify-map.py                          # NEW: cross-platform CI smoke test (SC-009)
docs/m2/
├── research-nsh-geospatial.md                # Pre-spec NSH & Duckomo research
├── acceptance.md                             # M2 acceptance matrix (filled during implementation)
└── evidence/                                 # S0, S1, V1-V8 & CI evidence records
```

**Structure Decision**: Preserve the five-crate workspace mandated by Constitution Principle I with zero new workspace crates. All map math, `data_spatial` I/O, tile rasterization, `OCB1` geometry, `.pmtiles` decoding, and PNG export compositing reside in UI-free `omo-engine::map`; wire contracts in `omo-protocol::map`; persisted state in `omo-project::map`; request serialization in `omo-app::host::map_host`; and NSH rendering in `omo-app::ui::{map_view,map_controls}`.

## Phasing for `/speckit.tasks` (Guidance)

1. **Phase 1 — Setup & Gate `S0-NSH-070`**:
   - Propose & ratify Constitution Principle II pin amendment (`=0.5.1 → =0.7.0`), bump `nsh-*` pins + `scripts/check-boundaries.sh`, add pure-Rust `omo-engine` dependencies (`pmtiles`, `ruzstd`, `mvt-reader`, `tiny-skia`, `colorgrad`, `png`, `image`), and run M1 X11 GUI regression smoke (`S0`).
   - Generate and verify `fixtures/geodata/ne_50m_coast_borders.ocb1`, `m2-regional-100.csv`, and `.pmtiles` test fixtures.
2. **Phase 2 — Foundational Engine & Gate `S1-M2`**:
   - Pre-lock `duckomo_remote_max_request` / `duckomo_remote_merge_gap` in `open_bootstrapped`; `SpatialLayout` catalog support; parallel latitude-band `data_spatial` fetcher + `[object_not_found] (HTTP 404) → data_run` fallback; **pass `S1-M2` (`SC-001` `< 30 s` median, `SC-004` bit-exact parity) before UI work**.
   - `EPSG:3857` math, antimeridian shortest-arc bbox, `TrapezoidBilinear` / regular-grid tile rasterizer (`SC-002`, `SC-006`), `PointReadout` inspector (`SC-003`), `OCB1` coastline clipper, and `.pmtiles` raster/MVT renderer (`SC-005`).
3. **Phase 3 — Protocol, Worker Supervision & Persistence**:
   - `omo-protocol::map` + `SIDECAR_SCHEMA_V3`, `omo-worker` handlers, `SupervisorCore` tracked kinds + stale-generation `g{old_gen}/` reaper (`SC-007`), `MapRequestScheduler`, and `omo-project/1` `MapViewState` + `MapPreviewKey` (`SC-008`).
4. **Phase 4 — NSH Map UI & Composite Export**:
   - `map_view.rs` (`ImageSrc::rgba` keyed tiles + `canvas(Painter)` coastlines/stations/HUD + responder pan/wheel/pinch), `map_controls.rs` (`LayerStack`, valid-time stepper, palette/range, `.pmtiles` picker), `project_ctl.rs` dirty/draft sync, and composite PNG + `.png.omo.json` atomic export + `omo-harness export-kill --source map`.
5. **Phase 5 — Acceptance & CI**:
   - `scripts/perf-map.py`, `scripts/fault-inject-map.py`, `scripts/automate_m2_gui.py`, `scripts/ci-verify-map.py` wired into `.github/workflows/ci.yml` (`SC-009`), and `docs/m2/acceptance.md`.

## Complexity Tracking

No constitution violations. Significant dependencies and mechanisms are justified below for review transparency:

| Addition | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| Pure-Rust `.pmtiles` + `mvt-reader` + `ruzstd` + `tiny-skia` in `omo-engine` | Required by user-selected scope (`FR-015`: ordered multi-layer stack hosting local raster & vector `.pmtiles` archives + composite PNG export) | Online XYZ tiles violate offline/no-scraping rules (`FR-018`); `pmtiles/zstd` or `maplibre-rs` pull C `zstd-sys`, `tokio`/`reqwest`, or `wgpu` GPU dependencies |
| Parallel latitude-band `data_spatial` fetcher + pre-lock `duckomo` request tuning | `duckomo` locks `duckomo_remote_max_request` (`64 KiB` default) on bootstrap, causing single-connection `30°×40°` reads to take `34.7–85.7 s` (`S1-M2`) | Single-connection un-tuned reads miss `SC-001` (`< 30 s`), and per-tile S3 queries stall for `~7.6 s/tile` and hit worker `busy` errors |
| `nsh-* =0.5.1 → =0.7.0` gated pin upgrade | User-requested evaluation (`S0-NSH-070`): unlocks HiDPI `ImageSrc::size`, macOS zero-copy opaque `CGImage`, and desktop `on_pinch` while shrinking `Cargo.lock` by 713 lines | Kept strictly gated behind X11 regression smoke + Constitution Principle II amendment; mandatory FRs remain compatible with `=0.5.1` if reverted |
