use serde::Deserialize;
use serde_json::{Map, Value};

use crate::studio::error::{details, invalid, StudioError, StudioErrorCode};
use crate::studio::model::SCHEMA_VERSION;

pub const PROFILE_JSON: &str = include_str!("../../../src/lib/studio/profile.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceProfile {
  pub schema_version: u32,
  pub profile_id: String,
  pub status: String,
  pub frozen_on: String,
  pub task: TaskLimits,
  pub transport: TransportLimits,
  pub work: WorkLimits,
  pub cache: CacheLimits,
  pub stations: StationLimits,
  pub results: ResultLimits,
  pub history: HistoryLimits,
  pub comparison: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskLimits {
  pub max_stations: u64,
  pub max_products: u64,
  pub max_variables: u64,
  pub max_window_seconds: u64,
  pub inclusive_endpoints: bool,
  pub max_result_rows: u64,
  pub max_active_jobs: u64,
  pub max_queued_jobs: u64,
  pub max_queue_seconds: u64,
  pub max_execution_seconds: u64,
  pub max_stop_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransportLimits {
  pub max_received_body_bytes: u64,
  pub max_response_body_bytes: u64,
  pub max_attempts: u64,
  pub max_concurrent_requests: u64,
  pub request_timeout_seconds: u64,
  pub max_retries_per_logical_request: u64,
  pub max_redirects_per_logical_request: u64,
  pub body_encoding: String,
  pub counter_scope: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkLimits {
  pub max_live_owned_bytes: u64,
  pub duckdb_managed_sub_limit_bytes: u64,
  pub max_decode_block_visits: u64,
  pub memory_coverage_required: String,
  pub process_rss_acceptance_max_bytes: u64,
  pub process_rss_is_runtime_hard_limit: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CacheLimits {
  pub default_capacity_bytes: u64,
  pub max_capacity_bytes: u64,
  pub max_entry_bytes: u64,
  pub max_entries: u64,
  pub max_index_bytes: u64,
  pub charge_includes: String,
  pub cross_job_reuse_requires_revalidation: bool,
  pub decoded_values_persisted: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StationLimits {
  pub max_file_bytes: u64,
  pub max_imported_rows: u64,
  pub max_columns: u64,
  pub max_cell_utf8_bytes: u64,
  pub max_grid_points: u64,
  pub nearest_cancellation_check_interval: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResultLimits {
  pub max_retained_backing_bytes: u64,
  pub max_handles: u64,
  pub max_page_rows: u64,
  pub max_concurrent_operations: u64,
  pub max_live_operation_temporary_bytes: u64,
  pub max_serialized_response_bytes: u64,
  pub max_operation_seconds: u64,
  pub retained_scope: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryLimits {
  pub max_completed_run_summaries: u64,
  pub max_summary_bytes_per_run: u64,
  pub max_diagnostic_bytes_per_run: u64,
  pub max_total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveLimits {
  pub max_stations: u64,
  pub max_result_rows: u64,
  pub max_window_seconds: u64,
  pub max_received_body_bytes: u64,
  pub max_response_body_bytes: u64,
  pub max_attempts: u64,
  pub max_decode_block_visits: u64,
  pub max_live_owned_bytes: u64,
  pub cache_capacity_bytes: u64,
}

pub fn load_profile() -> Result<AcceptanceProfile, StudioError> {
  let profile: AcceptanceProfile = serde_json::from_str(PROFILE_JSON)
    .map_err(|error| invalid(&format!("profile schema is invalid: {error}")))?;
  if profile.schema_version != SCHEMA_VERSION || profile.profile_id != "m1-v1" {
    return Err(invalid("profile id or schema is not m1-v1"));
  }
  if profile.task.max_active_jobs != 1 || profile.task.max_queued_jobs != 8 {
    return Err(invalid("profile admission limits drifted"));
  }
  if profile.work.process_rss_is_runtime_hard_limit || profile.cache.decoded_values_persisted {
    return Err(invalid("profile measurement rules drifted"));
  }
  Ok(profile)
}

pub fn effective_limits(
  profile: &AcceptanceProfile,
  lowered: Option<&Map<String, Value>>,
) -> Result<EffectiveLimits, StudioError> {
  let mut limits = EffectiveLimits {
    max_stations: profile.task.max_stations,
    max_result_rows: profile.task.max_result_rows,
    max_window_seconds: profile.task.max_window_seconds,
    max_received_body_bytes: profile.transport.max_received_body_bytes,
    max_response_body_bytes: profile.transport.max_response_body_bytes,
    max_attempts: profile.transport.max_attempts,
    max_decode_block_visits: profile.work.max_decode_block_visits,
    max_live_owned_bytes: profile.work.max_live_owned_bytes,
    cache_capacity_bytes: profile.cache.default_capacity_bytes,
  };
  let Some(lowered) = lowered else {
    return Ok(limits);
  };
  for (key, value) in lowered {
    let requested = value
      .as_u64()
      .ok_or_else(|| invalid("lowered limit must be an integer"))?;
    let slot = match key.as_str() {
      "maxStations" => &mut limits.max_stations,
      "maxResultRows" => &mut limits.max_result_rows,
      "maxWindowSeconds" => &mut limits.max_window_seconds,
      "maxReceivedBodyBytes" => &mut limits.max_received_body_bytes,
      "maxResponseBodyBytes" => &mut limits.max_response_body_bytes,
      "maxAttempts" => &mut limits.max_attempts,
      "maxDecodeBlockVisits" => &mut limits.max_decode_block_visits,
      "maxLiveOwnedBytes" => &mut limits.max_live_owned_bytes,
      "cacheCapacityBytes" => &mut limits.cache_capacity_bytes,
      other => {
        return Err(StudioError::new(
          StudioErrorCode::InvalidRequest,
          details(&[("reason", Value::String(format!("unknown limit {other}")))]),
        ));
      }
    };
    let ceiling = *slot;
    let allows_zero = key == "cacheCapacityBytes";
    if requested > ceiling || (!allows_zero && requested == 0) {
      return Err(StudioError::new(
        StudioErrorCode::InvalidRequest,
        details(&[
          ("limit", Value::String(key.clone())),
          ("reason", Value::String("limits can only be lowered within the profile".into())),
        ]),
      ));
    }
    *slot = requested;
  }
  if limits.cache_capacity_bytes > profile.cache.max_capacity_bytes {
    return Err(invalid("cache capacity exceeds the profile"));
  }
  Ok(limits)
}
