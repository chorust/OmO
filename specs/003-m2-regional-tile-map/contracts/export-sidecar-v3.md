# Contract: Map Export Sidecar `omo-export-sidecar/3`

Extends the [M0 export protocol](../../001-m0-skeleton-engine/contracts/export-protocol.md) and [M1 `omo-export-sidecar/2`](../../002-m1-station-time-series/contracts/export-sidecar-v2.md) for composite PNG map exports (`<target>.png` + `<target>.png.omo.json`).

## Commit & Startup Recovery Invariants (`FR-021`, `SC-007`)

1. **Unchanged atomic commit**: Staging directory `.<name>.png.omo-tmp-<id>/` with `.omo-owner` (`omo-export-staging/1\n`), `fsync`ed `data` (composite PNG) and `sidecar` (JSON), `.omo-old` backup on confirmed overwrite, `rename(data, target)` (`step2`), and `rename(sidecar, target.omo.json)` + parent `fsync` as the **single commit point** (`step3`), rejecting cross-filesystem `EXDEV`.
2. **Immutable export snapshot**: Submitting export snapshots `(revision, viewport, valid_time_utc, color_scale, layers)` and requires the forecast field (if visible) and all visible layers to be rendered synchronously by the worker onto the target `(width, height)` canvas—never screenshotting a partially loaded UI tile mosaic.
3. **`export_scan` compatibility**: `omo-protocol::export::is_supported_sidecar_schema` accepts `"omo-export-sidecar/1"`, `"omo-export-sidecar/2"`, and `"omo-export-sidecar/3"`. Because `/data/sha256` remains at the identical JSON pointer, `export_scan::is_saved` and `scan_export_dir` work unchanged.

## JSON Schema (`<target>.png.omo.json`)

```json
{
  "schema": "omo-export-sidecar/3",
  "exported_at_utc": "2026-11-25T10:30:00Z",
  "data": {
    "file": "east-asia-t2m.png",
    "format": "png",
    "bytes": 412890,
    "sha256": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
    "width": 1280,
    "height": 800
  },
  "engine": {
    "duckdb_version": "v2.0.0",
    "duckomo_version": "0.4.0",
    "channel": "community-signed"
  },
  "stations": {
    "sha256": "508f99b6bc8ce97672df8e85c8b2f5e0788a4b6578e62c...",
    "count": 100
  },
  "catalog": {
    "entry_id": "ecmwf_ifs.temperature_2m",
    "version": "2026-11-25.1"
  },
  "forecast": {
    "reference_time_utc": "2026-11-25T00:00:00Z",
    "cycle": "00",
    "valid_time_utc": "2026-11-25T06:00:00Z",
    "source_layout": "data_spatial",
    "source_object": "s3://openmeteo/data_spatial/ecmwf_ifs/2026/11/25/0000Z/2026-11-25T0600.om",
    "object_version": "\"c6c8685c9c9a7f8745d70c60bd3277fb-16\""
  },
  "viewport": {
    "crs": "EPSG:3857",
    "center_lat": 35.0,
    "center_lon": 120.0,
    "zoom": 5,
    "bbox_wgs84": [95.625, 16.636, 144.375, 50.736]
  },
  "color_scale": {
    "palette": "turbo",
    "min": -12.45,
    "max": 28.1,
    "unit": "°C",
    "locked": false
  },
  "layers": [
    {
      "id": "coastlines",
      "kind": "builtin_coastlines",
      "opacity": 1.0,
      "value_provenance": "display_only"
    },
    {
      "id": "forecast",
      "kind": "forecast_field",
      "opacity": 0.78,
      "value_provenance": "native_numeric"
    },
    {
      "id": "stations",
      "kind": "station_overlay",
      "opacity": 1.0,
      "value_provenance": "display_only"
    }
  ],
  "attribution": [
    "Weather data by Open-Meteo.com (CC BY 4.0, https://open-meteo.com/en/licence)",
    "Copyright © 2026 European Centre for Medium-Range Weather Forecasts (ECMWF). Source: www.ecmwf.int. Licence: CC BY 4.0. ECMWF does not accept any liability whatsoever for any error or omission in the data, their availability, or for any loss or damage arising from their use.",
    "Basemap boundaries: Natural Earth 1:50m (Public Domain)",
    "Modified: regional tile rasterisation and station overlay by omobench"
  ]
}
```

- When `stations` are absent (`0` stations imported), `"stations"` is `null`.
- When the `forecast` layer is hidden during a basemap-only export, `"forecast"` and `"color_scale"` are `null`.
- Each active `local_pmtiles` entry in `"layers"` records `"path"`, `"sha256"` (or `"size_bytes"` when `> 256 MiB`), `"tile_kind"`, and `"attribution"`.
