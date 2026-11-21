use std::fmt::Write as _;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::studio::error::{invalid, StudioError, StudioErrorCode};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Count {
  pub value: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub reason: Option<String>,
}

impl Count {
  pub fn observed(value: u64) -> Self {
    Self {
      value: Some(value.to_string()),
      reason: None,
    }
  }

  pub fn unknown(reason: impl Into<String>) -> Self {
    Self {
      value: None,
      reason: Some(reason.into()),
    }
  }

  pub fn validate(&self) -> Result<(), StudioError> {
    match (&self.value, &self.reason) {
      (Some(value), None) => {
        parse_u64_decimal(value)?;
        Ok(())
      }
      (None, Some(reason)) if !reason.trim().is_empty() => Ok(()),
      _ => Err(invalid("count must be either a decimal value or an unknown reason")),
    }
  }
}

pub fn parse_u64_decimal(value: &str) -> Result<u64, StudioError> {
  if value == "0" {
    return Ok(0);
  }
  if value.is_empty()
    || value.starts_with('+')
    || value.starts_with('-')
    || value.as_bytes().first() == Some(&b'0')
    || !value.bytes().all(|byte| byte.is_ascii_digit())
  {
    return Err(invalid("counter must be a canonical decimal string"));
  }
  value
    .parse::<u64>()
    .map_err(|_| invalid("counter does not fit in an unsigned 64-bit integer"))
}

pub fn format_utc_micros(micros: i64) -> Result<String, StudioError> {
  Utc
    .timestamp_micros(micros)
    .single()
    .map(|value| value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
    .ok_or_else(|| invalid("timestamp is outside the UTC range"))
}

pub fn parse_utc_micros(value: &str) -> Result<i64, StudioError> {
  if !value.ends_with('Z') || value.contains('+') {
    return Err(invalid("timestamp must be explicit UTC"));
  }
  let parsed = DateTime::parse_from_rfc3339(value)
    .map_err(|_| invalid("timestamp is not ISO-8601"))?;
  let micros = parsed.timestamp_micros();
  let rendered = format_utc_micros(micros)?;
  if fractional_digits(value) > 6 || rendered != normalize_fraction(value) {
    return Err(invalid("timestamp precision must round-trip to UTC microseconds"));
  }
  Ok(micros)
}

fn fractional_digits(value: &str) -> usize {
  value
    .split_once('.')
    .map(|(head, tail)| {
      let _ = head;
      tail.trim_end_matches('Z').len()
    })
    .unwrap_or(0)
}

fn normalize_fraction(value: &str) -> String {
  let Ok(parsed) = DateTime::parse_from_rfc3339(value) else {
    return String::new();
  };
  parsed
    .with_timezone(&Utc)
    .to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

pub fn canonical_sha256(value: &Value) -> String {
  let mut bytes = Vec::new();
  write_canonical(&mut bytes, value);
  hex_sha256(&bytes)
}

pub fn hex_sha256(bytes: &[u8]) -> String {
  let mut hasher = Sha256::new();
  hasher.update(bytes);
  format!("{:x}", hasher.finalize())
}

fn write_canonical(output: &mut Vec<u8>, value: &Value) {
  match value {
    Value::Null => output.extend_from_slice(b"null"),
    Value::Bool(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
    Value::Number(value) => output.extend_from_slice(value.to_string().as_bytes()),
    Value::String(value) => {
      output.push(b'"');
      for char in value.chars() {
        match char {
          '"' => output.extend_from_slice(b"\\\""),
          '\\' => output.extend_from_slice(b"\\\\"),
          other => {
            let mut buf = [0; 4];
            output.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
          }
        }
      }
      output.push(b'"');
    }
    Value::Array(values) => {
      output.push(b'[');
      for (index, value) in values.iter().enumerate() {
        if index > 0 {
          output.push(b',');
        }
        write_canonical(output, value);
      }
      output.push(b']');
    }
    Value::Object(values) => {
      output.push(b'{');
      let mut keys: Vec<&String> = values.keys().collect();
      keys.sort();
      for (index, key) in keys.iter().enumerate() {
        if index > 0 {
          output.push(b',');
        }
        write_canonical(output, &Value::String((*key).clone()));
        output.push(b':');
        write_canonical(output, &values[*key]);
      }
      output.push(b'}');
    }
  }
}

pub fn redact_uri(value: &str) -> String {
  let Ok(mut url) = url::Url::parse(value) else {
    return "[unparsed-uri]".to_string();
  };
  let _ = url.set_username("");
  let _ = url.set_password(None);
  url.set_query(None);
  url.set_fragment(None);
  url.to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
  Pending,
  Verified,
  Unavailable,
  Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobPhase {
  Queued,
  Preparing,
  Reading,
  Validating,
  Publishing,
  Stopping,
  Succeeded,
  Cancelled,
  Failed,
  Exceeded,
  Interrupted,
}

impl JobPhase {
  pub fn is_terminal(self) -> bool {
    matches!(
      self,
      Self::Succeeded | Self::Cancelled | Self::Failed | Self::Exceeded | Self::Interrupted
    )
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesRow {
  pub station_id: String,
  pub valid_time: String,
  pub variable_id: String,
  pub value: Option<f32>,
  pub unit: Option<String>,
  pub target_lat: f64,
  pub target_lon: f64,
  pub source_lat: f64,
  pub source_lon: f64,
  pub source_point_index: String,
  pub source_object_id: String,
  pub quality: String,
}

pub fn object_without_secrets(value: &Value) -> Result<Map<String, Value>, StudioError> {
  let Value::Object(map) = value else {
    return Err(invalid("document must be an object"));
  };
  for key in map.keys() {
    let lower = key.to_ascii_lowercase();
    if lower.contains("password") || lower.contains("token") || lower.contains("secret") {
      return Err(StudioError::new(
        StudioErrorCode::InvalidRequest,
        serde_json::json!({"reason": "credentials cannot be stored"}),
      ));
    }
  }
  Ok(map.clone())
}

pub fn round_trip_f64(value: f64) -> Result<String, StudioError> {
  if !value.is_finite() {
    return Err(invalid("coordinate must be finite"));
  }
  let encoded = serde_json::to_string(&value).map_err(|_| invalid("coordinate encoding failed"))?;
  let decoded: f64 = serde_json::from_str(&encoded).map_err(|_| invalid("coordinate decoding failed"))?;
  if decoded.to_bits() != value.to_bits() {
    return Err(invalid("coordinate does not round-trip as Float64"));
  }
  Ok(encoded)
}

pub fn append_note(buffer: &mut String, note: &str) {
  let _ = write!(buffer, "{note}");
}
