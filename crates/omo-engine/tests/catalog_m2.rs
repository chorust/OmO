//! M2 catalog `spatial_layout` extension (contracts/catalog-m2.md).

use std::path::PathBuf;

use omo_engine::catalog::{Catalog, CatalogError, spatial_object_url};
use omo_protocol::series::CatalogEntry;
use serde_json::{Value, json};

const OBJECT: &str =
    "{YYYY}/{MM}/{DD}/{HH}00Z/{valid_YYYY}-{valid_MM}-{valid_DD}T{valid_HH}{valid_mm}.om";

#[test]
fn omitted_spatial_layout_is_accepted_and_has_no_spatial_url() {
    let catalog = Catalog::parse(&catalog_with(None).to_string()).unwrap();
    let entry = catalog.default_entry();
    assert!(entry.spatial_layout.is_none());
    assert_eq!(
        spatial_object_url(entry, "2026-11-25T00:00:00Z", "2026-11-25T06:00:00Z"),
        None
    );
}

#[test]
fn valid_data_spatial_layout_builds_exact_urls() {
    let spatial = json!({
        "kind": "data_spatial",
        "base": "s3://openmeteo/data_spatial/ecmwf_ifs/",
        "object": OBJECT,
    });
    let catalog = Catalog::parse(&catalog_with(Some(spatial)).to_string()).unwrap();
    let entry = catalog.default_entry();
    assert_eq!(
        spatial_object_url(entry, "2026-11-25T00:00:00Z", "2026-11-25T06:00:00Z").unwrap(),
        "s3://openmeteo/data_spatial/ecmwf_ifs/2026/11/25/0000Z/2026-11-25T0600.om"
    );
    // valid time on a different day than the run, minutes preserved.
    assert_eq!(
        spatial_object_url(entry, "2026-11-25T18:00:00Z", "2026-11-26T03:30:00Z").unwrap(),
        "s3://openmeteo/data_spatial/ecmwf_ifs/2026/11/25/1800Z/2026-11-26T0330.om"
    );
}

#[test]
fn rejects_unknown_kind_missing_tokens_and_bad_base() {
    let bad_kind = json!({"kind": "data_tiles", "base": "s3://x/y/", "object": OBJECT});
    assert!(matches!(
        Catalog::parse(&catalog_with(Some(bad_kind)).to_string()).unwrap_err(),
        CatalogError::UnknownLayout { .. }
    ));

    for token in [
        "{YYYY}",
        "{MM}",
        "{DD}",
        "{HH}",
        "{valid_YYYY}",
        "{valid_MM}",
        "{valid_DD}",
        "{valid_HH}",
        "{valid_mm}",
    ] {
        let object = OBJECT.replace(token, "X");
        let spatial = json!({"kind": "data_spatial", "base": "s3://x/y/", "object": object});
        let error = Catalog::parse(&catalog_with(Some(spatial)).to_string()).unwrap_err();
        assert!(
            matches!(error, CatalogError::InvalidSpatialLayout { .. }),
            "token {token}: {error:?}"
        );
    }

    for base in ["", "ftp://x/y/", "s3://x/y"] {
        let spatial = json!({"kind": "data_spatial", "base": base, "object": OBJECT});
        let error = Catalog::parse(&catalog_with(Some(spatial)).to_string()).unwrap_err();
        assert!(
            matches!(error, CatalogError::InvalidSpatialLayout { .. }),
            "base {base:?}: {error:?}"
        );
    }
}

#[test]
fn shipped_catalog_declares_data_spatial_for_the_default_entry() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog/products.json");
    let catalog = Catalog::load(&path).unwrap();
    assert_eq!(catalog.version, "2026-11-25.1");
    let entry = catalog.default_entry();
    let spatial = entry.spatial_layout.as_ref().expect("spatial_layout");
    assert_eq!(spatial.kind, "data_spatial");
    assert_eq!(spatial.base, "s3://openmeteo/data_spatial/ecmwf_ifs/");
    assert_eq!(
        spatial_object_url(entry, "2026-11-25T12:00:00Z", "2026-11-25T15:00:00Z").unwrap(),
        "s3://openmeteo/data_spatial/ecmwf_ifs/2026/11/25/1200Z/2026-11-25T1500.om"
    );
}

#[test]
fn entry_roundtrips_without_emitting_absent_spatial_layout() {
    let entry: CatalogEntry = serde_json::from_value(valid_entry(None)).unwrap();
    let value = serde_json::to_value(&entry).unwrap();
    assert!(value.get("spatial_layout").is_none());
}

fn catalog_with(spatial: Option<Value>) -> Value {
    json!({
        "schema": "omo-catalog/1",
        "version": "2026-11-25.1",
        "entries": [valid_entry(spatial)]
    })
}

fn valid_entry(spatial: Option<Value>) -> Value {
    let mut entry = json!({
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
        "attribution": []
    });
    if let Some(spatial) = spatial {
        entry["spatial_layout"] = spatial;
    }
    entry
}
