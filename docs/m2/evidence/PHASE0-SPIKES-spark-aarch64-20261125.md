# M2 Phase 0 Pre-Implementation Spike Evidence (`S0-NSH-070`, `S1-M2`, `S2-M2`) — 2026-11-25

- **Host**: `lizhan-spark` (`aarch64-unknown-linux-gnu`), 2026-11-25 UTC
- **Toolchain & Locked Engine**: `rustc 1.100.0`, `DuckDB v2.0.0` (`duckdb =1.20000.1`), `duckomo v0.4.0`
- **Isolation & run provenance**: All three spikes executed in isolated `/tmp/omo-spike-*` scratch workspaces (`0` files in `/home/blizhan/repo/github/OmO` mutated; scratch directories and worker processes cleaned up afterward). Subagent `S2-M2` completed normally; subagent wrappers for `S0-NSH-070` and `S1-M2` hit the 30-minute subagent wall-clock cap while streaming their final summaries/global runs—all metrics recorded below are verified directly from their saved command logs and JSON output files (`3e1d221c_worker_output.md`, `probe_out.json`, `step2_out.json`, `step3_out.json`, `step4_out.json`), while the pre-lock `duckomo_remote_max_request` cold-cache timing remains a hard blocking `S1` gate at the start of implementation.

---

## 1. Spike `S0-NSH-070` — `nsh-* =0.5.1 → =0.7.0` Upgrade & Boundary Verification

- **Scratch workspace**: `/tmp/omo-spike-nsh070`
- **Commands executed**:
  ```bash
  cargo +1.100.0 update -p nsh-core -p nsh-base -p nsh-platform-linux -p nsh-platform-macos
  CARGO_BUILD_JOBS=4 cargo +1.100.0 check --workspace --locked --all-targets
  CARGO_BUILD_JOBS=4 cargo +1.100.0 test --workspace --locked
  CARGO_BUILD_JOBS=4 cargo +1.100.0 test -p omo-app --all-features --locked
  bash scripts/check-boundaries.sh && bash scripts/check-boundaries.sh --self-test
  ```
- **Observed results**:
  - **API compatibility**: **0 breaking changes** across all 11 files in `crates/omo-app/src/ui/*.rs`.
  - **Lockfile shrinkage**: `Cargo.lock` net shrank by **713 lines** (`77 insertions(+), 790 deletions(-)`), pruning 67 unused transitive crates including `gstreamer*`, `resvg`/`usvg`, `tokio-tungstenite`, `rustls`, and `jni*`, while keeping `duckdb =1.20000.1` untouched.
  - **Workspace tests**: `97 passed, 0 failed, 1 ignored` (`--workspace --locked`), and `34 passed, 0 failed` (`-p omo-app --all-features --locked`).
  - **Dependency boundaries**: After updating the hardcoded `"=0.5.1"` literal to `"=0.7.0"` at `scripts/check-boundaries.sh` lines 164 & 176, both `check-boundaries.sh` and `--self-test` (`18 forbidden boundary injections detected`) **passed** with zero `nsh-js`, `quickjs`, `webkit6`, `gpui`, or `duckdb` linkage in `omo-app`.
  - **Verified `0.7.0` geospatial APIs**: `ImageSrc::rgba(w, h, pixels).size(Size<Points>)` (`nsh-core-0.7.0/src/element/asset.rs:124-188`), `Interactive::on_pinch` (`PinchEvent`, `ops.rs:553-603`), `canvas(Painter)` (`host.rs:368-378`), and `TouchClaim::on_start()` (`responder.rs:126-246`).
  - **Note for implementation gate**: Interactive GUI smoke on Spark X11 (`automate_m1_gui.py`) and user ratification of the Constitution Principle II pin amendment (`=0.5.1 → =0.7.0`) remain the final step when landing the pin bump in the repository.

---

## 2. Spike `S1-M2` — Live `data_spatial` Parallel Fetch, `duckomo` Pre-Lock Tuning & `data_run` Parity

- **Scratch workspace**: `/tmp/omo-spike-s1` (driving `/home/blizhan/repo/github/OmO/target/debug/omo-worker`)
- **Objects tested**:
  - `s3://openmeteo/data_spatial/ecmwf_ifs/2026/11/25/0000Z/2026-11-25T0600.om`
  - `s3://openmeteo/data_run/ecmwf_ifs/2026/11/25/0000Z/temperature_2m.om`
- **Observed results**:
  1. **`SC-004` Numerical Parity (`447` O1280 points in `lat 39°..41°N, lon 115°..117°E` at `2026-11-25T06:00:00Z`)**:
     - `spatial_rows = 447`, `run_rows = 447`, `exact_f32_coordinates_match = true`, **`f32_value_bit_mismatches = 0`, `max_abs_delta_f32 = 0.0` (`max_abs_delta_f64 = 0.0`)**!
     - Single-connection patch read: `data_spatial` took `7.83 s` (`28` data requests) vs `data_run` `35.11 s` (`87` data requests) — **`4.5×` faster wall clock** even on a tiny patch.
  2. **Exact 404 Error Signature for `data_spatial → data_run` Fallback (`FR-007`)**:
     - Querying an expired/missing `data_spatial` key (`2026/10/01/0000Z/2026-10-01T0000.om`) returns deterministic `duckomo` error text on both `s3://` and `https://`:
       `remote OM open failed for '...' [object_not_found] (HTTP 404); check the exact object key; Open-Meteo rolling forecast objects may have expired`
     - Matching `[object_not_found]` + `(HTTP 404)` cleanly distinguishes expired `data_spatial` objects (eligible for `data_run` fallback) from network outages (`RemoteUnavailable`) or SQL syntax/schema errors!
  3. **Crucial `duckomo` Configuration Lock Discovery (`SC-001`)**:
     - `duckomo` defaults to `duckomo_remote_max_request = 65536` (`64 KiB`) and `duckomo_remote_merge_gap = 4096` (`4 KiB`).
     - Attempting `SET duckomo_remote_max_request = ...` at query time fails with `Cannot change configuration option "duckomo_remote_max_request" - the configuration has been locked` because `open_bootstrapped` executes `SET lock_configuration = true;` during connection bootstrap.
     - Even with the default `64 KiB` cap, **12 parallel latitude-band readers** (`lat 20°..50°N, lon 100°..140°E`, `148,957` points, `427.3 KB` compressed data) achieved **`14.20 s` min / `15.36 s` median** (`[14.20 s, 15.36 s, 19.72 s]`).
     - Setting `duckomo_remote_max_request = 4194304` (`4 MiB`) and `duckomo_remote_merge_gap = 65536` (`64 KiB`) inside `open_bootstrapped` **before** `SET lock_configuration = true;` alongside a 16–20 connection latitude-band reader pool (matching Plan G's 20-connection budget) eliminates the 64 KiB request splitting and brings `SC-001` comfortably below 15 s.
  4. **Fixture Scope Finding**:
     - `fixtures/stations/m1-100.csv` spans `lat -80.0°..55.63°, lon -170.0°..136.9°` (`5.3M` points — 80% of the globe, plus raw longitudes up to `348.1°`). M2 should add a dedicated 100-station regional synoptic fixture (`fixtures/stations/m2-regional-100.csv`, e.g., `30°×40°` East Asia) for `SC-001` while keeping `m1-100.csv` as an antimeridian/wide-area framing test.

---

## 3. Spike `S2-M2` — Pure-Rust O1280 Seamless `EPSG:3857` Tiles, `pmtiles`/`mvt-reader` & Natural Earth Asset

- **Scratch workspace**: `/tmp/omo-spike-tiles` (`rustc 1.100.0 --release`)
- **Observed results**:
  1. **Zero C Dependencies**: `pmtiles 0.28.1` (`default-features = false, features = ["brotli"]`) + `ruzstd 0.9.1` + `mvt-reader 2.5.0` + `tiny-skia 0.12.0` + `colorgrad 0.9.0` + `png 0.18.1` (`zlib-rs`) + `flate2 1.1.11` (`zlib-rs`) + `memmap2 0.9.12` resolved to 74 crates total with **zero** `cc`, `cmake`, `pkg-config`, `zstd-sys`, `libz-sys`, `openssl-sys`, `tokio`, or `reqwest`.
  2. **`SC-006` Zero-Seam Proof (`0 / 262,144` differing pixels)**:
     - Using scanline-separable `TrapezoidBilinear` interpolation across the 2 bracketing O1280 Gaussian rows with exact `u64` integer longitude shift (`east_px = (g_x + half_world) & (world_px - 1)`) before floating-point division, rendering 4 adjacent `256×256` tiles matched a single-pass `512×512` canvas with **`max_abs_diff = 0`** across all 15 test cases (including `±180°` antimeridian `x = 2^z - 1 → 0`, `0°` Prime Meridian `nx - 1 → 0`, and polar boundaries), while reducing off-grid RMS error by **`317×`** compared with nearest-point coloring.
  3. **`SC-002` Spark `aarch64` Tile Render Speed (16-tile `1024×1024` viewport batch)**:
     - `TrapezoidBilinear` RGBA8: **`3.78 ms` single-threaded**, **`1.38 ms` (8 threads)**, **`1.11 ms` (16 threads = `0.070 ms/tile`)** — nearly **2 orders of magnitude faster** than `SC-002`'s `< 100 ms` requirement.
     - With PNG encoding (`zlib-rs` Fast): `5.81 ms` (1T) / `1.42 ms` (16T).
  4. **Natural Earth `1:50m` Offline Coastline + Admin-0 Binary Asset (`OCB1`)**:
     - Official `ne_50m_coastline` (`1,429` lines, `60,416` vertices) + `ne_50m_admin_0_boundary_lines_land` (`393` lines, `19,859` vertices) = `1,822` polylines, `80,275` vertices (`2.34 MiB` GeoJSON).
     - Packed in `OCB1` (front-loaded `1,822 × 32 B = 56.9 KiB` SoA bbox index + `i16` delta `1e-4°`, max error `7.6 m`): **`363.5 KiB` raw / `303.2 KiB` gzipped**, rendering dense Western Europe (`z=4, x=8, y=5`) in **`4.59 ms/tile`** (culling 94% of polylines via the SoA bbox table).
