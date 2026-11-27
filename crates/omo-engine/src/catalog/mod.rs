//! Product catalog `omo-catalog/1` (research R-02, contracts/catalog.md).

pub mod run;

use std::{fs, path::Path};

use omo_protocol::series::{
    CATALOG_SCHEMA as PROTOCOL_CATALOG_SCHEMA, CatalogDocument, CatalogEntry,
    CatalogValidationError, NotReadyReason, Readiness,
};
use thiserror::Error;

use crate::conn::BootstrappedConn;
use crate::grid::reduced_gaussian::O1280;
use crate::series::quote_sql_literal;

pub use run::{
    ConnMetaSource, EffectiveWindow, ForecastRun, MetaSource, RunError, RunMeta, RunRef,
    effective_window, resolve, time_index_table,
};

pub const CATALOG_SCHEMA: &str = PROTOCOL_CATALOG_SCHEMA;

#[derive(Clone, Debug, PartialEq)]
pub struct Catalog {
    pub schema: String,
    pub version: String,
    pub entries: Vec<CatalogEntry>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("catalog file could not be read: {0}")]
    Io(String),
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

impl Catalog {
    pub fn load(path: &Path) -> Result<Self, CatalogError> {
        let text = fs::read_to_string(path).map_err(|error| CatalogError::Io(error.to_string()))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, CatalogError> {
        let doc = CatalogDocument::parse(text).map_err(|error| match error {
            CatalogValidationError::Json(msg) => CatalogError::Json(msg),
            CatalogValidationError::UnsupportedSchema { schema } => {
                CatalogError::UnsupportedSchema { schema }
            }
            CatalogValidationError::MissingVersion => CatalogError::MissingVersion,
            CatalogValidationError::DefaultCount { count } => CatalogError::DefaultCount { count },
            CatalogValidationError::UnknownLayout { kind } => CatalogError::UnknownLayout { kind },
            CatalogValidationError::UnknownGrid { kind } => CatalogError::UnknownGrid { kind },
            CatalogValidationError::InvalidSpatialLayout { detail } => {
                CatalogError::InvalidSpatialLayout { detail }
            }
            CatalogValidationError::WindowOutsideHorizons { start, end } => {
                CatalogError::WindowOutsideHorizons { start, end }
            }
            CatalogValidationError::Empty => CatalogError::Empty,
        })?;
        Ok(Self {
            schema: doc.schema,
            version: doc.version,
            entries: doc.entries,
        })
    }

    pub fn default_entry(&self) -> &CatalogEntry {
        self.entries
            .iter()
            .find(|entry| entry.is_default)
            .expect("catalog validation requires exactly one default entry")
    }
}

pub fn object_url(entry: &CatalogEntry, reference_time_utc: &str) -> String {
    render_template(entry, &entry.layout.object, reference_time_utc)
}

pub fn meta_url(entry: &CatalogEntry, reference_time_utc: &str) -> String {
    render_template(entry, &entry.layout.meta, reference_time_utc)
}

/// `data_spatial` object URL for one run and one valid time, or `None` when the entry does
/// not declare a `spatial_layout`.
pub fn spatial_object_url(
    entry: &CatalogEntry,
    reference_time_utc: &str,
    valid_time_utc: &str,
) -> Option<String> {
    let spatial = entry.spatial_layout.as_ref()?;
    let run = run::split_reference(reference_time_utc);
    let valid = run::split_reference(valid_time_utc);
    let valid_digits: String = valid_time_utc
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect();
    let valid_minute = valid_digits.get(10..12).unwrap_or("00").to_owned();
    let relative = spatial
        .object
        .replace("{valid_YYYY}", &valid.year)
        .replace("{valid_MM}", &valid.month)
        .replace("{valid_DD}", &valid.day)
        .replace("{valid_HH}", &valid.hour)
        .replace("{valid_mm}", &valid_minute)
        .replace("{YYYY}", &run.year)
        .replace("{MM}", &run.month)
        .replace("{DD}", &run.day)
        .replace("{HH}", &run.hour);
    Some(format!(
        "{}/{}",
        spatial.base.trim_end_matches('/'),
        relative.trim_start_matches('/')
    ))
}

fn render_template(entry: &CatalogEntry, template: &str, reference_time_utc: &str) -> String {
    let parts = run::split_reference(reference_time_utc);
    let relative = template
        .replace("{YYYY}", &parts.year)
        .replace("{MM}", &parts.month)
        .replace("{DD}", &parts.day)
        .replace("{HH}", &parts.hour)
        .replace("{variable}", &entry.variable.name);
    format!(
        "{}{}",
        entry.layout.base.trim_end_matches('/'),
        if relative.starts_with('/') {
            relative
        } else {
            format!("/{relative}")
        }
    )
}

/// Public HTTPS form of an `s3://openmeteo/...` URL. No DuckDB secret is created.
pub fn open_meteo_https(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("s3://openmeteo/") {
        format!("https://openmeteo.s3.amazonaws.com/{rest}")
    } else {
        url.to_owned()
    }
}

pub trait GridInfoSource {
    fn grid_info(&self, object_url: &str, domain: &str) -> Result<GridProbe, String>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridProbe {
    pub point_count: u64,
    pub samples: Vec<(u32, f32, f32)>,
    pub unit: Option<String>,
}

/// Extract total spatial point count from `om_grid_info` JSON without fabricating a default.
pub fn parse_grid_point_count(
    def: &serde_json::Value,
    layout: &serde_json::Value,
) -> Result<u64, String> {
    if let Some(points) = def
        .pointer("/parameters/points")
        .or_else(|| def.pointer("/parameters/total_points"))
        .or_else(|| def.pointer("/points"))
        .or_else(|| def.pointer("/parameters/nx"))
        .and_then(|value| value.as_u64())
    {
        return Ok(points);
    }
    if let Some(shape) = layout.pointer("/shape").and_then(|v| v.as_array()) {
        if let Some(axes) = layout.pointer("/axes").and_then(|v| v.as_array()) {
            let mut spatial_product: Option<u64> = None;
            for (axis, dim) in axes.iter().zip(shape.iter()) {
                let axis_name = axis.as_str().unwrap_or("");
                if matches!(
                    axis_name,
                    "location" | "point" | "points" | "grid_point" | "lat" | "lon"
                ) && let Some(n) = dim.as_u64()
                {
                    spatial_product = Some(spatial_product.unwrap_or(1).saturating_mul(n));
                }
            }
            if let Some(points) = spatial_product {
                return Ok(points);
            }
        }
        if let Some(first) = shape.first().and_then(|v| v.as_u64()) {
            return Ok(first);
        }
    }
    Err("om_grid_info did not report a grid point count".to_owned())
}

pub struct DuckDbGridInfo<'a> {
    pub conn: &'a BootstrappedConn,
}

impl GridInfoSource for DuckDbGridInfo<'_> {
    fn grid_info(&self, object_url: &str, domain: &str) -> Result<GridProbe, String> {
        let connection = self.conn.connection().map_err(|error| error.to_string())?;
        let https = open_meteo_https(object_url);
        let sql = format!(
            "SELECT definition, layout, provenance FROM om_grid_info({}, domain := {})",
            quote_sql_literal(&https),
            quote_sql_literal(domain)
        );
        let (def_json, layout_json, prov_json): (String, String, String) = connection
            .query_row(&sql, [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|error| error.to_string())?;
        let def: serde_json::Value =
            serde_json::from_str(&def_json).map_err(|error| error.to_string())?;
        let layout: serde_json::Value =
            serde_json::from_str(&layout_json).map_err(|error| error.to_string())?;
        let provenance: serde_json::Value =
            serde_json::from_str(&prov_json).unwrap_or(serde_json::Value::Null);
        let point_count = parse_grid_point_count(&def, &layout)?;
        let unit = provenance
            .pointer("/unit")
            .or_else(|| def.pointer("/unit"))
            .and_then(|value| value.as_str())
            .map(str::to_owned);

        // Query real duckomo coordinates at strategic O1280 anchor indices (poles, mid-latitudes,
        // equator) and return the coordinates observed from duckomo for verification.
        let total = O1280::points();
        let anchors = [0u32, total / 4, total / 2, (total / 4) * 3, total - 1];
        let mut samples = Vec::with_capacity(anchors.len());
        for flat in anchors {
            let (expected_lat, expected_lon) = O1280::point(flat);
            let point = crate::series::plan_g::SelectedPoint {
                flat_index: flat,
                lat: expected_lat,
                lon: expected_lon,
            };
            let coords = crate::series::plan_g::probe_om_point_coordinates(
                self.conn, object_url, domain, &point,
            )
            .map_err(|error| error.to_string())?;
            if coords.len() != 1 {
                return Err(format!(
                    "duckomo returned {} coordinates for O1280 flat index {flat}; expected 1",
                    coords.len()
                ));
            }
            let (actual_lat, actual_lon) = coords[0];
            samples.push((flat, actual_lat, actual_lon));
        }
        Ok(GridProbe {
            point_count,
            samples,
            unit,
        })
    }
}

pub fn check_readiness_with(
    entry: &CatalogEntry,
    remote_available: bool,
    meta: &dyn MetaSource,
    grid: &dyn GridInfoSource,
) -> Readiness {
    if !remote_available {
        return Readiness::NotReady {
            reason: NotReadyReason::RemoteUnavailable,
            detail: "HTTPFS remote capability is unavailable".into(),
        };
    }
    let forecast = match resolve(entry, meta, RunRef::LatestComplete) {
        Ok(forecast) => forecast,
        Err(RunError::CapabilityUnavailable(detail)) => {
            return Readiness::NotReady {
                reason: NotReadyReason::RemoteUnavailable,
                detail,
            };
        }
        Err(RunError::LatestMissing(detail) | RunError::Io(detail)) => {
            return Readiness::NotReady {
                reason: NotReadyReason::LatestMissing,
                detail,
            };
        }
        Err(RunError::VariableMissing {
            variable,
            reference_time_utc,
        }) => {
            return Readiness::NotReady {
                reason: NotReadyReason::VariableMissing,
                detail: format!("{variable} missing in {reference_time_utc}"),
            };
        }
        Err(RunError::NoCompleteRun) => {
            if let Ok(latest) = meta.latest()
                && !latest.variables.iter().any(|v| v == &entry.variable.name)
            {
                return Readiness::NotReady {
                    reason: NotReadyReason::VariableMissing,
                    detail: format!(
                        "{} missing in {}",
                        entry.variable.name, latest.reference_time_utc
                    ),
                };
            }
            return Readiness::NotReady {
                reason: NotReadyReason::NoCompleteRun,
                detail: "no completely published run in the last 8 cycles".into(),
            };
        }
        Err(error) => {
            return Readiness::NotReady {
                reason: NotReadyReason::LatestMissing,
                detail: error.to_string(),
            };
        }
    };
    if entry.variable.unit.trim().is_empty() {
        return Readiness::NotReady {
            reason: NotReadyReason::VariableMissing,
            detail: format!("variable {} has no declared unit", entry.variable.name),
        };
    }
    if entry.grid.kind == "reduced_gaussian_o1280" {
        match grid.grid_info(&forecast.object_url, &entry.grid.duckomo_domain) {
            Ok(probe) => {
                if let Some(actual_unit) = probe.unit.as_deref()
                    && actual_unit != entry.variable.unit
                {
                    return Readiness::NotReady {
                        reason: NotReadyReason::VariableMissing,
                        detail: format!(
                            "variable {} unit mismatch: expected `{}`, got `{actual_unit}`",
                            entry.variable.name, entry.variable.unit
                        ),
                    };
                }
                if probe.point_count != u64::from(O1280::points()) {
                    return Readiness::NotReady {
                        reason: NotReadyReason::GridMismatch,
                        detail: format!(
                            "om_grid_info reported {} points; expected {}",
                            probe.point_count,
                            O1280::points()
                        ),
                    };
                }
                if probe.samples.is_empty() {
                    return Readiness::NotReady {
                        reason: NotReadyReason::GridMismatch,
                        detail: "grid readiness check returned no sampled coordinates".into(),
                    };
                }
                for (flat, lat, lon) in probe.samples {
                    let (expected_lat, expected_lon) = O1280::point(flat);
                    if lat.to_bits() != expected_lat.to_bits()
                        || lon.to_bits() != expected_lon.to_bits()
                    {
                        return Readiness::NotReady {
                            reason: NotReadyReason::GridMismatch,
                            detail: format!("coordinate mismatch at flat index {flat}"),
                        };
                    }
                }
            }
            Err(detail) => {
                return Readiness::NotReady {
                    reason: NotReadyReason::GridMismatch,
                    detail,
                };
            }
        }
    }
    Readiness::Ready { run: forecast }
}

pub fn check_readiness(conn: &BootstrappedConn, entry: &CatalogEntry) -> Readiness {
    let meta = ConnMetaSource { conn, entry };
    let grid = DuckDbGridInfo { conn };
    check_readiness_with(entry, conn.capabilities().remote, &meta, &grid)
}
