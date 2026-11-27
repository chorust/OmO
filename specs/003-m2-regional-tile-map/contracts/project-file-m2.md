# Contract: Project File `omo-project/1` Map Extension (`.omoproj`)

Extends [M1 project file contract](../../002-m1-station-time-series/contracts/project-file.md) with backward-compatible M2 map persistence (`FR-023`, `SC-008`).

## Compatibility & Version Gate Rules

1. **Extension & schema**: Remains `.omoproj` with `"schema": "omo-project/1"`.
2. **Schema major pre-check (`open_from_path`)**: Before deserializing `ProjectDoc`, inspect the top-level `"schema"` string; if it is missing or its major version is not `1` (e.g. `"omo-project/2"`), immediately return `ProjectFileError::UnsupportedSchema` without failing earlier on field-type changes.
3. **M1 backward compatibility**: `ProjectDoc.map` carries `#[serde(default)]`. Opening any M1 `.omoproj` file (which lacks `"map"`) initializes `MapViewState::default()` (`viewport: None` → auto-fit stations on open, `valid_time_lead_h: 0`, `palette: "turbo"`, default 3-layer stack).
4. **Canonical deterministic serialization (`SC-008`)**:
   - All floating-point fields in `MapViewState` (`center_lat`, `center_lon`, `locked_min`, `locked_max`, `opacity`) normalize `-0.0` to `0.0` and reject `NaN`/`±∞`.
   - `layers` is stored as an ordered `Vec<MapLayerConfig>` (never an unordered `HashMap`).
   - Saving and reopening on `linux_arm64`, `linux_amd64`, and `osx_arm64` produces byte-identical UTF-8 JSON (`serde_json` `float_roundtrip`).
5. **Drafts & dirty tracking**: `ProjectController::{fingerprint, snapshot_doc, apply_doc}` includes `ProjectDoc.map` so panning/locking viewport, changing valid time, editing color scales, or modifying the layer stack marks the document edited (`●`) and triggers the 2 s debounce / 30 s periodic draft stage to `<app_data>/drafts/<doc_id>.json`.

## JSON Shape (`map` addition to `omo-project/1`)

```json
{
  "schema": "omo-project/1",
  "doc_id": "6f1c...",
  "stations": { "...": "unchanged from M1" },
  "catalog_selection": {
    "entry_id": "ecmwf_ifs.temperature_2m",
    "catalog_version": "2026-11-25.1"
  },
  "window": { "start_h": 0, "end_h": 168 },
  "max_distance_m": 7200,
  "sql_tabs": [{ "title": "Query 1", "sql": "SELECT * FROM station_series LIMIT 20;" }],
  "view": {
    "selected_station_ids": ["S001"],
    "chart": { "show_legend": true, "y_auto": true }
  },
  "map": {
    "viewport": {
      "center_lat": 34.85,
      "center_lon": 118.4,
      "zoom": 5
    },
    "valid_time_lead_h": 6,
    "color_scale": {
      "palette": "turbo",
      "locked_min": null,
      "locked_max": null
    },
    "layers": [
      { "id": "coastlines", "kind": { "type": "builtin_coastlines" }, "visible": true, "opacity": 1.0 },
      { "id": "pmtiles-1", "kind": { "type": "local_pmtiles", "path": "/data/basemap.pmtiles" }, "visible": true, "opacity": 1.0 },
      { "id": "forecast", "kind": { "type": "forecast_field" }, "visible": true, "opacity": 0.78 },
      { "id": "stations", "kind": { "type": "station_overlay" }, "visible": true, "opacity": 1.0 }
    ]
  }
}
```

**Not stored in `.omoproj`**: `CachedField` grids, rendered `.rgba` tile files, `.pmtiles` tile contents, readiness state, or worker generation.
