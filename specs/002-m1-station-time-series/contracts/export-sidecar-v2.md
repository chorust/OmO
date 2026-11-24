# Contract: export sidecar `omo-export-sidecar/2`

This contract extends the [M0 export protocol](../../001-m0-skeleton-engine/contracts/export-protocol.md).
The commit protocol is unchanged: staging directory, `.omo-old` backup, data rename, then the
sidecar rename as the single commit point, with EXDEV as an error.

Fields added to `/1` (definitions in [data-model › ExportSidecar](../data-model.md#exportsidecar-omo-export-sidecar2)):

```json
{
  "schema": "omo-export-sidecar/2",
  "stations": {"sha256": "…", "count": 100},
  "catalog": {"entry_id": "ecmwf_ifs.temperature_2m", "version": "2026-11-23.1"},
  "run": {"reference_time_utc": "2026-11-23T00:00:00Z", "cycle": "00",
          "object_url": "s3://openmeteo/data_run/ecmwf_ifs/2026/11/23/0000Z/temperature_2m.om",
          "object_version": "\"<etag>\""},
  "window": {"requested": [0, 168], "effective": [0, 168], "truncated": false},
  "plan": "G",
  "query": null,
  "attribution": ["Weather data by Open-Meteo.com …", "Copyright © 2026 ECMWF …", "Modified: …"]
}
```

- Series exports: `query` is null.
- SQL exports: `query` holds the SQL text. `run`/`window` describe the series result the
  `station_series` view pointed at, or are null if no series view was attached.
- The startup scan and the saved predicate accept `/1` and `/2` alike.
