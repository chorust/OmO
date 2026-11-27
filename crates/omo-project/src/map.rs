//! Persisted M2 map view state (feature 003, `contracts/project-file-m2.md`).
//!
//! Everything here is canonicalised before it is hashed or written: `-0.0` becomes `0.0`,
//! numbers are clamped into their documented domains, and layers keep their order in a
//! `Vec`, so `.omoproj` bytes are identical across platforms (`SC-008`).

use std::path::PathBuf;

pub use omo_protocol::{
    map::{DEFAULT_PALETTE, PALETTES},
    mercator::{MAX_LATITUDE, MAX_ZOOM, MIN_ZOOM, normalize_lon},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_FORECAST_OPACITY: f32 = 0.78;

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum MapStateError {
    #[error("map state contains a non-finite number in `{0}`")]
    NonFinite(&'static str),
    #[error("map color range requires locked_min < locked_max (got {min} and {max})")]
    BadColorRange { min: String, max: String },
    #[error("map color range must lock both bounds or neither")]
    PartialColorRange,
    #[error("unknown palette `{0}`")]
    UnknownPalette(String),
    #[error("duplicate map layer id `{0}`")]
    DuplicateLayer(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum LayerStackError {
    #[error("`{0}` is not a local .pmtiles file path (remote URLs are not supported)")]
    NotLocalPmtiles(String),
    #[error("that archive is already mounted as layer `{0}`")]
    AlreadyMounted(String),
    #[error("unknown layer `{0}`")]
    UnknownLayer(String),
    #[error("built-in layer `{0}` cannot be removed (hide it instead)")]
    BuiltinLayer(String),
}

/// Where a layer moves in the bottom-to-top stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayerMove {
    /// Towards the top of the stack (drawn later).
    Up,
    /// Towards the bottom of the stack (drawn earlier).
    Down,
}

fn zero_fix(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapViewport {
    pub center_lat: f64,
    pub center_lon: f64,
    pub zoom: u8,
}

impl MapViewport {
    pub fn canonical(self) -> Result<Self, MapStateError> {
        if !self.center_lat.is_finite() {
            return Err(MapStateError::NonFinite("viewport.center_lat"));
        }
        if !self.center_lon.is_finite() {
            return Err(MapStateError::NonFinite("viewport.center_lon"));
        }
        let lon = normalize_lon(self.center_lon);
        Ok(Self {
            center_lat: zero_fix(self.center_lat.clamp(-MAX_LATITUDE, MAX_LATITUDE)),
            center_lon: zero_fix(lon),
            zoom: self.zoom.clamp(MIN_ZOOM, MAX_ZOOM),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorScaleConfig {
    pub palette: String,
    #[serde(default)]
    pub locked_min: Option<f64>,
    #[serde(default)]
    pub locked_max: Option<f64>,
}

impl Default for ColorScaleConfig {
    fn default() -> Self {
        Self {
            palette: DEFAULT_PALETTE.to_owned(),
            locked_min: None,
            locked_max: None,
        }
    }
}

impl ColorScaleConfig {
    pub fn canonical(mut self) -> Result<Self, MapStateError> {
        if !PALETTES.contains(&self.palette.as_str()) {
            return Err(MapStateError::UnknownPalette(self.palette));
        }
        match (self.locked_min, self.locked_max) {
            (None, None) => {}
            (Some(min), Some(max)) => {
                if !min.is_finite() || !max.is_finite() || min >= max {
                    return Err(MapStateError::BadColorRange {
                        min: format!("{min}"),
                        max: format!("{max}"),
                    });
                }
                self.locked_min = Some(zero_fix(min));
                self.locked_max = Some(zero_fix(max));
            }
            _ => return Err(MapStateError::PartialColorRange),
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MapLayerKind {
    ForecastField,
    BuiltinCoastlines,
    LocalPmtiles { path: PathBuf },
    StationOverlay,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapLayerConfig {
    pub id: String,
    pub kind: MapLayerKind,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
}

fn default_true() -> bool {
    true
}

fn default_opacity() -> f32 {
    1.0
}

impl MapLayerConfig {
    pub fn new(id: impl Into<String>, kind: MapLayerKind, visible: bool, opacity: f32) -> Self {
        Self {
            id: id.into(),
            kind,
            visible,
            opacity,
        }
    }

    pub fn canonical(mut self) -> Result<Self, MapStateError> {
        if !self.opacity.is_finite() {
            return Err(MapStateError::NonFinite("layer.opacity"));
        }
        self.opacity = self.opacity.clamp(0.0, 1.0);
        if self.opacity == 0.0 {
            self.opacity = 0.0;
        }
        Ok(self)
    }
}

/// Canonical opacity bits used by hashes (`-0.0` and `0.0` agree).
pub fn opacity_bits(opacity: f32) -> u32 {
    if opacity == 0.0 {
        0.0f32.to_bits()
    } else {
        opacity.to_bits()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapViewState {
    /// `None` means auto-fit the confirmed stations on open (`FR-002`).
    #[serde(default)]
    pub viewport: Option<MapViewport>,
    #[serde(default)]
    pub valid_time_lead_h: u32,
    #[serde(default)]
    pub color_scale: ColorScaleConfig,
    #[serde(default = "default_layers")]
    pub layers: Vec<MapLayerConfig>,
}

pub fn default_layers() -> Vec<MapLayerConfig> {
    vec![
        MapLayerConfig::new("coastlines", MapLayerKind::BuiltinCoastlines, true, 1.0),
        MapLayerConfig::new(
            "forecast",
            MapLayerKind::ForecastField,
            true,
            DEFAULT_FORECAST_OPACITY,
        ),
        MapLayerConfig::new("stations", MapLayerKind::StationOverlay, true, 1.0),
    ]
}

impl Default for MapViewState {
    fn default() -> Self {
        Self {
            viewport: None,
            valid_time_lead_h: 0,
            color_scale: ColorScaleConfig::default(),
            layers: default_layers(),
        }
    }
}

impl MapViewState {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Clamp into documented domains, normalise `-0.0`, and reject non-finite numbers,
    /// unknown palettes, bad color ranges, and duplicate layer ids.
    pub fn canonical(self) -> Result<Self, MapStateError> {
        let viewport = self.viewport.map(MapViewport::canonical).transpose()?;
        let color_scale = self.color_scale.canonical()?;
        let mut layers = Vec::with_capacity(self.layers.len());
        for layer in self.layers {
            let layer = layer.canonical()?;
            if layers
                .iter()
                .any(|seen: &MapLayerConfig| seen.id == layer.id)
            {
                return Err(MapStateError::DuplicateLayer(layer.id));
            }
            layers.push(layer);
        }
        Ok(Self {
            viewport,
            valid_time_lead_h: self.valid_time_lead_h,
            color_scale,
            layers,
        })
    }
}

/// Layer-stack editing (`FR-013`..`FR-016`). The stack is bottom-to-top; the station overlay is
/// pinned on top, so nothing moves above it and it does not move.
impl MapViewState {
    fn is_pinned_top(layer: &MapLayerConfig) -> bool {
        matches!(layer.kind, MapLayerKind::StationOverlay)
    }

    pub fn layer(&self, id: &str) -> Option<&MapLayerConfig> {
        self.layers.iter().find(|layer| layer.id == id)
    }

    fn layer_mut(&mut self, id: &str) -> Result<&mut MapLayerConfig, LayerStackError> {
        self.layers
            .iter_mut()
            .find(|layer| layer.id == id)
            .ok_or_else(|| LayerStackError::UnknownLayer(id.to_owned()))
    }

    /// Mount a local archive directly below the station overlay; returns the new layer id.
    pub fn add_local_pmtiles(&mut self, path: PathBuf) -> Result<String, LayerStackError> {
        let text = path.to_string_lossy().into_owned();
        let lower = text.to_ascii_lowercase();
        let remote = lower.contains("://");
        if remote || !lower.ends_with(".pmtiles") || path.as_os_str().is_empty() {
            return Err(LayerStackError::NotLocalPmtiles(text));
        }
        if let Some(existing) = self.layers.iter().find(|layer| {
            matches!(&layer.kind, MapLayerKind::LocalPmtiles { path: other } if *other == path)
        }) {
            return Err(LayerStackError::AlreadyMounted(existing.id.clone()));
        }
        let mut n = 1;
        let id = loop {
            let candidate = format!("pmtiles-{n}");
            if self.layer(&candidate).is_none() {
                break candidate;
            }
            n += 1;
        };
        let layer = MapLayerConfig::new(id.clone(), MapLayerKind::LocalPmtiles { path }, true, 1.0);
        let at = self
            .layers
            .iter()
            .position(Self::is_pinned_top)
            .unwrap_or(self.layers.len());
        self.layers.insert(at, layer);
        Ok(id)
    }

    /// Remove a mounted archive; the built-in layers can only be hidden.
    pub fn remove_layer(&mut self, id: &str) -> Result<(), LayerStackError> {
        let index = self
            .layers
            .iter()
            .position(|layer| layer.id == id)
            .ok_or_else(|| LayerStackError::UnknownLayer(id.to_owned()))?;
        if !matches!(self.layers[index].kind, MapLayerKind::LocalPmtiles { .. }) {
            return Err(LayerStackError::BuiltinLayer(id.to_owned()));
        }
        self.layers.remove(index);
        Ok(())
    }

    /// Swap a layer with its neighbour. Returns `false` (and changes nothing) at the ends of the
    /// stack and for the pinned station overlay.
    pub fn move_layer(&mut self, id: &str, direction: LayerMove) -> Result<bool, LayerStackError> {
        let index = self
            .layers
            .iter()
            .position(|layer| layer.id == id)
            .ok_or_else(|| LayerStackError::UnknownLayer(id.to_owned()))?;
        if Self::is_pinned_top(&self.layers[index]) {
            return Ok(false);
        }
        let other = match direction {
            LayerMove::Up if index + 1 < self.layers.len() => index + 1,
            LayerMove::Down if index > 0 => index - 1,
            _ => return Ok(false),
        };
        if Self::is_pinned_top(&self.layers[other]) {
            return Ok(false);
        }
        self.layers.swap(index, other);
        Ok(true)
    }

    pub fn set_layer_visible(&mut self, id: &str, visible: bool) -> Result<(), LayerStackError> {
        self.layer_mut(id)?.visible = visible;
        Ok(())
    }

    /// Non-finite values are ignored; others are clamped to `0.0..=1.0`.
    pub fn set_layer_opacity(&mut self, id: &str, opacity: f32) -> Result<(), LayerStackError> {
        let layer = self.layer_mut(id)?;
        if opacity.is_finite() {
            layer.opacity = opacity.clamp(0.0, 1.0);
        }
        Ok(())
    }
}
