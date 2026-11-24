# Contract: omo-worker protocol v1 — M1 additions

This contract extends [M0 worker-protocol](../../001-m0-skeleton-engine/contracts/worker-protocol.md)
without changing the envelope, framing, cancellation, or result-file rules. `PROTOCOL_VERSION`
stays `1`. The new kinds are additive, and old hosts never send them.

## New requests (supervisor → worker)

| kind | body | result event body |
|---|---|---|
| `catalog_check` | `{entry: CatalogEntry}` | `{entry_id, readiness: Ready{run}\|NotReady{reason, detail}}` |
| `resolve_run` | `{entry: CatalogEntry, run: "latest_complete"\|{reference_time_utc}}` | `{run: ForecastRun}` |
| `series` | `SeriesRequest` (see [data-model](../data-model.md#seriesrequest-task-snapshot-omo-protocolseries)) | `{result_ref, parquet_path, matches_ref, plot_ref?, run, effective_window, plan, scan_metrics}` |
| `sql_session` | `{stations_ref, series_parquet?: path}` | `{views:["stations","station_series"?]}`. Re-creates the temp views; the host sends it after every `ready` and before SQL tasks |

`query` (M0) is unchanged. It runs against the views set up by `sql_session`.

`series_plan` (M0) is kept for spike and harness use only. The UI never sends it.

## Series processing order (worker)

1. Run the function probe (M0). Check `caps.remote`; if it is false, send
   `failed{CapabilityUnavailable(remote)}`.
2. `progress{stage:"planning"}`:
   - resolve the run (R-02);
   - compute the effective window;
   - map time index → valid_time from `meta.json`;
   - select the plan: `grid.kind = reduced_gaussian_o1280` → Plan G; an eligible regular grid
     → Plan B.
3. Plan G only: compute the nearest flat index per station. Stations beyond `max_distance_m`
   or with invalid coordinates are recorded in the match report.
4. `progress{stage:"reading"}`: run one `read_om(object_url, domain := 'ecmwf_ifs')` with OR
   lat/lon boxes. Check the returned coordinates; a mismatch fails the request with
   `failed{GridMismatch}`.
5. `progress{stage:"writing"}`: write the Arrow file, the Parquet file, the matches file, and
   the plot artifact. Rows are sorted by `(station_id, valid_time)`.
6. Send `result`.

Interrupt is honoured between stages and inside the DuckDB query (M0 level 1).

## New FailKind values

| FailKind | Meaning |
|---|---|
| `NoCompleteRun` | no complete run within 8 cycles (48 h) |
| `RunIncomplete` | an explicitly requested run is not complete |
| `GridMismatch` | duckomo coordinates ≠ the Rust row table, or not exactly one point per selection |
| `VariableMissing` | the variable is not in the run's `meta.json` |
| `SourceChanged` | the object ETag changed during the request |

Existing kinds are reused: `CapabilityUnavailable(remote)`, `Cancelled(level)`,
`WorkerCrashed`.
