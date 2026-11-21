use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cancel::CancelToken;
use crate::studio::error::{details, StudioError, StudioErrorCode};
use crate::studio::model::Count;

/// Internal adapter boundary. This is not a DuckOMO setting, SQL argument, or C ABI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityReport {
  pub schema_version: u32,
  pub reader_delivered: bool,
  pub artifact_id: Option<String>,
  pub engine: Option<String>,
  pub platform: Option<String>,
  pub shared_scope: bool,
  pub reserve_before_work: bool,
  pub cancel_and_drain: bool,
  pub raw_range_cache: bool,
  pub conditional_version_reads: bool,
  pub complete_metrics: bool,
  pub missing: Vec<String>,
  pub body_bytes: Count,
  pub note: String,
}

impl CapabilityReport {
  pub fn unavailable(note: impl Into<String>) -> Self {
    Self {
      schema_version: 1,
      reader_delivered: false,
      artifact_id: None,
      engine: None,
      platform: None,
      shared_scope: false,
      reserve_before_work: false,
      cancel_and_drain: false,
      raw_range_cache: false,
      conditional_version_reads: false,
      complete_metrics: false,
      missing: vec![
        "binding".into(),
        "shared_scope".into(),
        "reserve_before_work".into(),
        "cancel_and_drain".into(),
        "raw_range_cache".into(),
        "conditional_version_reads".into(),
        "complete_metrics".into(),
      ],
      body_bytes: Count::unknown("reader binding has not reported a measurement"),
      note: note.into(),
    }
  }

  pub fn remote_start_allowed(&self) -> bool {
    self.reader_delivered
      && self.shared_scope
      && self.reserve_before_work
      && self.cancel_and_drain
      && self.raw_range_cache
      && self.conditional_version_reads
      && self.complete_metrics
      && self.missing.is_empty()
      && self.artifact_id.is_some()
  }
}

#[derive(Debug, Clone)]
pub struct SeriesRequest {
  pub product_id: String,
  pub variable_id: String,
  pub filter: String,
  pub cancel: CancelToken,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourcePointRow {
  pub source_point_index: u64,
  pub source_lat: f64,
  pub source_lon: f64,
  pub valid_time: String,
  pub value: Option<f32>,
  pub source_object_id: String,
  pub version: String,
}

#[async_trait::async_trait]
pub trait ReaderExecutionPort: Send + Sync {
  fn capabilities(&self) -> CapabilityReport;
  async fn read_series(&self, request: SeriesRequest) -> Result<Vec<SourcePointRow>, StudioError>;
}

#[derive(Debug, Default)]
pub struct UnsupportedReader;

#[async_trait::async_trait]
impl ReaderExecutionPort for UnsupportedReader {
  fn capabilities(&self) -> CapabilityReport {
    CapabilityReport::unavailable(
      "D-READER is not delivered. v0.4.0 baseline series evidence is not a governance capability.",
    )
  }

  async fn read_series(&self, _request: SeriesRequest) -> Result<Vec<SourcePointRow>, StudioError> {
    Err(capability_error(&self.capabilities()))
  }
}

pub fn capability_error(report: &CapabilityReport) -> StudioError {
  StudioError::new(
    StudioErrorCode::ReaderCapabilityUnavailable,
    details(&[
      ("missing", Value::Array(report.missing.iter().cloned().map(Value::String).collect())),
      ("readerDelivered", json!(report.reader_delivered)),
    ]),
  )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityEvent {
  pub object_uri_redacted: String,
  pub size: Count,
  pub version: Option<String>,
  pub strength: String,
  pub phase: String,
}
