//! Wire bodies for the M2 map worker operations (feature 003).
//!
//! Contract: `specs/003-m2-regional-tile-map/contracts/worker-protocol-m2.md` and
//! `data-model.md` §3 and §5. Protocol version stays `1`; these are additive request kinds.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    mercator::{MAX_ZOOM, MIN_ZOOM, TILE_SIZE},
    series::{CatalogEntry, ForecastRun},
    utc::parse_utc_rfc3339,
};

pub const MIN_MAP_ZOOM: u8 = MIN_ZOOM;
pub const MAX_MAP_ZOOM: u8 = MAX_ZOOM;
pub const MAP_TILE_SIZE: u32 = TILE_SIZE;
pub const DEFAULT_PALETTE: &str = "turbo";
pub const PALETTES: &[&str] = &["turbo", "viridis", "coolwarm", "magma", "cividis"];

pub const REQ_MAP_LOAD_FIELD: &str = "map_load_field";
pub const REQ_MAP_RENDER_TILES: &str = "map_render_tiles";
pub const REQ_MAP_INSPECT: &str = "map_inspect";
pub const REQ_MAP_LAYER_PROBE: &str = "map_layer_probe";

/// Where a displayed or inspected value comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueProvenance {
    /// Sampled directly from the worker's numeric forecast field.
    NativeNumeric,
    /// Reserved for post-M2 validated palette inversion.
    PaletteDecoded,
    /// Rendered imagery or vectors; never a measured value.
    DisplayOnly,
}

impl ValueProvenance {
    /// Label shown in the UI HUD.
    pub fn label(self) -> &'static str {
        match self {
            Self::NativeNumeric => "native numeric",
            Self::PaletteDecoded => "palette decoded",
            Self::DisplayOnly => "display only",
        }
    }
}

/// `z/x/y` slippy tile coordinate (`z` in `1..=12`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct TileCoord {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

impl TileCoord {
    pub fn is_valid(&self) -> bool {
        if !(MIN_MAP_ZOOM..=MAX_MAP_ZOOM).contains(&self.z) {
            return false;
        }
        let n = 1u32 << self.z;
        self.x < n && self.y < n
    }
}

/// Latitude band plus one or two longitude segments (two when crossing the antimeridian).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapBbox {
    pub lat_min: f64,
    pub lat_max: f64,
    pub lon_spans: Vec<[f64; 2]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceLayout {
    DataSpatial,
    DataRun,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldStats {
    pub valid_count: u64,
    pub min: f64,
    pub max: f64,
    pub unit: String,
}

/// Selector for `map_load_field.run` (`"latest_complete"` or an RFC 3339 UTC reference time).
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum MapRunSelector {
    LatestComplete,
    Explicit(String),
}

impl MapRunSelector {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text == "latest_complete" {
            return Ok(Self::LatestComplete);
        }
        if !text.ends_with('Z') || parse_utc_rfc3339(text).is_none() {
            return Err(format!(
                "run selector must be `latest_complete` or an RFC 3339 UTC timestamp ending in Z, got `{text}`"
            ));
        }
        Ok(Self::Explicit(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::LatestComplete => "latest_complete",
            Self::Explicit(utc) => utc.as_str(),
        }
    }
}

impl Serialize for MapRunSelector {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MapRunSelector {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoadFieldRequest {
    pub field_id: String,
    pub catalog_entry: CatalogEntry,
    pub catalog_version: String,
    pub run: MapRunSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_time_utc: Option<String>,
    pub bbox: MapBbox,
    #[serde(default)]
    pub force_data_run: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoadFieldResult {
    pub field_id: String,
    pub run: ForecastRun,
    pub valid_time_utc: String,
    pub source_layout: SourceLayout,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_notice: Option<String>,
    pub source_object: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_version: Option<String>,
    pub stats: FieldStats,
    pub coverage_bbox: MapBbox,
    pub scan_metrics: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readout_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorScale {
    pub palette: String,
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderLayerKind {
    ForecastField,
    LocalPmtiles,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderLayerRequest {
    pub layer_id: String,
    pub kind: RenderLayerKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderTilesRequest {
    pub result_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
    pub revision: u64,
    pub color_scale: ColorScale,
    pub layers: Vec<RenderLayerRequest>,
    pub tiles: Vec<TileCoord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderedTile {
    pub layer_id: String,
    pub z: u8,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub rgba_path: String,
    pub empty: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerError {
    pub layer_id: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderTilesResult {
    pub result_id: String,
    pub revision: u64,
    pub tiles: Vec<RenderedTile>,
    pub layer_errors: Vec<LayerError>,
}

fn basis(t1: f32, v0: f32, v1: f32, v2: f32, v3: f32) -> f32 {
    let t2 = t1 * t1;
    let t3 = t2 * t1;
    ((1.0 - 3.0 * t1 + 3.0 * t2 - t3) * v0
        + (4.0 - 6.0 * t2 + 3.0 * t3) * v1
        + (1.0 + 3.0 * t1 + 3.0 * t2 - 3.0 * t3) * v2
        + t3 * v3)
        / 6.0
}

fn basis_hex_rgb(stops: &[u32], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let n = stops.len() - 1;
    let rgb = |hex: u32| -> [f32; 3] {
        [
            ((hex >> 16) & 0xff) as f32 / 255.0,
            ((hex >> 8) & 0xff) as f32 / 255.0,
            (hex & 0xff) as f32 / 255.0,
        ]
    };
    if t <= 0.0 {
        let c = stops[0];
        return [
            ((c >> 16) & 0xff) as u8,
            ((c >> 8) & 0xff) as u8,
            (c & 0xff) as u8,
        ];
    }
    if t >= 1.0 {
        let c = stops[n];
        return [
            ((c >> 16) & 0xff) as u8,
            ((c >> 8) & 0xff) as u8,
            (c & 0xff) as u8,
        ];
    }
    let scaled = t * n as f32;
    let i = (scaled.floor() as usize).min(n - 1);
    let local = scaled - i as f32;
    let val0 = rgb(stops[i]);
    let val1 = rgb(stops[i + 1]);
    let prev = if i > 0 {
        rgb(stops[i - 1])
    } else {
        [
            2.0 * val0[0] - val1[0],
            2.0 * val0[1] - val1[1],
            2.0 * val0[2] - val1[2],
        ]
    };
    let next = if i + 1 < n {
        rgb(stops[i + 2])
    } else {
        [
            2.0 * val1[0] - val0[0],
            2.0 * val1[1] - val0[1],
            2.0 * val1[2] - val0[2],
        ]
    };
    let mut out = [0u8; 3];
    for ch in 0..3 {
        let v = basis(local, prev[ch], val0[ch], val1[ch], next[ch]).clamp(0.0, 1.0);
        out[ch] = (v * 255.0).round() as u8;
    }
    out
}

/// Evaluate one of the built-in named palettes (`PALETTES`) at `t` in `[0, 1]`.
pub fn palette_rgb(palette: &str, t: f32) -> Option<[u8; 3]> {
    const VIRIDIS: &[u32] = &[
        0x440154, 0x482777, 0x3f4a8a, 0x31678e, 0x26838f, 0x1f9d8a, 0x6cce5a, 0xb6de2b, 0xfee825,
    ];
    const MAGMA: &[u32] = &[
        0x000004, 0x140e37, 0x3b0f70, 0x641a80, 0x8c2981, 0xb63679, 0xde4968, 0xf66f5c, 0xfe9f6d,
        0xfece91, 0xfcfdbf,
    ];
    const COOLWARM: [[f32; 3]; 5] = [
        [59.0, 76.0, 192.0],
        [141.0, 176.0, 254.0],
        [221.0, 221.0, 221.0],
        [245.0, 156.0, 125.0],
        [180.0, 4.0, 38.0],
    ];
    let t = t.clamp(0.0, 1.0);
    let to_u8 = |v: f32| ((v.round() / 255.0).clamp(0.0, 1.0) * 255.0).round() as u8;
    match palette {
        "turbo" => {
            let r = 34.61
                + t * (1172.33 - t * (10793.56 - t * (33300.12 - t * (38394.49 - t * 14825.05))));
            let g =
                23.31 + t * (557.33 + t * (1225.33 - t * (3574.96 - t * (1073.77 + t * 707.56))));
            let b =
                27.2 + t * (3211.1 - t * (15327.97 - t * (27814.0 - t * (22569.18 - t * 6838.66))));
            Some([to_u8(r), to_u8(g), to_u8(b)])
        }
        "cividis" => {
            let r =
                -4.54 - t * (35.34 - t * (2381.73 - t * (6402.7 - t * (7024.72 - t * 2710.57))));
            let g = 32.49 + t * (170.73 + t * (52.82 - t * (131.46 - t * (176.58 - t * 67.37))));
            let b =
                81.24 + t * (442.36 - t * (2482.43 - t * (6167.24 - t * (6614.94 - t * 2475.67))));
            Some([to_u8(r), to_u8(g), to_u8(b)])
        }
        "viridis" => Some(basis_hex_rgb(VIRIDIS, t)),
        "magma" => Some(basis_hex_rgb(MAGMA, t)),
        "coolwarm" => {
            let scaled = t * 4.0;
            let index = (scaled.floor() as usize).min(3);
            let frac = scaled - index as f32;
            let ch = |c: usize| {
                (COOLWARM[index][c] + (COOLWARM[index + 1][c] - COOLWARM[index][c]) * frac).round()
                    as u8
            };
            Some([ch(0), ch(1), ch(2)])
        }
        _ => None,
    }
}

/// `count >= 2` evenly spaced `[r, g, b]` stops from low to high for UI colorbar legends.
pub fn palette_stops(palette: &str, count: usize) -> Option<Vec<[u8; 3]>> {
    const LUT_LAST: usize = 4095;
    if !PALETTES.contains(&palette) {
        return None;
    }
    let count = count.max(2);
    Some(
        (0..count)
            .map(|i| {
                let lut_index = i * LUT_LAST / (count - 1);
                palette_rgb(palette, lut_index as f32 / LUT_LAST as f32).unwrap_or([0, 0, 0])
            })
            .collect(),
    )
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InspectRequest {
    pub field_id: String,
    pub latitude: f64,
    pub longitude: f64,
    pub max_distance_m: u32,
}

/// Forecast-layer readout state (`FR-012`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ForecastSampleState {
    Value {
        value: f32,
        unit: String,
        grid_lat: f64,
        grid_lon: f64,
        distance_m: f64,
        provenance: ValueProvenance,
        stale: bool,
    },
    NoData {
        grid_lat: f64,
        grid_lon: f64,
        distance_m: f64,
    },
    OutsideCachedExtent,
    LayerHidden,
    NotLoaded,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InspectResult {
    pub latitude: f64,
    pub longitude: f64,
    pub sample: ForecastSampleState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerProbeRequest {
    pub layer_id: String,
    pub kind: RenderLayerKind,
    pub path: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PmtilesTileKind {
    RasterPng,
    RasterJpeg,
    RasterWebp,
    VectorMvt,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PmtilesLayerInfo {
    pub path: String,
    pub file_size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint_sha256: Option<String>,
    pub tile_kind: PmtilesTileKind,
    pub min_zoom: u8,
    pub max_zoom: u8,
    /// `[min_lon, min_lat, max_lon, max_lat]`.
    pub bounds: [f64; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub value_provenance: ValueProvenance,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerProbeResult {
    pub layer_id: String,
    pub info: PmtilesLayerInfo,
}

// ---- composite PNG export (`contracts/export-sidecar-v3.md`) ---------------------------------

/// Smallest and largest export canvas edge, in pixels.
pub const EXPORT_MIN_EDGE: u32 = 64;
pub const EXPORT_MAX_EDGE: u32 = 4096;

/// The map pane the export reproduces: the center and zoom the user sees and its size in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportViewport {
    pub center_lat: f64,
    pub center_lon: f64,
    pub zoom: u8,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportStation {
    pub id: String,
    pub lat: f64,
    pub lon: f64,
    #[serde(default)]
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportLayerKind {
    BuiltinCoastlines,
    ForecastField,
    StationOverlay,
    LocalPmtiles,
}

/// One visible layer of the stack; the request lists them bottom to top.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportLayer {
    pub id: String,
    pub kind: ExportLayerKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub opacity: f32,
}

/// The settled forecast the export shows, exactly as the field load reported it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportForecast {
    pub reference_time_utc: String,
    pub cycle: String,
    pub valid_time_utc: String,
    pub source_layout: SourceLayout,
    pub source_object: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportColorScale {
    pub palette: String,
    pub min: f64,
    pub max: f64,
    pub unit: String,
    #[serde(default)]
    pub locked: bool,
}

/// The immutable snapshot a map export is rendered from (`FR-021`): everything below was fixed
/// at the moment the user pressed Export, so later pans or layer edits cannot change it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapExportRequest {
    pub revision: u64,
    /// The worker-resident field the forecast layer is drawn from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
    pub viewport: ExportViewport,
    #[serde(default)]
    pub stations: Vec<ExportStation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stations_sha256: Option<String>,
    pub catalog_entry: String,
    pub catalog_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forecast: Option<ExportForecast>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_scale: Option<ExportColorScale>,
    pub layers: Vec<ExportLayer>,
    pub attribution: Vec<String>,
    /// Header lines drawn above the map.
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
}

impl MapExportRequest {
    /// Reject a snapshot the worker could not render faithfully.
    pub fn validate(&self) -> Result<(), String> {
        let vp = &self.viewport;
        if !(MIN_MAP_ZOOM..=MAX_MAP_ZOOM).contains(&vp.zoom) {
            return Err(format!("export zoom {} is outside 1..=12", vp.zoom));
        }
        if !vp.center_lat.is_finite() || !vp.center_lon.is_finite() {
            return Err("export viewport center must be finite".into());
        }
        for (name, edge) in [("width", vp.width), ("height", vp.height)] {
            if !(EXPORT_MIN_EDGE..=EXPORT_MAX_EDGE).contains(&edge) {
                return Err(format!(
                    "export {name} {edge} is outside {EXPORT_MIN_EDGE}..={EXPORT_MAX_EDGE}"
                ));
            }
        }
        if self.layers.is_empty() {
            return Err("export has no visible layer".into());
        }
        for layer in &self.layers {
            if !(0.0..=1.0).contains(&layer.opacity) {
                return Err(format!("layer `{}` has an invalid opacity", layer.id));
            }
            match (layer.kind, &layer.path) {
                (ExportLayerKind::LocalPmtiles, None) => {
                    return Err(format!("archive layer `{}` has no path", layer.id));
                }
                (ExportLayerKind::LocalPmtiles, Some(_)) => {}
                (_, Some(_)) => {
                    return Err(format!("layer `{}` must not carry a path", layer.id));
                }
                _ => {}
            }
        }
        let shows_forecast = self
            .layers
            .iter()
            .any(|layer| layer.kind == ExportLayerKind::ForecastField);
        if shows_forecast && (self.forecast.is_none() || self.color_scale.is_none()) {
            return Err("a visible forecast layer needs its forecast and color scale".into());
        }
        Ok(())
    }
}
