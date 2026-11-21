use std::{
    fmt::Display,
    fs,
    path::{Component, Path, PathBuf},
};

use arrow::{datatypes::SchemaRef, record_batch::RecordBatch};
use arrow_ipc::writer::FileWriter;
use omo_protocol::result_ref::{ResultLimits, ResultRef, schema_hash_from_debug};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::limits::{LimitError, validate_limits};

#[derive(Debug, Error)]
pub enum ResultWriteError {
    #[error(transparent)]
    Limits(#[from] LimitError),
    #[error("invalid result id `{0}`")]
    InvalidId(String),
    #[error("query stream failed: {0}")]
    Query(String),
    #[error("Arrow IPC write failed: {0}")]
    Arrow(String),
    #[error("result file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("the byte limit is too small to hold the Arrow IPC schema and footer")]
    LimitTooSmall,
}

/// Stream Arrow batches to a generation-scoped IPC file. At most one DuckDB batch and one
/// truncated slice are live here; rows are never collected into a result-sized vector.
pub fn write_arrow_batches<I, E>(
    schema: SchemaRef,
    batches: I,
    session_tmp: &Path,
    id: &str,
    generation: u64,
    limits: ResultLimits,
) -> Result<ResultRef, ResultWriteError>
where
    I: IntoIterator<Item = Result<RecordBatch, E>>,
    E: Display,
{
    validate_limits(limits)?;
    validate_id(id)?;
    let generation_dir = session_tmp.join(format!("g{generation}"));
    fs::create_dir_all(&generation_dir)?;
    let final_path = generation_dir.join(format!("{id}.arrow"));
    let mut staged = NamedTempFile::new_in(&generation_dir)?;
    let mut writer = FileWriter::try_new(staged.as_file_mut(), schema.as_ref())
        .map_err(|error| ResultWriteError::Arrow(error.to_string()))?;

    let base_overhead = 512u64.saturating_add(schema.fields().len() as u64 * 128);
    let batch_overhead = 1024u64.saturating_add(schema.fields().len() as u64 * 256);
    let mut estimated_bytes = base_overhead;
    let mut rows = 0u64;
    let mut truncated = false;

    for batch in batches {
        let batch = batch.map_err(|error| ResultWriteError::Query(error.to_string()))?;
        if batch.num_rows() == 0 {
            continue;
        }
        if rows == limits.rows {
            truncated = true;
            break;
        }
        let rows_left = limits.rows - rows;
        let bytes_left = limits.bytes.saturating_sub(estimated_bytes);
        let batch_rows = batch.num_rows();
        let take = largest_prefix_fitting(&batch, rows_left, bytes_left, batch_overhead);
        if take == 0 {
            truncated = true;
            break;
        }
        let output = if take == batch.num_rows() {
            batch
        } else {
            batch.slice(0, take)
        };
        estimated_bytes = estimated_bytes
            .saturating_add(batch_overhead)
            .saturating_add(output.get_array_memory_size() as u64);
        writer
            .write(&output)
            .map_err(|error| ResultWriteError::Arrow(error.to_string()))?;
        rows += take as u64;
        if take < batch_rows {
            truncated = true;
            break;
        }
    }

    writer
        .finish()
        .map_err(|error| ResultWriteError::Arrow(error.to_string()))?;
    drop(writer);
    staged.as_file().sync_all()?;
    let actual_bytes = staged.as_file().metadata()?.len();
    if actual_bytes > limits.bytes {
        return Err(ResultWriteError::LimitTooSmall);
    }
    staged
        .persist(&final_path)
        .map_err(|error| ResultWriteError::Io(error.error))?;

    let schema_hash = schema_hash_from_debug(&format!("{schema:#?}"));
    Ok(ResultRef {
        id: id.to_owned(),
        generation,
        path: final_path,
        rows,
        schema_hash,
        truncated,
        limits,
    })
}

fn largest_prefix_fitting(
    batch: &RecordBatch,
    rows_left: u64,
    bytes_left: u64,
    overhead: u64,
) -> usize {
    let high = batch
        .num_rows()
        .min(usize::try_from(rows_left).unwrap_or(usize::MAX));
    let mut low = 0usize;
    let mut high = high;
    while low < high {
        let middle = low + (high - low + 1) / 2;
        let slice = batch.slice(0, middle);
        let estimate = overhead.saturating_add(slice.get_array_memory_size() as u64);
        if estimate <= bytes_left {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    low
}

pub(crate) fn validate_id(id: &str) -> Result<(), ResultWriteError> {
    let mut components = Path::new(id).components();
    if id.is_empty()
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(ResultWriteError::InvalidId(id.to_owned()));
    }
    Ok(())
}

pub fn result_path(session_tmp: &Path, generation: u64, id: &str) -> PathBuf {
    session_tmp
        .join(format!("g{generation}"))
        .join(format!("{id}.arrow"))
}
