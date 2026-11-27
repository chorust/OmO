//! M1 worker bodies: catalog entries, series requests, and readiness.
//!
//! These types extend protocol v1. The envelope, framing, and M0 bodies are unchanged.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::result_ref::ResultLimits;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Product {
    pub provider: String,
    pub model: String,
    pub open_meteo_domain: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub unit: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataLayout {
    pub kind: String,
    pub base: String,
    pub latest: String,
    pub object: String,
    pub meta: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridSpec {
    pub kind: String,
    pub duckomo_domain: String,
    pub points: u64,
}

/// Optional Open-Meteo `data_spatial` single-timestep layout (feature 003, `contracts/catalog-m2.md`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpatialLayout {
    pub kind: String,
    pub base: String,
    pub object: String,
}

pub const SPATIAL_LAYOUT_KIND: &str = "data_spatial";
const SPATIAL_OBJECT_TOKENS: &[&str] = &[
    "{YYYY}",
    "{MM}",
    "{DD}",
    "{HH}",
    "{valid_YYYY}",
    "{valid_MM}",
    "{valid_DD}",
    "{valid_HH}",
    "{valid_mm}",
];

impl SpatialLayout {
    /// Returns a human-readable reason when the layout is invalid. The `kind` is checked
    /// separately so callers can report `UnknownLayout`.
    pub fn validate_shape(&self) -> Result<(), String> {
        let base = self.base.as_str();
        if base.is_empty()
            || !(base.starts_with("s3://") || base.starts_with("https://"))
            || !base.ends_with('/')
        {
            return Err(format!(
                "spatial_layout.base `{base}` must be a non-empty s3:// or https:// URL ending with `/`"
            ));
        }
        for token in SPATIAL_OBJECT_TOKENS {
            if !self.object.contains(token) {
                return Err(format!("spatial_layout.object is missing token `{token}`"));
            }
        }
        Ok(())
    }
}

/// One `omo-catalog/1` entry. Field names match `contracts/catalog.md`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub is_default: bool,
    pub product: Product,
    pub variable: Variable,
    pub layout: DataLayout,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial_layout: Option<SpatialLayout>,
    pub grid: GridSpec,
    pub cycles: BTreeMap<String, u32>,
    pub default_window_h: [u32; 2],
    pub max_distance_m: u32,
    pub attribution: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LeadWindow {
    pub start_h: u32,
    pub end_h: u32,
}

impl LeadWindow {
    pub fn new(start_h: u32, end_h: u32) -> Self {
        Self { start_h, end_h }
    }
}

/// `"latest_complete"` or an explicit reference time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RunSelector {
    LatestComplete(String),
    Explicit { reference_time_utc: String },
}

impl RunSelector {
    pub fn latest_complete() -> Self {
        Self::LatestComplete("latest_complete".to_owned())
    }

    pub fn explicit(reference_time_utc: impl Into<String>) -> Self {
        Self::Explicit {
            reference_time_utc: reference_time_utc.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesRequest {
    pub stations_ref: String,
    pub stations_sha256: String,
    pub catalog_entry: CatalogEntry,
    pub catalog_version: String,
    pub run: RunSelector,
    pub window: LeadWindow,
    pub max_distance_m: u32,
    pub result_id: String,
    pub limits: ResultLimits,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForecastRun {
    pub reference_time_utc: String,
    pub cycle: String,
    pub horizon_h: u32,
    pub valid_times: Vec<String>,
    pub object_url: String,
    pub meta_etag: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectiveWindow {
    pub requested: [u32; 2],
    pub effective: [u32; 2],
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Readiness {
    Ready {
        run: ForecastRun,
    },
    NotReady {
        reason: NotReadyReason,
        detail: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotReadyReason {
    RemoteUnavailable,
    LatestMissing,
    VariableMissing,
    NoCompleteRun,
    GridMismatch,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogCheckRequest {
    pub entry: CatalogEntry,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogCheckResult {
    pub entry_id: String,
    pub readiness: Readiness,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolveRunRequest {
    pub entry: CatalogEntry,
    pub run: RunSelector,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolveRunResult {
    pub run: ForecastRun,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesResultBody {
    pub result_ref: serde_json::Value,
    pub parquet_path: String,
    pub matches_ref: serde_json::Value,
    #[serde(default)]
    pub plot_ref: Option<serde_json::Value>,
    pub run: ForecastRun,
    pub effective_window: EffectiveWindow,
    pub plan: String,
    pub scan_metrics: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SqlSessionRequest {
    pub stations_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_parquet: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SqlSessionResult {
    pub views: Vec<String>,
}

pub const CATALOG_SCHEMA: &str = "omo-catalog/1";
const KNOWN_LAYOUTS: &[&str] = &["data_run"];
const KNOWN_GRIDS: &[&str] = &["reduced_gaussian_o1280", "regular_latlon"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogDocument {
    pub schema: String,
    pub version: String,
    pub entries: Vec<CatalogEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CatalogValidationError {
    #[error("catalog JSON is invalid: {0}")]
    Json(String),
    #[error("unsupported catalog schema `{schema}`")]
    UnsupportedSchema { schema: String },
    #[error("catalog version is missing")]
    MissingVersion,
    #[error("catalog must contain exactly one default entry, found {count}")]
    DefaultCount { count: usize },
    #[error("unknown layout kind `{kind}`")]
    UnknownLayout { kind: String },
    #[error("unknown grid kind `{kind}`")]
    UnknownGrid { kind: String },
    #[error("invalid spatial_layout: {detail}")]
    InvalidSpatialLayout { detail: String },
    #[error("default window {start}–{end} h is outside the declared cycle horizons")]
    WindowOutsideHorizons { start: u32, end: u32 },
    #[error("catalog has no entries")]
    Empty,
}

#[derive(Deserialize)]
struct RawCatalogDocument {
    schema: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    entries: Vec<CatalogEntry>,
}

impl CatalogDocument {
    pub fn parse(text: &str) -> Result<Self, CatalogValidationError> {
        let raw: RawCatalogDocument = serde_json::from_str(text)
            .map_err(|error| CatalogValidationError::Json(error.to_string()))?;
        if raw.schema != CATALOG_SCHEMA {
            return Err(CatalogValidationError::UnsupportedSchema { schema: raw.schema });
        }
        let version = raw
            .version
            .filter(|version| !version.is_empty())
            .ok_or(CatalogValidationError::MissingVersion)?;
        if raw.entries.is_empty() {
            return Err(CatalogValidationError::Empty);
        }
        let defaults = raw.entries.iter().filter(|entry| entry.is_default).count();
        if defaults != 1 {
            return Err(CatalogValidationError::DefaultCount { count: defaults });
        }
        for entry in &raw.entries {
            if !KNOWN_LAYOUTS.contains(&entry.layout.kind.as_str()) {
                return Err(CatalogValidationError::UnknownLayout {
                    kind: entry.layout.kind.clone(),
                });
            }
            if let Some(spatial) = &entry.spatial_layout {
                if spatial.kind != SPATIAL_LAYOUT_KIND {
                    return Err(CatalogValidationError::UnknownLayout {
                        kind: spatial.kind.clone(),
                    });
                }
                spatial
                    .validate_shape()
                    .map_err(|detail| CatalogValidationError::InvalidSpatialLayout { detail })?;
            }
            if !KNOWN_GRIDS.contains(&entry.grid.kind.as_str()) {
                return Err(CatalogValidationError::UnknownGrid {
                    kind: entry.grid.kind.clone(),
                });
            }
            let max_horizon = entry.cycles.values().copied().max().unwrap_or(0);
            let [start, end] = entry.default_window_h;
            if entry.cycles.is_empty() || start > end || end > max_horizon {
                return Err(CatalogValidationError::WindowOutsideHorizons { start, end });
            }
        }
        Ok(Self {
            schema: raw.schema,
            version,
            entries: raw.entries,
        })
    }

    pub fn default_entry(&self) -> Option<&CatalogEntry> {
        self.entries.iter().find(|entry| entry.is_default)
    }

    pub fn entry_by_id(&self, id: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }
}
