//! Composite map PNG (`FR-021`, `FR-022`, `contracts/export-sidecar-v3.md`).
//!
//! The export is rendered *synchronously from an immutable snapshot*: every visible layer is
//! drawn straight onto one canvas at the viewport's zoom — forecast tiles from the worker-resident
//! numeric field, archive tiles from the mounted `.pmtiles`, coastlines and station markers as
//! vectors — so it can never be a screenshot of a half-loaded tile mosaic. The layers are
//! composited bottom to top with their opacity over a flat background, below a header (title,
//! subtitle, colorbar) and above an attribution footer. Output is deterministic: the same
//! snapshot always yields the same bytes.

use std::{collections::HashMap, sync::Arc};

use omo_protocol::{
    coastlines,
    map::{ExportLayer, ExportLayerKind, ExportViewport, MapExportRequest, ValueProvenance},
    mercator::{
        MAX_LATITUDE, TILE_SIZE, lonlat_to_world_px, normalize_lon, world_px, world_px_to_lonlat,
    },
};
use serde_json::{Value, json};
use thiserror::Error;
use tiny_skia::{
    FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Rect as SkRect, Stroke,
    Transform,
};

use super::{
    CachedField,
    pmtiles_layer::PmtilesLayer,
    raster::{ColorLut, render_tile},
    text,
};

/// Flat map background under every layer.
pub const BACKGROUND: [u8; 3] = [0xdd, 0xe5, 0xe8];
/// Header and footer panel color.
pub const PANEL: [u8; 3] = [0xf7, 0xf6, 0xf2];
const INK: [u8; 3] = [0x24, 0x31, 0x2c];
const MUTED: [u8; 3] = [0x5d, 0x68, 0x63];

const HEADER_HEIGHT: u32 = 64;
const PAD: u32 = 12;
const FOOTER_PAD: u32 = 8;
const FOOTER_LINE: u32 = 14;
const FOOTER_TEXT_SIZE: f32 = 11.0;
const TITLE_SIZE: f32 = 16.0;
const SUBTITLE_SIZE: f32 = 12.0;
const LABEL_SIZE: f32 = 11.0;
const BAR_HEIGHT: u32 = 14;
const BAR_MAX_WIDTH: u32 = 260;

const COAST_COLOR: [u8; 3] = [0x34, 0x45, 0x4d];
const BORDER_COLOR: [u8; 3] = [0x8a, 0x8f, 0x87];
const SELECTED_STATION: [u8; 3] = [0xbd, 0x79, 0x50];
const PLAIN_STATION: [u8; 3] = [0xff, 0xff, 0xff];

#[derive(Debug, Error)]
pub enum ComposeError {
    #[error("invalid export snapshot: {0}")]
    Invalid(String),
    #[error("export cancelled")]
    Cancelled,
    #[error("forecast field `{0}` is not loaded in the worker")]
    FieldMissing(String),
    #[error("{0}")]
    FieldMismatch(String),
    #[error("layer `{layer_id}`: {message}")]
    Layer { layer_id: String, message: String },
    #[error("cannot encode the PNG: {0}")]
    Encode(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Where everything sits on the output image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    /// The text area of the header (left of the colorbar).
    pub title: Rect,
    pub colorbar: Option<Rect>,
    pub map: Rect,
    pub footer: Rect,
}

pub struct Composed {
    pub png: Vec<u8>,
    pub layout: Layout,
    /// `[west, south, east, north]`, longitudes limited to the world.
    pub bbox_wgs84: [f64; 4],
    /// The layer manifest recorded in the sidecar, bottom to top.
    pub layers: Vec<Value>,
}

fn shows_forecast(req: &MapExportRequest) -> bool {
    req.layers
        .iter()
        .any(|layer| layer.kind == ExportLayerKind::ForecastField)
}

/// Compute the image layout; the footer grows with the wrapped attribution.
pub fn layout(req: &MapExportRequest) -> Layout {
    let (width, map_height) = (req.viewport.width, req.viewport.height);
    let colorbar = shows_forecast(req).then(|| {
        let bar_width = BAR_MAX_WIDTH.min(width / 3).max(48);
        Rect {
            x: width.saturating_sub(bar_width + PAD),
            y: PAD,
            width: bar_width,
            height: BAR_HEIGHT,
        }
    });
    let title_width = colorbar.map_or(width, |bar| bar.x.saturating_sub(PAD));
    let lines = footer_lines(req);
    let footer_height = FOOTER_PAD * 2 + FOOTER_LINE * lines.len().max(1) as u32;
    Layout {
        width,
        height: HEADER_HEIGHT + map_height + footer_height,
        title: Rect {
            x: 0,
            y: 0,
            width: title_width,
            height: HEADER_HEIGHT,
        },
        colorbar,
        map: Rect {
            x: 0,
            y: HEADER_HEIGHT,
            width,
            height: map_height,
        },
        footer: Rect {
            x: 0,
            y: HEADER_HEIGHT + map_height,
            width,
            height: footer_height,
        },
    }
}

fn footer_lines(req: &MapExportRequest) -> Vec<String> {
    let max_width = req.viewport.width.saturating_sub(PAD * 2) as f32;
    req.attribution
        .iter()
        .flat_map(|line| text::wrap(line, FOOTER_TEXT_SIZE, max_width))
        .collect()
}

/// World pixel of the map's top-left corner, rounded so tiles land on whole pixels.
pub fn map_origin(vp: &ExportViewport) -> (i64, i64) {
    let (cx, cy) = lonlat_to_world_px(vp.zoom, normalize_lon(vp.center_lon), vp.center_lat);
    (
        (cx - f64::from(vp.width) / 2.0).round() as i64,
        (cy - f64::from(vp.height) / 2.0).round() as i64,
    )
}

/// Map-local pixel of `(lon, lat)` on the copy of the world nearest the viewport center.
pub fn project(vp: &ExportViewport, lon: f64, lat: f64) -> (f64, f64) {
    let size = world_px(vp.zoom) as f64;
    let (ox, oy) = map_origin(vp);
    let (cx, _) = lonlat_to_world_px(vp.zoom, normalize_lon(vp.center_lon), vp.center_lat);
    let (wx, wy) = lonlat_to_world_px(vp.zoom, normalize_lon(lon), lat);
    let mut dx = (wx - cx).rem_euclid(size);
    if dx >= size / 2.0 {
        dx -= size;
    }
    (cx + dx - ox as f64, wy - oy as f64)
}

/// The visible window `[min_lon, min_lat, max_lon, max_lat]` with unwrapped longitudes.
fn geo_window(vp: &ExportViewport) -> [f64; 4] {
    let size = world_px(vp.zoom) as f64;
    let (ox, oy) = map_origin(vp);
    let (ox, oy) = (ox as f64, oy as f64);
    let lon = |sx: f64| (ox + sx) / size * 360.0 - 180.0;
    let (_, lat_max) = world_px_to_lonlat(vp.zoom, 0.0, oy.clamp(0.0, size));
    let (_, lat_min) =
        world_px_to_lonlat(vp.zoom, 0.0, (oy + f64::from(vp.height)).clamp(0.0, size));
    [lon(0.0), lat_min, lon(f64::from(vp.width)), lat_max]
}

fn bbox_wgs84(vp: &ExportViewport) -> [f64; 4] {
    let [west, south, east, north] = geo_window(vp);
    let round = |v: f64| (v * 1000.0).round() / 1000.0;
    [
        round(west.max(-180.0)),
        round(south),
        round(east.min(180.0)),
        round(north),
    ]
}

/// Write straight-RGBA `tile` premultiplied into `layer` with its top-left at `(left, top)`.
fn blit_tile(layer: &mut Pixmap, tile: &[u8], left: i64, top: i64) {
    let (width, height) = (i64::from(layer.width()), i64::from(layer.height()));
    let size = i64::from(TILE_SIZE);
    let stride = layer.width() as usize * 4;
    let data = layer.data_mut();
    for v in 0..size {
        let y = top + v;
        if !(0..height).contains(&y) {
            continue;
        }
        for u in 0..size {
            let x = left + u;
            if !(0..width).contains(&x) {
                continue;
            }
            let src = ((v * size + u) * 4) as usize;
            let a = u32::from(tile[src + 3]);
            if a == 0 {
                continue;
            }
            let dst = y as usize * stride + x as usize * 4;
            for c in 0..3 {
                data[dst + c] = ((u32::from(tile[src + c]) * a + 127) / 255) as u8;
            }
            data[dst + 3] = a as u8;
        }
    }
}

/// Visit every tile that intersects the map, with its unwrapped top-left in map pixels.
fn visit_tiles(
    vp: &ExportViewport,
    mut visit: impl FnMut(u32, u32, i64, i64) -> Result<(), ComposeError>,
) -> Result<(), ComposeError> {
    let z = vp.zoom;
    let tile = i64::from(TILE_SIZE);
    let n = 1i64 << z;
    let (ox, oy) = map_origin(vp);
    let first_x = ox.div_euclid(tile);
    let last_x = (ox + i64::from(vp.width) - 1).div_euclid(tile);
    let first_y = oy.div_euclid(tile).max(0);
    let last_y = (oy + i64::from(vp.height) - 1).div_euclid(tile).min(n - 1);
    for ty in first_y..=last_y {
        for tx in first_x..=last_x {
            visit(
                tx.rem_euclid(n) as u32,
                ty as u32,
                tx * tile - ox,
                ty * tile - oy,
            )?;
        }
    }
    Ok(())
}

fn stroke_lines(
    layer: &mut Pixmap,
    lines: &[Vec<(f32, f32)>],
    color: [u8; 3],
    width: f32,
) -> Result<(), ComposeError> {
    let mut path = PathBuilder::new();
    for line in lines {
        for (index, (x, y)) in line.iter().enumerate() {
            if index == 0 {
                path.move_to(*x, *y);
            } else {
                path.line_to(*x, *y);
            }
        }
    }
    let Some(path) = path.finish() else {
        return Ok(());
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], 255);
    paint.anti_alias = true;
    let stroke = Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    layer.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    Ok(())
}

fn draw_coastlines(layer: &mut Pixmap, vp: &ExportViewport) -> Result<(), ComposeError> {
    let (ox, oy) = map_origin(vp);
    let (coast, border) =
        coastlines::polylines(coastlines::builtin(), geo_window(vp), |lon, lat| {
            let (wx, wy) = lonlat_to_world_px(vp.zoom, lon, lat);
            (wx - ox as f64, wy - oy as f64)
        });
    stroke_lines(layer, &border, BORDER_COLOR, 0.8)?;
    stroke_lines(layer, &coast, COAST_COLOR, 1.1)
}

fn draw_stations(layer: &mut Pixmap, req: &MapExportRequest) {
    for station in &req.stations {
        if !(-MAX_LATITUDE..=MAX_LATITUDE).contains(&station.lat) || !station.lon.is_finite() {
            continue;
        }
        let (x, y) = project(&req.viewport, station.lon, station.lat);
        let fill = if station.selected {
            SELECTED_STATION
        } else {
            PLAIN_STATION
        };
        for (radius, color) in [(4.5_f32, INK), (2.5_f32, fill)] {
            let Some(circle) = PathBuilder::from_circle(x as f32, y as f32, radius) else {
                continue;
            };
            let mut paint = Paint::default();
            paint.set_color_rgba8(color[0], color[1], color[2], 255);
            paint.anti_alias = true;
            layer.fill_path(
                &circle,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
}

fn layer_error(layer: &ExportLayer, message: impl Into<String>) -> ComposeError {
    ComposeError::Layer {
        layer_id: layer.id.clone(),
        message: message.into(),
    }
}

fn draw_forecast_layer(
    layer: &mut Pixmap,
    req: &MapExportRequest,
    field: Option<&CachedField>,
    check: &mut dyn FnMut() -> Result<(), ComposeError>,
) -> Result<(), ComposeError> {
    let (Some(scale), Some(forecast)) = (&req.color_scale, &req.forecast) else {
        return Err(ComposeError::Invalid(
            "forecast layer without forecast or color scale".into(),
        ));
    };
    let field = field.ok_or_else(|| {
        ComposeError::FieldMissing(req.field_id.clone().unwrap_or_else(|| "(none)".into()))
    })?;
    if field.valid_time_utc != forecast.valid_time_utc
        || field.reference_time_utc != forecast.reference_time_utc
    {
        return Err(ComposeError::FieldMismatch(format!(
            "the loaded field is {} valid {}, not the exported {} valid {}",
            field.reference_time_utc,
            field.valid_time_utc,
            forecast.reference_time_utc,
            forecast.valid_time_utc
        )));
    }
    let lut = ColorLut::new(&scale.palette).map_err(|e| ComposeError::Invalid(e.to_string()))?;
    let sampler = field.values.sampler();
    visit_tiles(&req.viewport, |x, y, left, top| {
        check()?;
        let (pixels, empty) = render_tile(
            &sampler,
            &lut,
            scale.min,
            scale.max,
            req.viewport.zoom,
            x,
            y,
        );
        if !empty {
            blit_tile(layer, &pixels, left, top);
        }
        Ok(())
    })
}

fn draw_archive_layer(
    layer: &mut Pixmap,
    req: &MapExportRequest,
    spec: &ExportLayer,
    archive: &PmtilesLayer,
    check: &mut dyn FnMut() -> Result<(), ComposeError>,
) -> Result<(), ComposeError> {
    visit_tiles(&req.viewport, |x, y, left, top| {
        check()?;
        let tile = archive
            .tile_rgba(req.viewport.zoom, x, y)
            .map_err(|e| layer_error(spec, e.to_string()))?;
        if let Some(tile) = tile {
            blit_tile(layer, &tile, left, top);
        }
        Ok(())
    })
}

fn fill_rect(pixmap: &mut Pixmap, rect: Rect, color: [u8; 3]) {
    let Some(sk) = SkRect::from_xywh(
        rect.x as f32,
        rect.y as f32,
        rect.width as f32,
        rect.height as f32,
    ) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], 255);
    paint.anti_alias = false;
    pixmap.fill_rect(sk, &paint, Transform::identity(), None);
}

fn format_number(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

fn draw_header(
    canvas: &mut Pixmap,
    req: &MapExportRequest,
    layout: &Layout,
) -> Result<(), ComposeError> {
    let max = layout.title.width.saturating_sub(PAD * 2) as f32;
    text::draw(
        canvas,
        &text::fit(&req.title, TITLE_SIZE, max),
        PAD as f32,
        26.0,
        TITLE_SIZE,
        INK,
    );
    text::draw(
        canvas,
        &text::fit(&req.subtitle, SUBTITLE_SIZE, max),
        PAD as f32,
        46.0,
        SUBTITLE_SIZE,
        MUTED,
    );
    let (Some(bar), Some(scale)) = (layout.colorbar, &req.color_scale) else {
        return Ok(());
    };
    let lut = ColorLut::new(&scale.palette).map_err(|e| ComposeError::Invalid(e.to_string()))?;
    for column in 0..bar.width {
        let t = f64::from(column) / f64::from(bar.width - 1);
        let color = lut.color(
            (scale.min + t * (scale.max - scale.min)) as f32,
            scale.min,
            scale.max,
        );
        fill_rect(
            canvas,
            Rect {
                x: bar.x + column,
                y: bar.y,
                width: 1,
                height: bar.height,
            },
            [color[0], color[1], color[2]],
        );
    }
    let label_y = (bar.y + bar.height + 14) as f32;
    let low = format_number(scale.min);
    let high = if scale.unit.is_empty() {
        format_number(scale.max)
    } else {
        format!("{} {}", format_number(scale.max), scale.unit)
    };
    text::draw(canvas, &low, bar.x as f32, label_y, LABEL_SIZE, INK);
    let high_width = text::width(&high, LABEL_SIZE);
    let high_x = (bar.x + bar.width) as f32 - high_width;
    text::draw(canvas, &high, high_x, label_y, LABEL_SIZE, INK);
    let low_end = bar.x as f32 + text::width(&low, LABEL_SIZE) + 8.0;
    let gap = high_x - 8.0 - low_end;
    if gap > 20.0 {
        let fitted = text::fit(&scale.palette, LABEL_SIZE, gap);
        let w = text::width(&fitted, LABEL_SIZE);
        text::draw(
            canvas,
            &fitted,
            low_end + (gap - w) * 0.5,
            label_y,
            LABEL_SIZE,
            MUTED,
        );
    }
    Ok(())
}

fn encode_png(canvas: &Pixmap) -> Result<Vec<u8>, ComposeError> {
    // The canvas is fully opaque, so premultiplied and straight RGBA are the same bytes.
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, canvas.width(), canvas.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Balanced);
    let mut writer = encoder
        .write_header()
        .map_err(|e| ComposeError::Encode(e.to_string()))?;
    writer
        .write_image_data(canvas.data())
        .map_err(|e| ComposeError::Encode(e.to_string()))?;
    writer
        .finish()
        .map_err(|e| ComposeError::Encode(e.to_string()))?;
    Ok(out)
}

fn provenance(kind: ExportLayerKind) -> ValueProvenance {
    match kind {
        ExportLayerKind::ForecastField => ValueProvenance::NativeNumeric,
        _ => ValueProvenance::DisplayOnly,
    }
}

fn manifest_entry(spec: &ExportLayer, archive: Option<&PmtilesLayer>) -> Value {
    let mut entry = json!({
        "id": spec.id,
        "kind": spec.kind,
        "opacity": spec.opacity,
        "value_provenance": provenance(spec.kind),
    });
    if let (Some(archive), Some(object)) = (archive, entry.as_object_mut()) {
        let info = archive.info(true);
        object.insert("path".into(), json!(spec.path));
        match &info.fingerprint_sha256 {
            Some(sha) => object.insert("sha256".into(), json!(sha)),
            None => object.insert("size_bytes".into(), json!(info.file_size)),
        };
        object.insert("tile_kind".into(), json!(info.tile_kind));
        object.insert("attribution".into(), json!(info.attribution));
    }
    entry
}

/// Render the snapshot. `archives` maps each archive layer id to its opened `.pmtiles`;
/// `check` is polled between tiles so cancellation aborts promptly.
pub fn compose(
    req: &MapExportRequest,
    field: Option<&CachedField>,
    archives: &HashMap<String, Arc<PmtilesLayer>>,
    check: &mut dyn FnMut() -> Result<(), ComposeError>,
) -> Result<Composed, ComposeError> {
    req.validate().map_err(ComposeError::Invalid)?;
    let layout = layout(req);
    let (width, height) = (layout.width, layout.height);
    let new_pixmap = |w: u32, h: u32| {
        Pixmap::new(w, h).ok_or_else(|| ComposeError::Invalid(format!("cannot allocate {w}x{h}")))
    };
    let mut map = new_pixmap(layout.map.width, layout.map.height)?;
    map.fill(tiny_skia::Color::from_rgba8(
        BACKGROUND[0],
        BACKGROUND[1],
        BACKGROUND[2],
        255,
    ));

    let mut manifest = Vec::with_capacity(req.layers.len());
    for spec in &req.layers {
        check()?;
        let mut layer = new_pixmap(layout.map.width, layout.map.height)?;
        let mut archive = None;
        match spec.kind {
            ExportLayerKind::BuiltinCoastlines => draw_coastlines(&mut layer, &req.viewport)?,
            ExportLayerKind::ForecastField => draw_forecast_layer(&mut layer, req, field, check)?,
            ExportLayerKind::StationOverlay => draw_stations(&mut layer, req),
            ExportLayerKind::LocalPmtiles => {
                let opened = archives
                    .get(&spec.id)
                    .ok_or_else(|| layer_error(spec, "the archive is not open"))?;
                draw_archive_layer(&mut layer, req, spec, opened, check)?;
                archive = Some(opened.as_ref());
            }
        }
        manifest.push(manifest_entry(spec, archive));
        let paint = PixmapPaint {
            opacity: spec.opacity.clamp(0.0, 1.0),
            ..PixmapPaint::default()
        };
        map.draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::identity(), None);
    }

    let mut canvas = new_pixmap(width, height)?;
    canvas.fill(tiny_skia::Color::from_rgba8(
        PANEL[0], PANEL[1], PANEL[2], 255,
    ));
    canvas.draw_pixmap(
        layout.map.x as i32,
        layout.map.y as i32,
        map.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    draw_header(&mut canvas, req, &layout)?;
    for (index, line) in footer_lines(req).iter().enumerate() {
        text::draw(
            &mut canvas,
            line,
            PAD as f32,
            (layout.footer.y + FOOTER_PAD + FOOTER_LINE * (index as u32 + 1) - 3) as f32,
            FOOTER_TEXT_SIZE,
            MUTED,
        );
    }
    check()?;
    Ok(Composed {
        png: encode_png(&canvas)?,
        bbox_wgs84: bbox_wgs84(&req.viewport),
        layout,
        layers: manifest,
    })
}

/// The `omo-export-sidecar/3` document for a committed composite. `file_name` is the PNG's
/// file name, `engine` the worker's engine/extension versions.
pub fn build_sidecar(
    req: &MapExportRequest,
    composed: &Composed,
    png_sha256: &str,
    file_name: &str,
    engine: Value,
    exported_at_utc: &str,
) -> Value {
    let forecast = shows_forecast(req)
        .then_some(req.forecast.as_ref())
        .flatten()
        .map(|f| {
            json!({
                "reference_time_utc": f.reference_time_utc,
                "cycle": f.cycle,
                "valid_time_utc": f.valid_time_utc,
                "source_layout": f.source_layout,
                "source_object": f.source_object,
                "object_version": f.object_version,
            })
        });
    let color_scale = shows_forecast(req)
        .then_some(req.color_scale.as_ref())
        .flatten()
        .map(|c| {
            json!({
                "palette": c.palette,
                "min": c.min,
                "max": c.max,
                "unit": c.unit,
                "locked": c.locked,
            })
        });
    let stations = (!req.stations.is_empty()).then(|| {
        json!({
            "sha256": req.stations_sha256,
            "count": req.stations.len(),
        })
    });
    let vp = &req.viewport;
    json!({
        "schema": omo_protocol::export::SIDECAR_SCHEMA_V3,
        "exported_at_utc": exported_at_utc,
        "data": {
            "file": file_name,
            "format": "png",
            "bytes": composed.png.len(),
            "sha256": png_sha256,
            "width": composed.layout.width,
            "height": composed.layout.height,
        },
        "engine": engine,
        "stations": stations,
        "catalog": {
            "entry_id": req.catalog_entry,
            "version": req.catalog_version,
        },
        "forecast": forecast,
        "viewport": {
            "crs": "EPSG:3857",
            "center_lat": vp.center_lat,
            "center_lon": vp.center_lon,
            "zoom": vp.zoom,
            "bbox_wgs84": composed.bbox_wgs84,
        },
        "color_scale": color_scale,
        "layers": composed.layers,
        "attribution": req.attribution,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_drop_noise_and_negative_zero() {
        assert_eq!(format_number(-12.45), "-12.45");
        assert_eq!(format_number(28.1), "28.1");
        assert_eq!(format_number(30.0), "30");
        assert_eq!(format_number(-0.001), "0");
    }

    #[test]
    fn tile_size_matches_the_protocol() {
        assert_eq!(TILE_SIZE, omo_protocol::map::MAP_TILE_SIZE);
    }
}
