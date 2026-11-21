use std::collections::BTreeSet;
use std::sync::{Mutex, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow::array::{Array, ArrayRef, Float32Array, Float64Array, RecordBatch, StringArray, TimestampMicrosecondArray, UInt64Array};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use duckdb::core::{DataChunkHandle, LogicalTypeHandle, LogicalTypeId};
use duckdb::vtab::{BindInfo, InitInfo, TableFunctionInfo, VTab};
use duckdb::vtab::arrow::{record_batch_to_duckdb_data_chunk, to_duckdb_logical_type_for_field};
use serde_json::Value;

use crate::dialect::duckdb::duckdb_sync::DuckDbSyncConnection;
use crate::studio::error::{StudioError, StudioErrorCode};
use crate::studio::model::{SeriesRow, format_utc_micros, parse_utc_micros};
use crate::studio::results::{PoolState, Publication, ResultLease, RetainedPool, error, validate_row};

pub(super) fn json_memory(value: &Value) -> u64 {
  let base = std::mem::size_of::<Value>() as u64;
  base + match value {
    Value::String(value) => value.capacity() as u64,
    Value::Array(values) => (values.capacity() - values.len()) as u64 * base + values.iter().map(json_memory).sum::<u64>(),
    Value::Object(values) => values.iter().map(|(key, value)| key.capacity() as u64 + 256 + json_memory(value)).sum(),
    Value::Number(value) => value.to_string().len() as u64,
    _ => 0,
  }
}

pub fn validate_publication(input: &Publication) -> Result<u64, StudioError> {
  if input.rows.is_empty() || input.rows.len() > 25_000 || input.snapshot.rows.is_empty() || input.snapshot.rows.len() > 100 {
    return Err(error(StudioErrorCode::InvalidRequest, "publication has invalid row or station counts"));
  }
  if input.snapshot.schema_version != 1 || [&input.snapshot.snapshot_hash, &input.snapshot.content_sha256].iter().any(|hash| hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())) {
    return Err(error(StudioErrorCode::InvalidRequest, "invalid frozen station snapshot identity"));
  }
  let mut keys = BTreeSet::new();
  let mut stations = BTreeSet::new();
  let mut bytes = 65_536_u64;
  for station in &input.snapshot.rows {
    if !stations.insert(station.station_id.as_str()) { return Err(error(StudioErrorCode::StationValidation, "duplicate snapshot station")); }
    bytes = bytes.saturating_add(station.station_id.len() as u64 * 3 + 512);
    for (key, value) in &station.business { bytes = bytes.saturating_add((key.len() + value.len()) as u64 * 12 + 256); }
  }
  for row in &input.rows {
    validate_row(row)?;
    let station = input.snapshot.rows.iter().find(|station| station.station_id == row.station_id)
      .ok_or_else(|| error(StudioErrorCode::StationValidation, "result station is absent from frozen snapshot"))?;
    if station.latitude != row.target_lat || station.longitude != row.target_lon
      || row.variable_id != input.rows[0].variable_id || row.unit != input.rows[0].unit
      || !keys.insert((&row.station_id, parse_utc_micros(&row.valid_time)?)) {
      return Err(error(StudioErrorCode::InvalidRequest, "result and station snapshot disagree or keys repeat"));
    }
    bytes = bytes.saturating_add(512 + [row.station_id.len(), row.valid_time.len(), row.variable_id.len(), row.unit.as_ref().map_or(0, String::len), row.source_point_index.len(), row.source_object_id.len(), row.quality.len()].iter().sum::<usize>() as u64 * 6);
  }
  if !input.provenance.is_object() { return Err(error(StudioErrorCode::InvalidRequest, "provenance must be an object")); }
  validate_provenance(&input.provenance, 0)?;
  let size = crate::studio::result_query::json_size(&input.provenance, 1024 * 1024)?;
  Ok(bytes.saturating_add(json_memory(&input.provenance).saturating_mul(4)).saturating_add(size as u64 * 6))
}

fn validate_provenance(value: &Value, depth: usize) -> Result<(), StudioError> {
  if depth > 24 { return Err(error(StudioErrorCode::InvalidRequest, "provenance nesting is too deep")); }
  match value {
    Value::Object(map) => for (key, value) in map {
      let key = key.to_ascii_lowercase();
      if ["password", "secret", "token", "passphrase"].iter().any(|part| key.contains(part)) {
        return Err(error(StudioErrorCode::InvalidRequest, "provenance must not contain credentials"));
      }
      validate_provenance(value, depth + 1)?;
    },
    Value::Array(values) => for value in values { validate_provenance(value, depth + 1)?; },
    Value::String(text) => if let Ok(url) = url::Url::parse(text) {
      if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err(error(StudioErrorCode::InvalidRequest, "source URIs must be redacted before publication"));
      }
    },
    _ => {},
  }
  Ok(())
}

pub fn batches(input: &Publication) -> Result<(RecordBatch, RecordBatch), StudioError> {
  let rows = &input.rows;
  let strings = |get: fn(&SeriesRow) -> &str| -> ArrayRef { std::sync::Arc::new(StringArray::from_iter_values(rows.iter().map(get))) };
  let doubles = |get: fn(&SeriesRow) -> f64| -> ArrayRef { std::sync::Arc::new(Float64Array::from_iter_values(rows.iter().map(get))) };
  let fields = vec![
    Field::new("station_id", DataType::Utf8, false),
    Field::new("valid_time", DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())), false),
    Field::new("variable_id", DataType::Utf8, false), Field::new("value", DataType::Float32, true),
    Field::new("unit", DataType::Utf8, true), Field::new("target_lat", DataType::Float64, false),
    Field::new("target_lon", DataType::Float64, false), Field::new("source_lat", DataType::Float64, false),
    Field::new("source_lon", DataType::Float64, false), Field::new("source_point_index", DataType::Utf8, false),
    Field::new("source_object_id", DataType::Utf8, false), Field::new("quality", DataType::Utf8, false),
  ];
  let times = rows.iter().map(|row| parse_utc_micros(&row.valid_time)).collect::<Result<Vec<_>, _>>()?;
  let batch = RecordBatch::try_new(std::sync::Arc::new(Schema::new(fields)), vec![
    strings(|row| &row.station_id), std::sync::Arc::new(TimestampMicrosecondArray::from(times).with_timezone("UTC")),
    strings(|row| &row.variable_id), std::sync::Arc::new(Float32Array::from(rows.iter().map(|row| row.value).collect::<Vec<_>>())),
    std::sync::Arc::new(StringArray::from(rows.iter().map(|row| row.unit.as_deref()).collect::<Vec<_>>())),
    doubles(|row| row.target_lat), doubles(|row| row.target_lon), doubles(|row| row.source_lat), doubles(|row| row.source_lon),
    strings(|row| &row.source_point_index), strings(|row| &row.source_object_id), strings(|row| &row.quality),
  ]).map_err(|_| error(StudioErrorCode::Internal, "result Arrow conversion failed"))?;
  let stations = &input.snapshot.rows;
  let business = stations.iter().map(|row| serde_json::to_string(&row.business)).collect::<Result<Vec<_>, _>>()
    .map_err(|_| error(StudioErrorCode::Internal, "station snapshot conversion failed"))?;
  let snapshot = RecordBatch::try_new(std::sync::Arc::new(Schema::new(vec![
    Field::new("station_id", DataType::Utf8, false), Field::new("row_ordinal", DataType::UInt64, false),
    Field::new("target_lat", DataType::Float64, false), Field::new("target_lon", DataType::Float64, false), Field::new("business_json", DataType::Utf8, false),
  ])), vec![
    std::sync::Arc::new(StringArray::from_iter_values(stations.iter().map(|row| row.station_id.as_str()))),
    std::sync::Arc::new(UInt64Array::from_iter_values(stations.iter().map(|row| row.row_ordinal))),
    std::sync::Arc::new(Float64Array::from_iter_values(stations.iter().map(|row| row.latitude))),
    std::sync::Arc::new(Float64Array::from_iter_values(stations.iter().map(|row| row.longitude))),
    std::sync::Arc::new(StringArray::from_iter_values(business.iter().map(String::as_str))),
  ]).map_err(|_| error(StudioErrorCode::Internal, "station Arrow conversion failed"))?;
  Ok((batch, snapshot))
}

pub fn row(batch: &RecordBatch, index: usize) -> SeriesRow {
  let string = |column: usize| batch.column(column).as_any().downcast_ref::<StringArray>().unwrap().value(index).to_string();
  let double = |column: usize| batch.column(column).as_any().downcast_ref::<Float64Array>().unwrap().value(index);
  let values = batch.column(3).as_any().downcast_ref::<Float32Array>().unwrap();
  let units = batch.column(4).as_any().downcast_ref::<StringArray>().unwrap();
  let time = batch.column(1).as_any().downcast_ref::<TimestampMicrosecondArray>().unwrap().value(index);
  SeriesRow { station_id: string(0), valid_time: format_utc_micros(time).expect("validated timestamp"), variable_id: string(2),
    value: (!values.is_null(index)).then(|| values.value(index)), unit: (!units.is_null(index)).then(|| units.value(index).to_string()),
    target_lat: double(5), target_lon: double(6), source_lat: double(7), source_lon: double(8), source_point_index: string(9), source_object_id: string(10), quality: string(11) }
}

pub fn business_json<'a>(batch: &'a RecordBatch, station_id: &str) -> &'a str {
  let ids = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
  let values = batch.column(4).as_any().downcast_ref::<StringArray>().unwrap();
  (0..ids.len()).find(|index| ids.value(*index) == station_id).map(|index| values.value(index)).unwrap_or("{}")
}

#[derive(Clone)]
struct Scope {
  pool: Weak<Mutex<PoolState>>,
  owner_id: String,
  generation: u64,
}

pub struct OwnedResultScan;

pub struct BoundResult {
  lease: ResultLease,
  stations: bool,
}

pub struct ScanState { offset: AtomicUsize }

impl VTab for OwnedResultScan {
  type BindData = BoundResult;
  type InitData = ScanState;

  fn bind(info: &BindInfo) -> Result<BoundResult, Box<dyn std::error::Error>> {
    let scope = unsafe { &*info.get_extra_info::<Scope>() };
    let state = scope.pool.upgrade().ok_or("result pool expired")?;
    let handle = info.get_parameter(0).to_string();
    let stations = info.get_parameter(1).to_string() == "true";
    let lease = RetainedPool::acquire_state(&state, &handle, &scope.owner_id, scope.generation, true)?;
    let batch = if stations { lease.stations() } else { lease.batch() };
    for field in batch.schema().fields() { info.add_result_column(field.name(), to_duckdb_logical_type_for_field(field)?); }
    info.set_cardinality(batch.num_rows() as u64, true);
    Ok(BoundResult { lease, stations })
  }

  fn init(_: &InitInfo) -> Result<ScanState, Box<dyn std::error::Error>> { Ok(ScanState { offset: AtomicUsize::new(0) }) }

  fn func(info: &TableFunctionInfo<Self>, output: &mut DataChunkHandle) -> Result<(), Box<dyn std::error::Error>> {
    let bound = info.get_bind_data();
    let batch = if bound.stations { bound.lease.stations() } else { bound.lease.batch() };
    let capacity = unsafe { duckdb::ffi::duckdb_vector_size() } as usize;
    let start = info.get_init_data().offset.fetch_add(capacity, Ordering::Relaxed);
    if start >= batch.num_rows() { output.set_len(0); return Ok(()); }
    record_batch_to_duckdb_data_chunk(&batch.slice(start, capacity.min(batch.num_rows() - start)), output)?;
    Ok(())
  }

  fn parameters() -> Option<Vec<LogicalTypeHandle>> {
    Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar), LogicalTypeHandle::from(LogicalTypeId::Boolean)])
  }
}

pub fn install(pool: &RetainedPool, connection: &DuckDbSyncConnection, handle: &str, owner_id: &str, generation: u64, alias: &str, station_alias: &str) -> Result<(u64, u64), StudioError> {
  let function = format!("studio_scan_{generation}");
  let registered: bool = connection.inner.query_row("SELECT count(*) > 0 FROM duckdb_functions() WHERE function_name=?", [&function], |row| row.get(0))
    .map_err(|_| error(StudioErrorCode::Internal, "cannot inspect result scan registration"))?;
  if !registered {
    connection.inner.register_table_function_with_extra_info::<OwnedResultScan, _>(&function, &Scope { pool: pool.weak(), owner_id: owner_id.into(), generation })
      .map_err(|_| error(StudioErrorCode::Internal, "cannot register owned result scan"))?;
  }
  let sql = format!("CREATE TEMP VIEW {alias} AS SELECT * FROM {function}('{handle}', false); CREATE TEMP VIEW {station_alias} AS SELECT * FROM {function}('{handle}', true);");
  if connection.inner.execute_batch(&sql).is_err() {
    let _ = connection.inner.execute_batch(&format!("DROP VIEW IF EXISTS {alias}; DROP VIEW IF EXISTS {station_alias};"));
    return Err(error(StudioErrorCode::Internal, "cannot bind local result views"));
  }
  Ok((view_identity(connection, alias)?, view_identity(connection, station_alias)?))
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::Arc;
  use crate::dialect::duckdb::session::NativeSession;
  use crate::studio::stations::{confirm_rows, LongitudeConvention, WhitespacePolicy};

  #[tokio::test]
  async fn prepared_scans_reset_the_cursor_and_rebind_only_live_results() {
    let owner = Arc::new(NativeSession::new_local(None, None));
    let session = Arc::clone(&owner);
    let generation = owner.snapshot().generation;
    owner.run(None, move |connection, _| {
      let columns = vec!["id".into(), "lat".into(), "lon".into(), "name".into()];
      let station = std::collections::BTreeMap::from([("id".into(), "001".into()), ("lat".into(), "10".into()), ("lon".into(), "20".into()), ("name".into(), "北京".into())]);
      let hash = "a".repeat(64);
      let snapshot = confirm_rows(&hash, &hash, &columns, &[station], "id", "lat", "lon", LongitudeConvention::Signed180, WhitespacePolicy::Preserve, &BTreeSet::new(), &[0])?;
      let row = SeriesRow { station_id: "001".into(), valid_time: "2026-11-10T00:00:00Z".into(), variable_id: "test".into(), value: Some(1.5), unit: None, target_lat: 10.0, target_lon: 20.0, source_lat: 10.0, source_lon: 20.0, source_point_index: "0".into(), source_object_id: "synthetic".into(), quality: "present".into() };
      let pool = RetainedPool::new();
      let manifest = pool.publish(pool.reserve(1024 * 1024)?, Publication { session, owner_id: "owner".into(), generation, rows: vec![row], snapshot, provenance: serde_json::json!({"synthetic":true}) }, connection)?;
      let mut prepared = connection.inner.prepare(&format!("SELECT value::FLOAT FROM {}", manifest.alias))?;
      for _ in 0..2 { assert_eq!(prepared.query_row([], |row| row.get::<_, f32>(0))?, 1.5); }
      pool.close(&manifest.handle, "owner", generation)?;
      assert_eq!(pool.charged_bytes(), 0);
      assert!(prepared.query_row([], |row| row.get::<_, f32>(0)).is_err());
      assert!(pool.acquire(&manifest.handle, "owner", generation).is_err());
      drop(prepared);
      connection.inner.execute_batch("SELECT 1")?;
      assert_eq!(pool.charged_bytes(), 0);
      Ok(())
    }).await.unwrap();
  }
}

pub fn view_identity(connection: &DuckDbSyncConnection, name: &str) -> Result<u64, StudioError> {
  connection.inner.query_row("SELECT view_oid FROM duckdb_views() WHERE temporary AND view_name=?", [name], |row| row.get(0))
    .map_err(|_| error(StudioErrorCode::InvalidRequest, "result SQL alias was replaced or removed"))
}
