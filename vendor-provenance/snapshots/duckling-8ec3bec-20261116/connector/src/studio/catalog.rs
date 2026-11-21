use serde::Deserialize;
use serde_json::{json, Value};

use crate::studio::error::{details, StudioError, StudioErrorCode};
use crate::studio::model::{canonical_sha256, Readiness};

pub const CATALOG_JSON: &str = include_str!("catalog.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
  pub schema_version: u32,
  pub catalog_revision: String,
  pub products: Vec<CatalogProduct>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogProduct {
  pub product_id: String,
  pub title: String,
  pub provider: String,
  pub provider_url: String,
  pub license_url: Option<String>,
  pub attribution: String,
  pub representation: String,
  pub directory_template: Option<String>,
  pub model_key: String,
  pub native_step_seconds: Option<u64>,
  pub time_origin_utc: Option<String>,
  pub readiness: Readiness,
  pub evidence_ref: Option<String>,
  pub variables: Vec<CatalogVariable>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogVariable {
  pub variable_id: String,
  pub unit: Option<String>,
  pub height_or_level: Option<String>,
  pub quantity_kind: Option<String>,
  pub readiness: Readiness,
}

pub fn load_catalog() -> Result<Catalog, StudioError> {
  let catalog: Catalog = serde_json::from_str(CATALOG_JSON).map_err(|error| {
    StudioError::new(
      StudioErrorCode::Internal,
      details(&[("reason", json!(format!("catalog schema is invalid: {error}")))]),
    )
  })?;
  validate_catalog(&catalog)?;
  Ok(catalog)
}

pub fn validate_catalog(catalog: &Catalog) -> Result<(), StudioError> {
  if catalog.schema_version != 1 {
    return Err(StudioError::new(
      StudioErrorCode::CatalogChanged,
      details(&[("reason", json!("unsupported catalog schema"))]),
    ));
  }
  let mut seen = Vec::new();
  for product in &catalog.products {
    if product.representation != "om-rolling-data" {
      return Err(StudioError::new(
        StudioErrorCode::ProductUnavailable,
        details(&[("productId", json!(product.product_id))]),
      ));
    }
    if let Some(template) = &product.directory_template {
      let lower = template.to_ascii_lowercase();
      if lower.contains("://") && !(lower.starts_with("https://") || lower.starts_with("s3://"))
        || template.contains('?')
        || template.contains('#')
        || template.contains('*')
        || template.contains('@')
      {
        return Err(StudioError::new(
          StudioErrorCode::InvalidRequest,
          details(&[("reason", json!("directory template is not a credential-free HTTPS or S3 prefix"))]),
        ));
      }
    }
    if product.readiness == Readiness::Verified && product.evidence_ref.as_deref().unwrap_or("").is_empty()
    {
      return Err(StudioError::new(
        StudioErrorCode::ProductUnavailable,
        details(&[("reason", json!("verified readiness requires evidence"))]),
      ));
    }
    for variable in &product.variables {
      if variable.readiness == Readiness::Verified && variable.unit.as_deref().unwrap_or("").is_empty() {
        return Err(StudioError::new(
          StudioErrorCode::UnitUnverified,
          details(&[("variableId", json!(variable.variable_id))]),
        ));
      }
      if variable.variable_id == "temperature_80m" {
        return Err(StudioError::new(
          StudioErrorCode::ProductUnavailable,
          details(&[("reason", json!("80 m is not a substitute for temperature_2m"))]),
        ));
      }
      seen.push(format!("{}:{}", product.product_id, variable.variable_id));
    }
  }
  Ok(())
}

pub fn require_executable<'a>(
  catalog: &'a Catalog,
  product_id: &str,
  variable_id: &str,
  expected_revision: &str,
) -> Result<(&'a CatalogProduct, &'a CatalogVariable), StudioError> {
  if catalog.catalog_revision != expected_revision {
    return Err(StudioError::new(
      StudioErrorCode::CatalogChanged,
      details(&[("catalogRevision", json!(catalog.catalog_revision))]),
    ));
  }
  let product = catalog
    .products
    .iter()
    .find(|product| product.product_id == product_id)
    .ok_or_else(|| {
      StudioError::new(
        StudioErrorCode::ProductUnavailable,
        details(&[("productId", json!(product_id))]),
      )
    })?;
  let variable = product
    .variables
    .iter()
    .find(|variable| variable.variable_id == variable_id)
    .ok_or_else(|| {
      StudioError::new(
        StudioErrorCode::ProductUnavailable,
        details(&[("variableId", json!(variable_id))]),
      )
    })?;
  if product.readiness != Readiness::Verified || variable.readiness != Readiness::Verified {
    return Err(StudioError::new(
      StudioErrorCode::ProductUnavailable,
      details(&[
        ("readiness", json!(format!("{:?}", product.readiness))),
        ("reason", json!("product or variable is not verified")),
      ]),
    ));
  }
  if variable.unit.is_none() {
    return Err(StudioError::new(
      StudioErrorCode::UnitUnverified,
      details(&[("variableId", json!(variable_id))]),
    ));
  }
  if product.directory_template.is_none() {
    return Err(StudioError::new(
      StudioErrorCode::ProductUnavailable,
      details(&[("reason", json!("directory template is not qualified"))]),
    ));
  }
  Ok((product, variable))
}

pub fn catalog_hash() -> String {
  let value: Value = serde_json::from_str(CATALOG_JSON).unwrap_or(Value::Null);
  canonical_sha256(&value)
}
