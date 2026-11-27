# Quickstart: Validating M2 Regional Tile Map & Layer Stack

These are runnable validation scenarios proving M2 end-to-end against [spec.md](spec.md). Contracts:
- [catalog extension](contracts/catalog-m2.md)
- [worker protocol M2](contracts/worker-protocol-m2.md)
- [layer stack, coastlines & PMTiles](contracts/layer-stack-and-pmtiles.md)
- [export sidecar `/3`](contracts/export-sidecar-v3.md)
- [project file `.omoproj` M2](contracts/project-file-m2.md)

All recorded evidence goes to `docs/m2/evidence/` per Constitution Principle VI.

## Prerequisites

- Reference host `lizhan-spark` (`aarch64-unknown-linux-gnu`, X11 / isolated `Xvfb` session), Rust `1.100.0`, locked `duckomo` v0.4.0 artifacts, and network access to `s3://openmeteo` (plus `bwrap --unshare-net` or `no_network` mode for offline checks).
- Baseline workspace checks:
  ```bash
  cargo +1.100.0 build --workspace --locked
  cargo +1.100.0 test --workspace --locked
  scripts/check-boundaries.sh && scripts/check-boundaries.sh --self-test
  ```

---

## S0 — NSH `=0.7.0` Upgrade & Interactive Regression Gate (before M2 UI work)

1. Apply the `nsh-* =0.5.1 → =0.7.0` pin update (`default-features = false`), update `scripts/check-boundaries.sh`, and run:
   ```bash
   cargo +1.100.0 test --workspace --locked
   cargo +1.100.0 test -p omo-app --all-features --locked
   scripts/check-boundaries.sh && scripts/check-boundaries.sh --self-test
   ```
2. Run M1's automated X11 GUI smoke (`scripts/automate_m1_gui.py`) to confirm zero regression on existing Table/Plot/SQL/Draft/Close-guard views.
3. Record `docs/m2/evidence/S0-NSH070-spark-aarch64-<date>.md` and present the Constitution Principle II pin amendment (`=0.5.1 → =0.7.0`) for user confirmation. (If any blocker occurs, revert to `=0.5.1` and proceed on `=0.5.1`.)

---

## S1 — Pre-Lock `duckomo` Parallel `data_spatial` Performance Gate (`SC-001`)

1. Run the M2 map field benchmark (`scripts/perf-map.py` against `fixtures/stations/m2-regional-100.csv`, `30°×40°` synoptic extent, 5 cold-field trials on `data_spatial/ecmwf_ifs` with pre-lock `duckomo_remote_max_request = 4194304` and parallel latitude-band readers).
2. Record median wall time, `bytes_fetched`, and `read_requests` in `docs/m2/evidence/S1-spark-aarch64-<date>.md`.
3. Expected: median `< 15 s` (`SC-001`). If missed, stop and ask the user before changing the target.

---

## V1 — Default Regional Tile Map & Cached Navigation (`US1`; `SC-001`, `SC-002`)

1. Start `omo-app`, import `fixtures/stations/m2-regional-100.csv` (100 regional stations), confirm mapping, and open the **Map** tab.
2. Trigger map load for default `ecmwf_ifs.temperature_2m` (`lead 0 h` of latest complete run):
   - Viewport frames the 100-station bounding box + margin;
   - `256×256` forecast tiles, offline coastlines, station markers, and color scale legend (`°C`) render with status `"received, not saved"`.
3. Drag-pan and scroll-zoom within the cached extent:
   - Measure tile batch latency (`< 100 ms` p95, `0` remote requests, `0` blank-frame flashes on already-mounted tiles).
4. Step `valid_time` to `+6 h`: previous map marks `stale` until `+6 h` settles.

---

## V2 — Numeric Point Readout & `data_spatial`/`data_run` Parity (`SC-003`, `SC-004`)

1. Against a pinned run and valid time, run `map_inspect` across 57+ reference points (station coordinates, off-grid points, antimeridian `±180°`, polar `>85.05°`, and null test cells) and compare with M1 `station_series` / official `omfiles` oracle:
   - Expected: `100%` coordinate and value equality (`max |Δ| == 0.0`), provenance `"native_numeric"`.
2. Repeat `map_load_field` on the same pinned run with `force_data_run: true` (and test an expired `data_spatial` run key returning `[object_not_found] (HTTP 404)`):
   - Expected: automatic single fallback to `data_run`, fallback notice surfaced, `source_layout == "data_run"`, and `max |Δ| == 0.0` across all regional points (`SC-004`).

---

## V3 — Zero-Seam Tile & Antimeridian Verification (`SC-006`)

Run `cargo +1.100.0 test -p omo-engine --test map_tiles --locked`:
- Compares `2×2` `256×256` tiles against single-pass `512×512` renders across interior seams, `0°` Prime Meridian, and `±180°` antimeridian (`x = 2^z - 1 → 0`), plus antimeridian shortest-arc station framing (`+178°` & `-178°` → `4°` Pacific window).
- Expected: `0` differing pixels (`max |ΔRGBA| == 0`).

---

## V4 — Offline Mode, Layer Stack & Local `.pmtiles` (`US2`; `SC-005`)

1. Run worker/app under offline isolation (`bwrap --unshare-net` or `no_network` config):
   - Built-in Natural Earth `1:50m` coastlines/borders render across `z = 1..12` with `0` network calls.
   - Mount `fixtures/pmtiles/sample-raster.pmtiles` (PNG/WebP) and `fixtures/pmtiles/sample-mvt.pmtiles` (MVT vector basemap): both render aligned in `EPSG:3857`, surface archive attribution, and report `"display_only"` on point inspection.
   - Mount `fixtures/pmtiles/corrupt.pmtiles`: reports a per-layer error without crashing the worker or hiding healthy layers.
   - Toggle visibility, adjust opacity (`0–100%`), and reorder layers; verify hiding the forecast layer suppresses its numeric readout.

---

## V5 — Composite PNG Export & `export-kill` (`US3`; `SC-007`)

1. Export a settled multi-layer map to `map.png`:
   - Verify `map.png` dimensions and visual composition (layers, stations, colorbar, header, attribution) and `map.png.omo.json` (`omo-export-sidecar/3` schema, `/data/sha256`, `source_layout`, `layers`, `attribution`).
2. Run fault-injected commits:
   ```bash
   for at in step1 step2 step3; do
     ./target/debug/omo-harness export-kill --at $at --source map
   done
   ```
   Expected startup scan verdicts: `incomplete` (`saved=false`), `incomplete` (`saved=false`), `finished` (`saved=true`).

---

## V6 — `.omoproj` Round-Trip, M1 Compatibility & Draft Recovery (`US3`; `SC-008`)

1. Open `fixtures/projects/m1-roundtrip.omoproj`: succeeds with default M2 `MapViewState`.
2. Customize map viewport, valid time (`+12 h`), locked color range, and `.pmtiles` layer stack; save as `m2-roundtrip.omoproj`, reopen on Linux and macOS CI, and verify byte-identical canonical JSON hash and crash-draft recovery.

---

## V7 — Fault Injection & Stale-Generation Tile Reaping (`SC-007`, ≥ 10 trials each)

Run `scripts/fault-inject-map.py --trials 10`:
- Cancel during `map_load_field` reading → `Cancelled`, prior map kept `stale`;
- `kill -9` worker during `map_load_field` or `map_render_tiles` → `WorkerCrashed`, generation increments `g -> g+1`, `<result_root>/g<g>/` reaped with **0 leaked `.rgba`/`.png`/`.f32bin` files**, retry succeeds;
- Offline before request → `CapabilityUnavailable(remote)`, prior map kept `stale`.

---

## V8 — Cross-Platform CI Smoke (`SC-009`)

Run `python3 scripts/ci-verify-map.py` locally and in `.github/workflows/ci.yml` on `linux_amd64`, `linux_arm64`, and `osx_arm64` (`map_load_field` + 4-tile `map_render_tiles` + `map_inspect` + composite PNG `export` against the latest complete run; network outages reported as `skipped(network)`, never as a pass).
