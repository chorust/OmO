//! Composite PNG map export (`T039`, `SC-007`): layer z-order and opacity, colorbar, station
//! markers, header/attribution text, the `omo-export-sidecar/3` manifest and the atomic commit.

use std::{collections::HashMap, fs, path::PathBuf, sync::Arc};

use omo_engine::{
    export::{ExportError, export_map_png},
    grid::reduced_gaussian::O1280,
    map::{
        CachedField, GridValues,
        compose_export::{self, BACKGROUND, ComposeError, Composed},
        pmtiles_layer::PmtilesLayer,
        raster::ColorLut,
    },
};
use omo_protocol::{
    export::sidecar_path,
    map::{
        ExportColorScale, ExportForecast, ExportLayer, ExportLayerKind, ExportStation,
        ExportViewport, FieldStats, MapBbox, MapExportRequest, SourceLayout,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/pmtiles")
        .join(name)
}

const ARCHIVE: &str = "sample-raster.pmtiles";

fn layer(id: &str, kind: ExportLayerKind, opacity: f32) -> ExportLayer {
    ExportLayer {
        id: id.to_owned(),
        kind,
        path: (kind == ExportLayerKind::LocalPmtiles)
            .then(|| fixture(ARCHIVE).to_string_lossy().into_owned()),
        opacity,
    }
}

fn request(layers: Vec<ExportLayer>) -> MapExportRequest {
    MapExportRequest {
        revision: 7,
        field_id: Some("map-1".into()),
        viewport: ExportViewport {
            center_lat: 20.0,
            center_lon: 50.0,
            zoom: 3,
            width: 640,
            height: 400,
        },
        stations: vec![
            ExportStation {
                id: "S1".into(),
                lat: 20.0,
                lon: 50.0,
                selected: true,
            },
            ExportStation {
                id: "S2".into(),
                lat: 25.0,
                lon: 62.0,
                selected: false,
            },
        ],
        stations_sha256: Some("508f99b6".into()),
        catalog_entry: "ecmwf_ifs.temperature_2m".into(),
        catalog_version: "2026-11-25.1".into(),
        forecast: Some(ExportForecast {
            reference_time_utc: "2026-11-25T00:00:00Z".into(),
            cycle: "00".into(),
            valid_time_utc: "2026-11-25T06:00:00Z".into(),
            source_layout: SourceLayout::DataSpatial,
            source_object:
                "s3://openmeteo/data_spatial/ecmwf_ifs/2026/11/25/0000Z/2026-11-25T0600.om".into(),
            object_version: Some("\"abc-16\"".into()),
        }),
        color_scale: Some(ExportColorScale {
            palette: "turbo".into(),
            min: -10.0,
            max: 30.0,
            unit: "°C".into(),
            locked: false,
        }),
        layers,
        attribution: vec![
            "Weather data by Open-Meteo.com (CC BY 4.0, https://open-meteo.com/en/licence)".into(),
            "Basemap boundaries: Natural Earth 1:50m (Public Domain)".into(),
        ],
        title: "ECMWF IFS 9 km · 2 m temperature".into(),
        subtitle: "run 2026-11-25T00:00:00Z · valid 2026-11-25T06:00:00Z".into(),
    }
}

fn uniform_field(value: f32) -> CachedField {
    CachedField {
        field_id: "map-1".into(),
        entry_id: "ecmwf_ifs.temperature_2m".into(),
        reference_time_utc: "2026-11-25T00:00:00Z".into(),
        valid_time_utc: "2026-11-25T06:00:00Z".into(),
        variable: "temperature_2m".into(),
        unit: "°C".into(),
        source_layout: SourceLayout::DataSpatial,
        source_object: "s3://x".into(),
        object_version: None,
        coverage: MapBbox {
            lat_min: -90.0,
            lat_max: 90.0,
            lon_spans: vec![[-180.0, 180.0]],
        },
        values: GridValues::O1280Dense(vec![value; O1280::points() as usize]),
        stats: FieldStats {
            valid_count: u64::from(O1280::points()),
            min: f64::from(value),
            max: f64::from(value),
            unit: "°C".into(),
        },
    }
}

fn archives(req: &MapExportRequest) -> HashMap<String, Arc<PmtilesLayer>> {
    req.layers
        .iter()
        .filter_map(|layer| {
            layer.path.as_ref().map(|path| {
                (
                    layer.id.clone(),
                    Arc::new(PmtilesLayer::open(std::path::Path::new(path)).unwrap()),
                )
            })
        })
        .collect()
}

fn compose(req: &MapExportRequest, field: Option<&CachedField>) -> Result<Composed, ComposeError> {
    compose_export::compose(req, field, &archives(req), &mut || Ok(()))
}

struct Image {
    width: usize,
    rgba: Vec<u8>,
}

impl Image {
    fn decode(png: &[u8]) -> Self {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buffer).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        buffer.truncate(info.buffer_size());
        Self {
            width: info.width as usize,
            rgba: buffer,
        }
    }

    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width + x as usize) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }
}

/// A map-local pixel away from the 2 px white border of every fixture tile.
fn interior_pixel(req: &MapExportRequest) -> (u32, u32) {
    let (ox, oy) = compose_export::map_origin(&req.viewport);
    for dv in 0..200u32 {
        for du in 0..200u32 {
            let (u, v) = (req.viewport.width / 2 + du, req.viewport.height / 2 + dv);
            let (tx, ty) = (
                (ox + i64::from(u)).rem_euclid(256),
                (oy + i64::from(v)).rem_euclid(256),
            );
            if (8..248).contains(&tx) && (8..248).contains(&ty) {
                return (u, v);
            }
        }
    }
    unreachable!()
}

fn forecast_color(value: f32) -> [u8; 3] {
    let c = ColorLut::new("turbo").unwrap().color(value, -10.0, 30.0);
    [c[0], c[1], c[2]]
}

fn near(a: [u8; 4], b: [u8; 3], tolerance: i32) -> bool {
    (0..3).all(|i| (i32::from(a[i]) - i32::from(b[i])).abs() <= tolerance)
}

#[test]
fn an_opaque_forecast_layer_replaces_the_background_and_has_the_requested_size() {
    let req = request(vec![layer("forecast", ExportLayerKind::ForecastField, 1.0)]);
    let composed = compose(&req, Some(&uniform_field(10.0))).unwrap();
    let image = Image::decode(&composed.png);
    assert_eq!(
        (
            image.width as u32,
            (image.rgba.len() / image.width / 4) as u32
        ),
        (composed.layout.width, composed.layout.height)
    );
    assert_eq!(composed.layout.map.width, 640);
    assert_eq!(composed.layout.map.height, 400);
    let (u, v) = interior_pixel(&req);
    let pixel = image.at(composed.layout.map.x + u, composed.layout.map.y + v);
    assert_eq!(pixel[3], 255);
    assert!(near(pixel, forecast_color(10.0), 1), "{pixel:?}");
}

#[test]
fn opacity_blends_the_layer_with_what_is_below_it() {
    let req = request(vec![layer("forecast", ExportLayerKind::ForecastField, 0.5)]);
    let composed = compose(&req, Some(&uniform_field(10.0))).unwrap();
    let image = Image::decode(&composed.png);
    let (u, v) = interior_pixel(&req);
    let pixel = image.at(composed.layout.map.x + u, composed.layout.map.y + v);
    let c = forecast_color(10.0);
    let expected = [0, 1, 2].map(|i| ((u16::from(c[i]) + u16::from(BACKGROUND[i])) / 2) as u8);
    assert!(near(pixel, expected, 2), "{pixel:?} vs {expected:?}");
}

#[test]
fn layers_are_composited_in_stack_order() {
    let forecast_on_top = request(vec![
        layer("archive", ExportLayerKind::LocalPmtiles, 1.0),
        layer("forecast", ExportLayerKind::ForecastField, 1.0),
    ]);
    let archive_on_top = request(vec![
        layer("forecast", ExportLayerKind::ForecastField, 1.0),
        layer("archive", ExportLayerKind::LocalPmtiles, 1.0),
    ]);
    let field = uniform_field(10.0);
    for (req, hides_archive) in [(&forecast_on_top, true), (&archive_on_top, false)] {
        let composed = compose(req, Some(&field)).unwrap();
        let image = Image::decode(&composed.png);
        let (u, v) = interior_pixel(req);
        let pixel = image.at(composed.layout.map.x + u, composed.layout.map.y + v);
        assert_eq!(
            near(pixel, forecast_color(10.0), 1),
            hides_archive,
            "{pixel:?}"
        );
    }
}

#[test]
fn a_hidden_layer_is_simply_absent_from_the_request() {
    // Only the archive: the pixel is the archive over the background, never the forecast.
    let req = request(vec![layer("archive", ExportLayerKind::LocalPmtiles, 1.0)]);
    let composed = compose(&req, None).unwrap();
    let image = Image::decode(&composed.png);
    let (u, v) = interior_pixel(&req);
    let pixel = image.at(composed.layout.map.x + u, composed.layout.map.y + v);
    assert!(!near(pixel, forecast_color(10.0), 8));
    assert!(!near(pixel, BACKGROUND, 0));
}

#[test]
fn station_markers_use_the_selected_and_plain_colors_at_their_projected_position() {
    let mut req = request(vec![layer(
        "stations",
        ExportLayerKind::StationOverlay,
        1.0,
    )]);
    req.forecast = None;
    req.color_scale = None;
    let composed = compose(&req, None).unwrap();
    let image = Image::decode(&composed.png);
    for (station, expected) in [
        (&req.stations[0], [0xbd, 0x79, 0x50]),
        (&req.stations[1], [255, 255, 255]),
    ] {
        let (px, py) = compose_export::project(&req.viewport, station.lon, station.lat);
        let pixel = image.at(
            composed.layout.map.x + px.round() as u32,
            composed.layout.map.y + py.round() as u32,
        );
        assert!(near(pixel, expected, 3), "{}: {pixel:?}", station.id);
    }
}

#[test]
fn the_colorbar_runs_from_the_low_to_the_high_end_of_the_palette() {
    let req = request(vec![layer("forecast", ExportLayerKind::ForecastField, 1.0)]);
    let composed = compose(&req, Some(&uniform_field(10.0))).unwrap();
    let image = Image::decode(&composed.png);
    let bar = composed
        .layout
        .colorbar
        .expect("a visible forecast has a colorbar");
    let y = bar.y + bar.height / 2;
    let lut = ColorLut::new("turbo").unwrap();
    let (low, high) = (lut.color(-10.0, -10.0, 30.0), lut.color(30.0, -10.0, 30.0));
    assert!(near(image.at(bar.x, y), [low[0], low[1], low[2]], 2));
    assert!(near(
        image.at(bar.x + bar.width - 1, y),
        [high[0], high[1], high[2]],
        2
    ));
    // The bar lives in the header band, above the map.
    assert!(bar.y + bar.height <= composed.layout.map.y);
}

#[test]
fn without_a_forecast_layer_there_is_no_colorbar() {
    let mut req = request(vec![layer("archive", ExportLayerKind::LocalPmtiles, 1.0)]);
    req.forecast = None;
    req.color_scale = None;
    assert!(compose(&req, None).unwrap().layout.colorbar.is_none());
}

fn ink(image: &Image, rect: compose_export::Rect) -> usize {
    let mut count = 0;
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            let p = image.at(x, y);
            if p[..3] != compose_export::PANEL[..] {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn header_and_attribution_text_is_drawn_and_the_footer_grows_with_the_lines() {
    let req = request(vec![layer("forecast", ExportLayerKind::ForecastField, 1.0)]);
    let composed = compose(&req, Some(&uniform_field(10.0))).unwrap();
    let image = Image::decode(&composed.png);
    assert!(ink(&image, composed.layout.title) > 150, "title drawn");
    assert!(
        ink(&image, composed.layout.footer) > 300,
        "attribution drawn"
    );

    let mut longer = req.clone();
    longer
        .attribution
        .push("Copyright © 2026 ECMWF. ".repeat(30));
    let taller = compose(&longer, Some(&uniform_field(10.0))).unwrap();
    assert!(taller.layout.footer.height > composed.layout.footer.height);
    assert_eq!(taller.layout.map, composed.layout.map);
}

#[test]
fn composing_the_same_snapshot_twice_gives_identical_bytes() {
    let req = request(vec![
        layer("coastlines", ExportLayerKind::BuiltinCoastlines, 1.0),
        layer("forecast", ExportLayerKind::ForecastField, 0.78),
        layer("stations", ExportLayerKind::StationOverlay, 1.0),
    ]);
    let field = uniform_field(12.5);
    let a = compose(&req, Some(&field)).unwrap();
    let b = compose(&req, Some(&field)).unwrap();
    assert_eq!(a.png, b.png);
}

#[test]
fn coastlines_are_stroked_over_the_background() {
    let mut req = request(vec![layer(
        "coastlines",
        ExportLayerKind::BuiltinCoastlines,
        1.0,
    )]);
    req.forecast = None;
    req.color_scale = None;
    req.viewport = ExportViewport {
        center_lat: 35.0,
        center_lon: 12.0,
        zoom: 4,
        width: 640,
        height: 400,
    };
    let composed = compose(&req, None).unwrap();
    let image = Image::decode(&composed.png);
    let map = composed.layout.map;
    let mut stroked = 0;
    for y in 0..map.height {
        for x in 0..map.width {
            if image.at(map.x + x, map.y + y)[..3] != BACKGROUND[..] {
                stroked += 1;
            }
        }
    }
    assert!(
        stroked > 300,
        "the Mediterranean must show coast: {stroked}"
    );
}

#[test]
fn an_unrenderable_snapshot_fails_instead_of_producing_a_partial_png() {
    // Visible forecast whose field is not resident in the worker.
    let req = request(vec![layer("forecast", ExportLayerKind::ForecastField, 1.0)]);
    assert!(matches!(
        compose(&req, None),
        Err(ComposeError::FieldMissing(_))
    ));
    // A field of another valid time than the snapshot's.
    let mut other = uniform_field(1.0);
    other.valid_time_utc = "2026-11-25T09:00:00Z".into();
    assert!(matches!(
        compose(&req, Some(&other)),
        Err(ComposeError::FieldMismatch(_))
    ));
    // A mounted archive that cannot be opened.
    let req = request(vec![layer("archive", ExportLayerKind::LocalPmtiles, 1.0)]);
    let missing = compose_export::compose(&req, None, &HashMap::new(), &mut || Ok(()));
    assert!(matches!(missing, Err(ComposeError::Layer { .. })));
    // Cancellation aborts the render.
    let req = request(vec![layer("archive", ExportLayerKind::LocalPmtiles, 1.0)]);
    let cancelled = compose_export::compose(&req, None, &archives(&req), &mut || {
        Err(ComposeError::Cancelled)
    });
    assert!(matches!(cancelled, Err(ComposeError::Cancelled)));
}

fn sidecar_for(req: &MapExportRequest, field: Option<&CachedField>) -> (Composed, Value, String) {
    let composed = compose(req, field).unwrap();
    let sha = format!("{:x}", Sha256::digest(&composed.png));
    let engine = json!({"duckdb_version": "v2.0.0", "duckomo_version": "0.4.0", "channel": "community-signed"});
    let sidecar = compose_export::build_sidecar(
        req,
        &composed,
        &sha,
        "east-asia.png",
        engine,
        "2026-11-25T10:30:00Z",
    );
    (composed, sidecar, sha)
}

#[test]
fn the_sidecar_records_the_full_provenance_of_the_snapshot() {
    let req = request(vec![
        layer("coastlines", ExportLayerKind::BuiltinCoastlines, 1.0),
        layer("archive", ExportLayerKind::LocalPmtiles, 0.6),
        layer("forecast", ExportLayerKind::ForecastField, 0.78),
        layer("stations", ExportLayerKind::StationOverlay, 1.0),
    ]);
    let (composed, sidecar, sha) = sidecar_for(&req, Some(&uniform_field(10.0)));
    assert_eq!(sidecar["schema"], "omo-export-sidecar/3");
    assert_eq!(sidecar["exported_at_utc"], "2026-11-25T10:30:00Z");
    assert_eq!(sidecar["data"]["file"], "east-asia.png");
    assert_eq!(sidecar["data"]["format"], "png");
    assert_eq!(sidecar["data"]["sha256"], sha.as_str());
    assert_eq!(sidecar["data"]["bytes"], composed.png.len());
    assert_eq!(sidecar["data"]["width"], composed.layout.width);
    assert_eq!(sidecar["data"]["height"], composed.layout.height);
    assert_eq!(sidecar["engine"]["channel"], "community-signed");
    assert_eq!(sidecar["stations"]["count"], 2);
    assert_eq!(sidecar["stations"]["sha256"], "508f99b6");
    assert_eq!(sidecar["catalog"]["entry_id"], "ecmwf_ifs.temperature_2m");
    assert_eq!(sidecar["catalog"]["version"], "2026-11-25.1");
    assert_eq!(sidecar["forecast"]["cycle"], "00");
    assert_eq!(
        sidecar["forecast"]["valid_time_utc"],
        "2026-11-25T06:00:00Z"
    );
    assert_eq!(sidecar["forecast"]["source_layout"], "data_spatial");
    assert_eq!(sidecar["forecast"]["object_version"], "\"abc-16\"");
    assert_eq!(sidecar["viewport"]["crs"], "EPSG:3857");
    assert_eq!(sidecar["viewport"]["zoom"], 3);
    assert!(sidecar["viewport"].get("map_width").is_none());
    assert!(sidecar["viewport"].get("map_height").is_none());
    let bbox = sidecar["viewport"]["bbox_wgs84"].as_array().unwrap();
    assert_eq!(bbox.len(), 4);
    assert!(bbox[0].as_f64().unwrap() < 50.0 && bbox[2].as_f64().unwrap() > 50.0);
    assert!(bbox[1].as_f64().unwrap() < 20.0 && bbox[3].as_f64().unwrap() > 20.0);
    assert_eq!(sidecar["color_scale"]["palette"], "turbo");
    assert_eq!(sidecar["color_scale"]["min"], -10.0);
    assert_eq!(sidecar["color_scale"]["unit"], "°C");
    assert_eq!(sidecar["color_scale"]["locked"], false);
    let layers = sidecar["layers"].as_array().unwrap();
    assert_eq!(
        layers
            .iter()
            .map(|l| l["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["coastlines", "archive", "forecast", "stations"]
    );
    assert_eq!(layers[0]["value_provenance"], "display_only");
    assert_eq!(layers[2]["value_provenance"], "native_numeric");
    assert_eq!(layers[2]["opacity"], 0.78f32 as f64);
    let archive = &layers[1];
    assert_eq!(archive["kind"], "local_pmtiles");
    assert_eq!(archive["tile_kind"], "raster_png");
    assert_eq!(
        archive["attribution"],
        "OMO test fixture (synthetic, public domain)"
    );
    assert_eq!(archive["path"], fixture(ARCHIVE).to_string_lossy().as_ref());
    assert_eq!(archive["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(sidecar["attribution"], json!(req.attribution));
}

#[test]
fn sidecar_nulls_follow_what_the_export_actually_shows() {
    let mut req = request(vec![layer(
        "coastlines",
        ExportLayerKind::BuiltinCoastlines,
        1.0,
    )]);
    req.stations.clear();
    req.stations_sha256 = None;
    req.forecast = None;
    req.color_scale = None;
    let (_, sidecar, _) = sidecar_for(&req, None);
    assert!(sidecar["stations"].is_null());
    assert!(sidecar["forecast"].is_null());
    assert!(sidecar["color_scale"].is_null());
}

#[test]
fn export_publishes_png_and_sidecar_atomically_and_the_hash_verifies() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("map.png");
    let req = request(vec![layer("forecast", ExportLayerKind::ForecastField, 1.0)]);
    let composed = compose(&req, Some(&uniform_field(10.0))).unwrap();
    let mut stages = Vec::new();
    let hash = export_map_png(
        &target,
        &composed.png,
        |sha| {
            compose_export::build_sidecar(
                &req,
                &composed,
                sha,
                "map.png",
                json!({}),
                "2026-11-25T10:30:00Z",
            )
        },
        false,
        |stage| stages.push(stage),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(stages, ["writing", "committing"]);
    assert_eq!(fs::read(&target).unwrap(), composed.png);
    assert_eq!(hash, format!("{:x}", Sha256::digest(&composed.png)));
    let sidecar: Value = serde_json::from_slice(&fs::read(sidecar_path(&target)).unwrap()).unwrap();
    assert_eq!(sidecar.pointer("/data/sha256").unwrap(), hash.as_str());
    // No staging directory is left behind.
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers.len(), 2, "{leftovers:?}");
}

#[test]
fn overwriting_needs_confirmation_and_keeps_the_old_pair_when_refused() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("map.png");
    fs::write(&target, b"old png").unwrap();
    let result = export_map_png(
        &target,
        b"new png",
        |_| json!({}),
        false,
        |_| {},
        |_| Ok(()),
    );
    assert!(matches!(
        result,
        Err(ExportError::OverwriteConfirmationRequired)
    ));
    assert_eq!(fs::read(&target).unwrap(), b"old png");
    export_map_png(&target, b"new png", |_| json!({}), true, |_| {}, |_| Ok(())).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"new png");
}

/// `OMO_DUMP_EXPORT=<dir>` writes a sample composite for visual inspection.
#[test]
fn dump_a_sample_composite_when_asked() {
    let Some(dir) = std::env::var_os("OMO_DUMP_EXPORT") else {
        return;
    };
    let mut req = request(vec![
        layer("coastlines", ExportLayerKind::BuiltinCoastlines, 1.0),
        layer("archive", ExportLayerKind::LocalPmtiles, 0.5),
        layer("forecast", ExportLayerKind::ForecastField, 0.78),
        layer("stations", ExportLayerKind::StationOverlay, 1.0),
    ]);
    req.viewport = ExportViewport {
        center_lat: 35.0,
        center_lon: 12.0,
        zoom: 4,
        width: 900,
        height: 560,
    };
    req.attribution = vec![
        "Weather data by Open-Meteo.com (CC BY 4.0, https://open-meteo.com/en/licence)".into(),
        "Copyright © 2026 European Centre for Medium-Range Weather Forecasts (ECMWF). Source: www.ecmwf.int. Licence: CC BY 4.0. ECMWF does not accept any liability whatsoever for any error or omission in the data, their availability, or for any loss or damage arising from their use.".into(),
        "Basemap boundaries: Natural Earth 1:50m (Public Domain)".into(),
        "Modified: regional tile rasterisation and station overlay by omobench".into(),
    ];
    let mut field = uniform_field(0.0);
    let values = field.values.as_mut_slice();
    for (i, v) in values.iter_mut().enumerate() {
        *v = ((i % 7919) as f32 / 7919.0) * 40.0 - 10.0;
    }
    let composed = compose(&req, Some(&field)).unwrap();
    fs::write(
        PathBuf::from(dir).join("sample-composite.png"),
        composed.png,
    )
    .unwrap();
}
