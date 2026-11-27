//! M2 map wire contracts (contracts/worker-protocol-m2.md, data-model.md §3, §5).

use omo_protocol::{
    export::{SIDECAR_SCHEMA_V2, SIDECAR_SCHEMA_V3, is_supported_sidecar_schema},
    map::{
        ColorScale, FieldStats, ForecastSampleState, InspectRequest, InspectResult, LayerError,
        LayerProbeRequest, LoadFieldRequest, LoadFieldResult, MAX_MAP_ZOOM, MIN_MAP_ZOOM, MapBbox,
        MapRunSelector, PALETTES, PmtilesLayerInfo, PmtilesTileKind, RenderLayerKind,
        RenderLayerRequest, RenderTilesRequest, RenderTilesResult, RenderedTile, SourceLayout,
        TileCoord, ValueProvenance, palette_stops,
    },
    messages::{failed_body, parse_fail_kind},
    state::FailKind,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) -> T {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

#[test]
fn value_provenance_uses_snake_case_wire_names() {
    assert_eq!(
        serde_json::to_value(ValueProvenance::NativeNumeric).unwrap(),
        json!("native_numeric")
    );
    assert_eq!(
        serde_json::to_value(ValueProvenance::PaletteDecoded).unwrap(),
        json!("palette_decoded")
    );
    assert_eq!(
        serde_json::to_value(ValueProvenance::DisplayOnly).unwrap(),
        json!("display_only")
    );
    assert_eq!(ValueProvenance::NativeNumeric.label(), "native numeric");
    assert_eq!(ValueProvenance::DisplayOnly.label(), "display only");
}

#[test]
fn tile_coord_validates_zoom_and_index_ranges() {
    assert_eq!((MIN_MAP_ZOOM, MAX_MAP_ZOOM), (1, 12));
    assert!(TileCoord { z: 1, x: 1, y: 1 }.is_valid());
    assert!(
        TileCoord {
            z: 12,
            x: 4095,
            y: 4095
        }
        .is_valid()
    );
    assert!(!TileCoord { z: 0, x: 0, y: 0 }.is_valid());
    assert!(!TileCoord { z: 13, x: 0, y: 0 }.is_valid());
    assert!(!TileCoord { z: 3, x: 8, y: 0 }.is_valid());
    assert!(!TileCoord { z: 3, x: 0, y: 8 }.is_valid());
}

#[test]
fn load_field_request_and_result_round_trip_with_contract_field_names() {
    let wire = json!({
        "field_id": "f1",
        "catalog_entry": serde_json::Value::Null,
    });
    // catalog_entry is required to be a full entry; a null entry must be rejected.
    assert!(serde_json::from_value::<LoadFieldRequest>(wire).is_err());

    let bbox = MapBbox {
        lat_min: 10.0,
        lat_max: 40.0,
        lon_spans: vec![[100.0, 140.0]],
    };
    let result = LoadFieldResult {
        field_id: "f1".into(),
        run: serde_json::from_value(json!({
            "reference_time_utc": "2026-11-25T00:00:00Z", "cycle": "00", "horizon_h": 360,
            "valid_times": ["2026-11-25T00:00:00Z"], "object_url": "s3://x", "meta_etag": null
        }))
        .unwrap(),
        valid_time_utc: "2026-11-25T00:00:00Z".into(),
        source_layout: SourceLayout::DataRun,
        fallback_notice: Some("data_spatial expired".into()),
        source_object: "s3://openmeteo/data_run/x.om".into(),
        object_version: Some("\"etag\"".into()),
        stats: FieldStats {
            valid_count: 10,
            min: -3.5,
            max: 21.25,
            unit: "°C".into(),
        },
        coverage_bbox: bbox,
        scan_metrics: json!({"read_requests": 3}),
        readout_path: Some("/tmp/g1/field_f1.f32bin".into()),
    };
    assert_eq!(round_trip(&result), result);
    let value = serde_json::to_value(&result).unwrap();
    assert_eq!(value["source_layout"], "data_run");
    assert_eq!(value["stats"]["unit"], "°C");
}

#[test]
fn render_tiles_and_inspect_bodies_round_trip() {
    let request = RenderTilesRequest {
        result_id: "r1".into(),
        field_id: Some("f1".into()),
        revision: 7,
        color_scale: ColorScale {
            palette: "turbo".into(),
            min: -10.0,
            max: 30.0,
        },
        layers: vec![
            RenderLayerRequest {
                layer_id: "forecast".into(),
                kind: RenderLayerKind::ForecastField,
                path: None,
            },
            RenderLayerRequest {
                layer_id: "pm".into(),
                kind: RenderLayerKind::LocalPmtiles,
                path: Some("/data/a.pmtiles".into()),
            },
        ],
        tiles: vec![TileCoord { z: 5, x: 26, y: 12 }],
    };
    assert_eq!(round_trip(&request), request);
    let wire = serde_json::to_value(&request).unwrap();
    assert_eq!(wire["layers"][0]["kind"], "forecast_field");
    assert_eq!(wire["layers"][1]["kind"], "local_pmtiles");

    let result = RenderTilesResult {
        result_id: "r1".into(),
        revision: 7,
        tiles: vec![RenderedTile {
            layer_id: "forecast".into(),
            z: 5,
            x: 26,
            y: 12,
            width: 256,
            height: 256,
            rgba_path: "/tmp/g1/tile_forecast_5_26_12.rgba".into(),
            empty: false,
        }],
        layer_errors: vec![LayerError {
            layer_id: "pm".into(),
            message: "corrupt".into(),
        }],
    };
    assert_eq!(round_trip(&result), result);
    assert!(
        serde_json::to_value(&result)
            .unwrap()
            .get("legend")
            .is_none()
    );

    let inspect = InspectRequest {
        field_id: "f1".into(),
        latitude: 31.2,
        longitude: 121.5,
        max_distance_m: 7200,
    };
    assert_eq!(round_trip(&inspect), inspect);
    let samples = [
        ForecastSampleState::Value {
            value: 12.5,
            unit: "°C".into(),
            grid_lat: 31.19,
            grid_lon: 121.48,
            distance_m: 1234.5,
            provenance: ValueProvenance::NativeNumeric,
            stale: false,
        },
        ForecastSampleState::NoData {
            grid_lat: 1.0,
            grid_lon: 2.0,
            distance_m: 3.0,
        },
        ForecastSampleState::OutsideCachedExtent,
        ForecastSampleState::LayerHidden,
        ForecastSampleState::NotLoaded,
    ];
    for sample in samples {
        let body = InspectResult {
            latitude: 31.2,
            longitude: 121.5,
            sample: sample.clone(),
        };
        assert_eq!(round_trip(&body), body);
    }
    let wire = serde_json::to_value(ForecastSampleState::OutsideCachedExtent).unwrap();
    assert_eq!(wire["state"], "outside_cached_extent");
}

#[test]
fn pmtiles_layer_probe_defaults_to_display_only() {
    let probe = LayerProbeRequest {
        layer_id: "pm".into(),
        kind: RenderLayerKind::LocalPmtiles,
        path: "/a.pmtiles".into(),
    };
    assert_eq!(round_trip(&probe), probe);
    let info = PmtilesLayerInfo {
        path: "/a.pmtiles".into(),
        file_size: 10,
        fingerprint_sha256: None,
        tile_kind: PmtilesTileKind::VectorMvt,
        min_zoom: 0,
        max_zoom: 3,
        bounds: [-180.0, -85.0, 180.0, 85.0],
        attribution: Some("x".into()),
        name: None,
        value_provenance: ValueProvenance::DisplayOnly,
    };
    assert_eq!(round_trip(&info), info);
    assert_eq!(
        serde_json::to_value(&info).unwrap()["tile_kind"],
        "vector_mvt"
    );
}

#[test]
fn sidecar_v3_is_supported_alongside_v1_and_v2() {
    assert_eq!(SIDECAR_SCHEMA_V3, "omo-export-sidecar/3");
    assert!(is_supported_sidecar_schema("omo-export-sidecar/1"));
    assert!(is_supported_sidecar_schema(SIDECAR_SCHEMA_V2));
    assert!(is_supported_sidecar_schema(SIDECAR_SCHEMA_V3));
    assert!(!is_supported_sidecar_schema("omo-export-sidecar/4"));
}

#[test]
fn new_fail_kinds_round_trip_through_failed_bodies() {
    for kind in [FailKind::ValidTimeUnavailable, FailKind::InvalidLayer] {
        let body = failed_body(4, &kind, "message");
        assert_eq!(parse_fail_kind(&body), Some(kind.clone()));
    }
    assert_eq!(
        serde_json::to_value(FailKind::ValidTimeUnavailable).unwrap(),
        json!({"kind": "valid_time_unavailable"})
    );
    assert_eq!(
        serde_json::to_value(FailKind::InvalidLayer).unwrap(),
        json!({"kind": "invalid_layer"})
    );
    assert!(FailKind::Query("busy: worker already has an active request".into()).is_worker_busy());
    assert!(!FailKind::Query("syntax error".into()).is_worker_busy());
}

#[test]
fn map_run_selector_and_palettes_follow_wire_contracts() {
    assert_eq!(
        serde_json::to_value(MapRunSelector::LatestComplete).unwrap(),
        json!("latest_complete")
    );
    assert_eq!(
        round_trip(&MapRunSelector::Explicit("2026-11-25T00:00:00Z".into())),
        MapRunSelector::Explicit("2026-11-25T00:00:00Z".into())
    );
    assert!(serde_json::from_value::<MapRunSelector>(json!("not-a-timestamp")).is_err());
    assert!(serde_json::from_value::<MapRunSelector>(json!("2026-11-25T00:00:00+08:00")).is_err());
    for palette in PALETTES {
        assert_eq!(palette_stops(palette, 33).unwrap().len(), 33);
    }
    assert!(palette_stops("unknown", 33).is_none());
}
