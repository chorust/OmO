# Contract: product catalog `omo-catalog/1`

File: `catalog/products.json`. It replaces the `omo-catalog/0` placeholder and is shipped with
the app. Field definitions are in [data-model › CatalogEntry](../data-model.md#catalogentry-omo-enginecatalog-schema-omo-catalog1).

```json
{
  "schema": "omo-catalog/1",
  "version": "2026-11-23.1",
  "entries": [
    {
      "id": "ecmwf_ifs.temperature_2m",
      "is_default": true,
      "product": {"provider": "ECMWF", "model": "IFS HRES", "open_meteo_domain": "ecmwf_ifs"},
      "variable": {"name": "temperature_2m", "unit": "°C", "label": "2 m air temperature"},
      "layout": {
        "kind": "data_run",
        "base": "s3://openmeteo/data_run/ecmwf_ifs/",
        "latest": "latest.json",
        "object": "{YYYY}/{MM}/{DD}/{HH}00Z/{variable}.om",
        "meta": "{YYYY}/{MM}/{DD}/{HH}00Z/meta.json"
      },
      "grid": {"kind": "reduced_gaussian_o1280", "duckomo_domain": "ecmwf_ifs", "points": 6599680},
      "cycles": {"00": 360, "06": 144, "12": 360, "18": 144},
      "default_window_h": [0, 168],
      "max_distance_m": 7200,
      "attribution": ["<see research R-08, verbatim>"]
    }
  ]
}
```

Rules:
- Loading MUST reject the following, without using the file partially:
  - an unknown `schema` major version;
  - zero `is_default` entries, or more than one;
  - an unknown `layout.kind` or `grid.kind`;
  - `default_window_h` outside the declared cycle horizons.
- Any field change bumps `version`. Projects and sidecars record `version`, and previews are
  keyed by it (FR-016).
- Readiness is computed at runtime (`catalog_check`) and is never written to this file.
