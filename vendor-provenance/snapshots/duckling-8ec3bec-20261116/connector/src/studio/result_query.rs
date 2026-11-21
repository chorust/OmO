use std::cmp::Ordering;
use std::io::{self, Write};

use arrow::array::{Float32Array, RecordBatch, StringArray, TimestampMicrosecondArray, UInt32Array, Array};
use serde::{Deserialize, Serialize};

use crate::studio::error::{StudioError, StudioErrorCode};
use crate::studio::results::{LocalOperation, ResultLease, ResultManifest, RetainedPool, error};

pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
pub const OPERATION_RESERVATION: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResultSort { #[default] Time, Station, Value }

pub struct ResultPage {
  pub manifest: ResultManifest,
  pub offset: usize,
  pub total: usize,
  pub batch: RecordBatch,
  pub alias_matches: bool,
  _lease: ResultLease,
  _operation: LocalOperation,
}

pub fn page_rows(
  pool: &RetainedPool, handle: &str, owner_id: &str, generation: u64,
  offset: usize, limit: usize, station_filter: Option<&str>,
) -> Result<ResultPage, StudioError> {
  sorted_page(pool, handle, owner_id, generation, offset, limit, station_filter, ResultSort::Time, false)
}

#[allow(clippy::too_many_arguments)]
pub fn sorted_page(
  pool: &RetainedPool, handle: &str, owner_id: &str, generation: u64,
  offset: usize, limit: usize, station_filter: Option<&str>, sort: ResultSort, descending: bool,
) -> Result<ResultPage, StudioError> {
  let lease = pool.acquire(handle, owner_id, generation)?;
  let operation = reserve_page(pool, &lease)?;
  project_page(lease, operation, offset, limit, station_filter, sort, descending)
}

pub(super) fn reserve_page(pool: &RetainedPool, lease: &ResultLease) -> Result<LocalOperation, StudioError> {
  let reservation = OPERATION_RESERVATION.max(lease.manifest_memory_bytes().saturating_mul(2).saturating_add(MAX_RESPONSE_BYTES as u64 * 2 + 1024 * 1024));
  pool.begin_operation(reservation)
}

pub(super) fn project_page(
  lease: ResultLease, operation: LocalOperation, offset: usize, limit: usize,
  station_filter: Option<&str>, sort: ResultSort, descending: bool,
) -> Result<ResultPage, StudioError> {
  if limit == 0 || limit > 1000 || offset > 25_000 {
    return Err(error(StudioErrorCode::ResultPageTooLarge, "page size must be 1..1000 and offset 0..25000"));
  }
  let batch = lease.batch();
  let ids = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
  let times = batch.column(1).as_any().downcast_ref::<TimestampMicrosecondArray>().unwrap();
  let values = batch.column(3).as_any().downcast_ref::<Float32Array>().unwrap();
  let mut indices: Vec<u32> = (0..batch.num_rows()).filter(|index| station_filter.is_none_or(|station| ids.value(*index) == station)).map(|index| index as u32).collect();
  indices.sort_by(|left, right| {
    let (left, right) = (*left as usize, *right as usize);
    if matches!(sort, ResultSort::Value) && values.is_null(left) != values.is_null(right) {
      return if values.is_null(left) { Ordering::Greater } else { Ordering::Less };
    }
    let order = match sort {
      ResultSort::Time => times.value(left).cmp(&times.value(right)),
      ResultSort::Station => ids.value(left).cmp(ids.value(right)).then_with(|| times.value(left).cmp(&times.value(right))),
      ResultSort::Value => match (values.is_null(left), values.is_null(right)) {
        (true, true) => Ordering::Equal, (true, false) => Ordering::Greater, (false, true) => Ordering::Less,
        _ => values.value(left).partial_cmp(&values.value(right)).unwrap_or(Ordering::Equal),
      },
    };
    let order = if descending { order.reverse() } else { order };
    order.then_with(|| left.cmp(&right))
  });
  let total = indices.len();
  let selected = &indices[offset.min(total)..(offset.saturating_add(limit)).min(total)];
  let mut estimated = 0_usize;
  for index in selected {
    for column in batch.columns() {
      estimated = estimated.saturating_add(column.as_any().downcast_ref::<StringArray>().map_or(16, |array| array.value(*index as usize).len().saturating_mul(6) + 32));
    }
  }
  if estimated > MAX_RESPONSE_BYTES { return Err(error(StudioErrorCode::ResultPageTooLarge, "selected page is too wide; reduce the page size")); }
  let selected = UInt32Array::from(selected.to_vec());
  let columns = batch.columns().iter().map(|column| arrow::compute::take(column.as_ref(), &selected, None)).collect::<Result<Vec<_>, _>>()
    .map_err(|_| error(StudioErrorCode::Internal, "local result projection failed"))?;
  let projected = RecordBatch::try_new(batch.schema(), columns).map_err(|_| error(StudioErrorCode::Internal, "local result projection schema failed"))?;
  Ok(ResultPage { manifest: lease.manifest().clone(), offset, total, batch: projected, alias_matches: true, _lease: lease, _operation: operation })
}

impl ResultPage {
  pub fn json_rows(&self) -> Vec<serde_json::Map<String, serde_json::Value>> {
    (0..self.batch.num_rows()).map(|index| {
      let row = crate::studio::result_relation::row(&self.batch, index);
      let serde_json::Value::Object(object) = serde_json::json!({
        "station_id": row.station_id, "valid_time": row.valid_time, "variable_id": row.variable_id,
        "value": row.value, "unit": row.unit, "target_lat": row.target_lat, "target_lon": row.target_lon,
        "source_lat": row.source_lat, "source_lon": row.source_lon, "source_point_index": row.source_point_index,
        "source_object_id": row.source_object_id, "quality": row.quality,
      }) else { unreachable!() };
      object
    }).collect()
  }
}

pub struct LimitedWriter<W> { inner: W, bytes: usize, limit: usize }

impl<W> LimitedWriter<W> {
  pub fn new(inner: W, limit: usize) -> Self { Self { inner, bytes: 0, limit } }
  pub fn into_inner(self) -> W { self.inner }
}

impl<W: Write> Write for LimitedWriter<W> {
  fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
    if bytes.len() > self.limit.saturating_sub(self.bytes) { return Err(io::Error::new(io::ErrorKind::FileTooLarge, "result serialization exceeds its byte limit")); }
    let written = self.inner.write(bytes)?;
    self.bytes += written;
    Ok(written)
  }
  fn flush(&mut self) -> io::Result<()> { self.inner.flush() }
}

pub fn arrow_bytes(batch: &RecordBatch) -> Result<Option<Vec<u8>>, StudioError> {
  let mut output = LimitedWriter::new(Vec::new(), MAX_RESPONSE_BYTES);
  let serialized = (|| -> Result<(), arrow::error::ArrowError> {
    let mut writer = arrow::ipc::writer::StreamWriter::try_new(&mut output, batch.schema().as_ref())?;
    writer.write(batch)?;
    writer.finish()
  })();
  match serialized {
    Ok(()) => Ok(Some(output.into_inner())),
    Err(arrow::error::ArrowError::IoError(_, io_error)) if io_error.kind() == io::ErrorKind::FileTooLarge => Err(error(StudioErrorCode::ResultPageTooLarge, "Arrow page exceeds its byte limit")),
    Err(_) => Ok(None),
  }
}

pub fn json_size(value: &impl Serialize, limit: usize) -> Result<usize, StudioError> {
  let mut writer = LimitedWriter::new(io::sink(), limit);
  serde_json::to_writer(&mut writer, value).map_err(|_| error(StudioErrorCode::ResultPageTooLarge, "JSON response exceeds its byte limit"))?;
  Ok(writer.bytes)
}

pub fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>, StudioError> {
  let mut writer = LimitedWriter::new(Vec::new(), MAX_RESPONSE_BYTES);
  serde_json::to_writer(&mut writer, value).map_err(|_| error(StudioErrorCode::ResultPageTooLarge, "JSON response exceeds 8 MiB; reduce the page size"))?;
  Ok(writer.into_inner())
}

pub fn local_sql_example(alias: &str, station_alias: &str) -> Result<String, StudioError> {
  if [alias, station_alias].iter().any(|name| !name.starts_with("studio_") || name.len() > 80 || !name.chars().all(|value| value.is_ascii_alphanumeric() || value == '_')) {
    return Err(error(StudioErrorCode::InvalidRequest, "invalid studio SQL alias"));
  }
  Ok(format!("SELECT r.*, s.business_json FROM {alias} r JOIN {station_alias} s USING (station_id) ORDER BY r.station_id, r.valid_time"))
}
