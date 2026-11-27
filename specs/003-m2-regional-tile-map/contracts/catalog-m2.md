# Contract: Catalog `omo-catalog/1` Spatial Layout Extension (M2)

Extends [M1 catalog contract](../../002-m1-station-time-series/contracts/catalog.md) without bumping the `omo-catalog/1` major schema, preserving 100% backward compatibility with M1 `SeriesRequest` payloads and tests.

## Schema (`catalog/products.json`)

```json
{
  "schema": "omo-catalog/1",
  "version": "2026-11-25.1",
  "entries": [
    {
      "id": "ecmwf_ifs.temperature_2m",
      "is_default": true,
      "product": {
        "provider": "ECMWF",
        "model": "IFS HRES",
        "open_meteo_domain": "ecmwf_ifs"
      },
      "variable": {
        "name": "temperature_2m",
        "unit": "°C",
        "label": "2 m air temperature"
      },
      "layout": {
        "kind": "data_run",
        "base": "s3://openmeteo/data_run/ecmwf_ifs/",
        "latest": "latest.json",
        "object": "{YYYY}/{MM}/{DD}/{HH}00Z/{variable}.om",
        "meta": "{YYYY}/{MM}/{DD}/{HH}00Z/meta.json"
      },
      "spatial_layout": {
        "kind": "data_spatial",
        "base": "s3://openmeteo/data_spatial/ecmwf_ifs/",
        "object": "{YYYY}/{MM}/{DD}/{HH}00Z/{valid_YYYY}-{valid_MM}-{valid_DD}T{valid_HH}{valid_mm}.om"
      },
      "grid": {
        "kind": "reduced_gaussian_o1280",
        "duckomo_domain": "ecmwf_ifs",
        "points": 6599680
      },
      "cycles": { "00": 360, "06": 144, "12": 360, "18": 144 },
      "default_window_h": [0, 168],
      "max_distance_m": 7200,
      "attribution": [
        "Weather data by Open-Meteo.com (CC BY 4.0, https://open-meteo.com/en/licence)",
        "Copyright © <run year> European Centre for Medium-Range Weather Forecasts (ECMWF). Source: www.ecmwf.int. Licence: CC BY 4.0. ECMWF does not accept any liability whatsoever for any error or omission in the data, their availability, or for any loss or damage arising from their use.",
        "Modified: nearest-point sampling and regional tile rasterisation by omobench"
      ]
    }
  ]
}
```

## Validation Rules (`CatalogDocument::parse`)

1. `spatial_layout` is optional (`#[serde(default)]`). When absent, map field loads go directly to `entry.layout` (`data_run`).
2. When `spatial_layout` is present:
   - `spatial_layout.kind` MUST equal `"data_spatial"`; any other value fails load with `CatalogValidationError::UnknownLayout`.
   - `spatial_layout.base` MUST be a non-empty `s3://` or `https://` URL ending with `/`.
   - `spatial_layout.object` MUST contain `{YYYY}`, `{MM}`, `{DD}`, `{HH}`, `{valid_YYYY}`, `{valid_MM}`, `{valid_DD}`, `{valid_HH}`, and `{valid_mm}`.
3. Run completeness resolution (`resolve_run` and `catalog_check`) continues to validate against `entry.layout` (`data_run` `latest.json` + `meta.json`) so partial early uploads are never treated as complete runs.
4. When reading a `data_spatial` object via `read_om`, the engine MUST pass `variable := '<entry.variable.name>'` explicitly so `duckomo` binds only the requested child variable rather than all variables in the hierarchical `.om` file.
