//! Catalog `omo-catalog/1` validation (contracts/catalog.md).

use std::fs;

use omo_engine::catalog::{Catalog, CatalogError};
use omo_protocol::series::CatalogEntry;
use serde_json::{Value, json};
use tempfile::TempDir;

#[test]
fn valid_catalog_loads_and_selects_the_single_default() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("products.json");
    fs::write(
        &path,
        serde_json::to_string_pretty(&valid_catalog()).unwrap(),
    )
    .unwrap();
    let catalog = Catalog::load(&path).unwrap();
    assert_eq!(catalog.schema, "omo-catalog/1");
    assert_eq!(catalog.version, "2026-11-23.1");
    assert_eq!(catalog.default_entry().id, "ecmwf_ifs.temperature_2m");
    let entry: CatalogEntry = serde_json::from_value(valid_entry()).unwrap();
    assert_eq!(
        omo_engine::catalog::object_url(&entry, "2026-11-23T00:00:00Z"),
        "s3://openmeteo/data_run/ecmwf_ifs/2026/11/23/0000Z/temperature_2m.om"
    );
    assert_eq!(
        omo_engine::catalog::meta_url(&entry, "2026-11-22T18:00:00Z"),
        "s3://openmeteo/data_run/ecmwf_ifs/2026/11/22/1800Z/meta.json"
    );
}

#[test]
fn rejects_unknown_schema_major_versions() {
    for schema in ["omo-catalog/0", "omo-catalog/2"] {
        let mut value = valid_catalog();
        value["schema"] = json!(schema);
        let error = Catalog::parse(&value.to_string()).unwrap_err();
        assert!(
            matches!(error, CatalogError::UnsupportedSchema { .. }),
            "{error:?}"
        );
    }
}

#[test]
fn rejects_default_count_unknown_kinds_window_and_missing_version() {
    let mut zero = valid_catalog();
    zero["entries"][0]["is_default"] = json!(false);
    assert!(matches!(
        Catalog::parse(&zero.to_string()).unwrap_err(),
        CatalogError::DefaultCount { count: 0 }
    ));

    let mut two = valid_catalog();
    let mut extra = valid_entry();
    extra["id"] = json!("other");
    extra["is_default"] = json!(true);
    two["entries"].as_array_mut().unwrap().push(extra);
    assert!(matches!(
        Catalog::parse(&two.to_string()).unwrap_err(),
        CatalogError::DefaultCount { count: 2 }
    ));

    let mut layout = valid_catalog();
    layout["entries"][0]["layout"]["kind"] = json!("spatial_tiles");
    assert!(matches!(
        Catalog::parse(&layout.to_string()).unwrap_err(),
        CatalogError::UnknownLayout { .. }
    ));

    let mut grid = valid_catalog();
    grid["entries"][0]["grid"]["kind"] = json!("healpix");
    assert!(matches!(
        Catalog::parse(&grid.to_string()).unwrap_err(),
        CatalogError::UnknownGrid { .. }
    ));

    let mut window = valid_catalog();
    window["entries"][0]["default_window_h"] = json!([0, 400]);
    assert!(matches!(
        Catalog::parse(&window.to_string()).unwrap_err(),
        CatalogError::WindowOutsideHorizons { start: 0, end: 400 }
    ));

    let mut missing = valid_catalog();
    missing.as_object_mut().unwrap().remove("version");
    assert_eq!(
        Catalog::parse(&missing.to_string()).unwrap_err(),
        CatalogError::MissingVersion
    );
}

fn valid_catalog() -> Value {
    json!({
        "schema": "omo-catalog/1",
        "version": "2026-11-23.1",
        "entries": [valid_entry()]
    })
}

fn valid_entry() -> Value {
    json!({
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
        "attribution": ["Weather data by Open-Meteo.com (CC BY 4.0, https://open-meteo.com/en/licence)"]
    })
}
