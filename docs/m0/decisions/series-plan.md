# Station-series plan decision

**Decision date:** 2026-11-17
**Decision:** Use Plan B (`read_om_regions(method := 'nearest')`) for M0 station-series sampling on supported regular spherical longitude/latitude grids.

## Evidence

The same 9 stations and 3-hourly window were run through both plans using the locked Duckomo v0.4.0 GitHub release artifact on Linux aarch64. Both emitted the stable `station_series` columns and matched the independent official-reader value oracle, nearest-cell reference, UTC timestamps, null positions, CRS, and source identity for all 27 rows. The detailed record is [EXIT-2 evidence](../evidence/EXIT-2-spark-aarch64-20261117.md); raw per-plan reports and the field-by-field comparison JSON are linked there.

Plan A is functionally correct. It uses `om_grid_info`, host-side spherical nearest-cell selection, and `read_om_series`, then joins on the source row-major point index. The locked API does not accept selected point indices, so the table-function scan remains full-grid: this run reported 150 candidates for a 5×5×3×2 fixture despite five selected points. `duckomo_last_scan_metrics` marked Plan A's scan incomplete, so its reported zero value bytes cannot support a physical-byte comparison.

Plan B constructs stable region IDs and small buffer polygons, preinstalls and loads spatial under the R4 mitigation, enumerates chunk indices from the time layout, reads nearest samples, appends chunk results, then de-duplicates and sorts by the stable key. Its scan metrics were complete in this run and reported 27 candidates and 206 value bytes. On the tiny fixture Plan B took 120 ms versus Plan A's 49 ms; this one-run overhead does not establish production performance.

## Rationale and scope

Plan B is selected because it expresses the required sampling operation at the source and reports a complete scan for this fixture. The choice is based on query capability and measurable scan observability, not on a claimed speed win. Plan A remains a semantically correct full-grid fallback for eligible regular spherical grids. Lack of point slicing does not make Plan A generally infeasible; it means it cannot meet a selective-read goal with the current API.

This decision covers M0 nearest station sampling on regular spherical lon/lat grids with the R4 spatial installation path. It does not establish production-grid accuracy, scale behavior, multi-chunk overlap behavior, or support for other grid classes. Multi-chunk runtime overlap was not present in the one-chunk fixture; schema tests cover exact duplicate removal and rejection of conflicting duplicate values. Re-measure with representative production fixtures before making a scale or latency claim.
