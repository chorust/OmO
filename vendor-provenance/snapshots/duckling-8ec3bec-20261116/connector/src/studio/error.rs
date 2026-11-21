use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudioErrorCode {
  InvalidRequest,
  RequestConflict,
  StationValidation,
  StationSourceChanged,
  SourceRepairRequired,
  CatalogChanged,
  ProductUnavailable,
  UnitUnverified,
  GridUnsupported,
  GridMismatch,
  CoverageUnavailable,
  ReaderCapabilityUnavailable,
  ArtifactUnavailable,
  SessionExpired,
  SourceChanged,
  Transport,
  Decode,
  BudgetExceeded,
  Cancelled,
  Interrupted,
  CacheIo,
  ExportIo,
  ResultCapacityExceeded,
  ResultPageTooLarge,
  Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioError {
  pub code: StudioErrorCode,
  pub message_key: String,
  pub details: Value,
  pub retryable: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub job_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub revision: Option<String>,
}

impl StudioError {
  pub fn new(code: StudioErrorCode, details: Value) -> Self {
    let retryable = matches!(code, StudioErrorCode::Transport | StudioErrorCode::CacheIo);
    Self {
      message_key: format!("studio.error.{}", code.as_key()),
      code,
      details,
      retryable,
      job_id: None,
      revision: None,
    }
  }

  pub fn with_job(mut self, job_id: impl Into<String>, revision: impl Into<String>) -> Self {
    self.job_id = Some(job_id.into());
    self.revision = Some(revision.into());
    self
  }
}

impl StudioErrorCode {
  pub fn as_key(self) -> &'static str {
    match self {
      Self::InvalidRequest => "invalid_request",
      Self::RequestConflict => "request_conflict",
      Self::StationValidation => "station_validation",
      Self::StationSourceChanged => "station_source_changed",
      Self::SourceRepairRequired => "source_repair_required",
      Self::CatalogChanged => "catalog_changed",
      Self::ProductUnavailable => "product_unavailable",
      Self::UnitUnverified => "unit_unverified",
      Self::GridUnsupported => "grid_unsupported",
      Self::GridMismatch => "grid_mismatch",
      Self::CoverageUnavailable => "coverage_unavailable",
      Self::ReaderCapabilityUnavailable => "reader_capability_unavailable",
      Self::ArtifactUnavailable => "artifact_unavailable",
      Self::SessionExpired => "session_expired",
      Self::SourceChanged => "source_changed",
      Self::Transport => "transport",
      Self::Decode => "decode",
      Self::BudgetExceeded => "budget_exceeded",
      Self::Cancelled => "cancelled",
      Self::Interrupted => "interrupted",
      Self::CacheIo => "cache_io",
      Self::ExportIo => "export_io",
      Self::ResultCapacityExceeded => "result_capacity_exceeded",
      Self::ResultPageTooLarge => "result_page_too_large",
      Self::Internal => "internal",
    }
  }
}

impl std::fmt::Display for StudioError {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(formatter, "{}", self.code.as_key())
  }
}

impl std::error::Error for StudioError {}

pub fn details(entries: &[(&str, Value)]) -> Value {
  let mut map = Map::new();
  for (key, value) in entries {
    if *key == "password" || *key == "token" || *key == "authorization" || *key == "secret" {
      continue;
    }
    map.insert((*key).to_string(), value.clone());
  }
  Value::Object(map)
}

pub fn invalid(reason: &str) -> StudioError {
  StudioError::new(
    StudioErrorCode::InvalidRequest,
    details(&[("reason", Value::String(reason.to_string()))]),
  )
}
