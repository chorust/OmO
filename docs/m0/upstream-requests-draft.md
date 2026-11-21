# Draft: duckomo upstream requests

**Status:** draft for review only. Nothing in this file has been sent or posted. Whether to file requests remains OQ-3.

## 1. Push selected-point filters into `read_om_series`

The locked v0.4.0 table function has no selected-point argument. In the M0 Plan A spike, joining the returned grid to five host-selected points happened after the table-function scan: it produced 150 candidate rows and reported `scan_complete=false` on a 5×5×3×2 fixture. Could `read_om_series` accept a point-index selection (or expose a composable relation/filter contract) that applies before reading value chunks? Please document whether selected points can prune the physical scan, and expose scan metrics that distinguish complete from incomplete byte counts.

Evidence: [`EXIT-2`](evidence/EXIT-2-spark-aarch64-20261117.md), especially the Plan A feasibility and metrics sections. Plan B is the selected M0 approach for eligible regular spherical grids; this request is for a more selective future Plan A path.

## 2. Specify nearest-region representative semantics or add a point-based nearest input

Plan B uses small buffer polygons with `read_om_regions(method := 'nearest')` and `point_on_surface_v1` representatives. The M0 independent oracle covered grid centers, off-grid stations, ties, longitude wrapping, nulls, and out-of-domain stations, and both plans matched it on the retained fixture. Please document precisely how region representatives, ties, CRS, longitude normalization, and distance are defined. If polygon representatives are not intended to mean nearest grid center, consider a point-coordinate input that returns the selected grid cell directly.

Evidence and limits: [`EXIT-2`](evidence/EXIT-2-spark-aarch64-20261117.md). Its 5×5×3×2 fixture is diagnostic and does not establish large-grid performance.

## 3. Expose an explicit version function

The M0 bootstrap reads `extension_version` from `duckdb_extensions()` and compares it exactly with the artifact lock. A small stable SQL function such as `duckomo_version()` would provide a direct compatibility probe and clearer diagnostics. Please define whether it reports the package release, source revision, or both, and keep the value independent of whether optional functions are registered.

## 4. Publish a stable error taxonomy

The caller needs to distinguish invalid/unsupported OM data, malformed metadata, missing objects, unavailable remote access, unsupported grid geometry, cancellation, and internal extension errors. Please document stable error codes/classes and which failures are retryable. Human-readable message text should remain separate from the code so the worker can map failures without parsing strings.

## 5. Refresh community-extension channel documentation

The public community-extension documentation and the tested v0.4.0 community artifact did not agree in this environment: the locked Linux arm64 artifact failed both tested local `LOAD` and controlled local `INSTALL` under signature enforcement, while the separate GitHub-release unsigned artifact loaded and passed its locked golden check. Please clarify the supported local-install procedure and artifact/channel relationship for DuckDB v2.0.0, and update `docs/community-extensions.md` where its claims or examples are stale. This request reports the measured scope only; it does not infer why signature validation failed or claim that the release artifacts are otherwise identical.

Evidence: [`SPIKE-3 signed-local result`](evidence/SPIKE-3-signed-local-spark-aarch64-20261116.md), [`EXIT-1`](evidence/EXIT-1-spark-aarch64-n-a-20261116.md), and [`channel decision`](decisions/channel.md).
