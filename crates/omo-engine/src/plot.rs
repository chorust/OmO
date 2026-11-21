//! Worker-side downsampling for bounded plot previews.

use std::{fs, path::Path, sync::Arc};

use arrow::{
    array::{ArrayRef, Float64Array},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
    util::display::array_value_to_string,
};
use arrow_ipc::{reader::FileReader, writer::FileWriter};
use omo_protocol::{
    plot::{MAX_PLOT_POINTS, MAX_PLOT_SERIES},
    result_ref::{ResultLimits, ResultRef, schema_hash_from_debug},
};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::results::{ResultWriteError, result_path, validate_id};

#[derive(Debug, Error)]
pub enum PlotSampleError {
    #[error(transparent)]
    ResultWrite(#[from] ResultWriteError),
    #[error("plot result file I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("plot result Arrow IPC failed: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
}

#[derive(Debug)]
pub struct PlotArtifact {
    pub reference: Option<ResultRef>,
    pub source_rows: u64,
}

struct NumericSample {
    sampled_rows: usize,
    columns: Vec<(String, Vec<f64>)>,
}

/// Read a completed query result, retain an evenly spaced numeric sample, and write that sample
/// as a small generation-scoped Arrow IPC artifact. The full result remains intact for table pages.
pub fn create_plot_artifact(
    source: &ResultRef,
    result_root: &Path,
) -> Result<PlotArtifact, PlotSampleError> {
    validate_id(&source.id)?;
    let sample = read_numeric_sample(&source.path, MAX_PLOT_POINTS, MAX_PLOT_SERIES)?;
    if sample.columns.is_empty() || sample.sampled_rows == 0 {
        return Ok(PlotArtifact {
            reference: None,
            source_rows: source.rows,
        });
    }

    let schema = Schema::new(
        sample
            .columns
            .iter()
            .map(|(name, _)| Field::new(name, DataType::Float64, false))
            .collect::<Vec<_>>(),
    );
    let arrays = sample
        .columns
        .into_iter()
        .map(|(_, values)| Arc::new(Float64Array::from(values)) as ArrayRef)
        .collect::<Vec<_>>();
    let batch = RecordBatch::try_new(Arc::new(schema.clone()), arrays)
        .map_err(|error| ResultWriteError::Arrow(error.to_string()))?;

    let plot_id = format!("{}-plot", source.id);
    let final_path = result_path(result_root, source.generation, &plot_id);
    let parent = final_path
        .parent()
        .expect("generation-scoped result path has a parent");
    fs::create_dir_all(parent)?;
    let mut staged = NamedTempFile::new_in(parent)?;
    let mut writer = FileWriter::try_new(staged.as_file_mut(), &schema)
        .map_err(|error| ResultWriteError::Arrow(error.to_string()))?;
    writer
        .write(&batch)
        .map_err(|error| ResultWriteError::Arrow(error.to_string()))?;
    writer
        .finish()
        .map_err(|error| ResultWriteError::Arrow(error.to_string()))?;
    drop(writer);
    staged.as_file().sync_all()?;
    let bytes = staged.as_file().metadata()?.len();
    staged
        .persist(&final_path)
        .map_err(|error| ResultWriteError::Io(error.error))?;

    let reference = ResultRef {
        id: plot_id,
        generation: source.generation,
        path: final_path,
        rows: batch.num_rows() as u64,
        schema_hash: schema_hash(&schema),
        truncated: source.rows > batch.num_rows() as u64,
        limits: ResultLimits {
            rows: MAX_PLOT_POINTS as u64,
            bytes,
        },
    };
    Ok(PlotArtifact {
        reference: Some(reference),
        source_rows: source.rows,
    })
}

fn read_numeric_sample(
    path: &Path,
    max_points: usize,
    max_series: usize,
) -> Result<NumericSample, PlotSampleError> {
    let mut count_reader = FileReader::try_new(fs::File::open(path)?, None)?;
    let schema = count_reader.schema();
    let mut total_rows = 0u64;
    for batch in &mut count_reader {
        total_rows = total_rows.saturating_add(batch?.num_rows() as u64);
    }
    let sample_count = total_rows.min(max_points as u64) as usize;
    if sample_count == 0 {
        return Ok(NumericSample {
            sampled_rows: 0,
            columns: Vec::new(),
        });
    }

    let mut selected = schema
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, field)| is_numeric(field.data_type()))
        .take(max_series)
        .map(|(index, field)| {
            (
                index,
                field.name().to_owned(),
                Vec::with_capacity(sample_count),
            )
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok(NumericSample {
            sampled_rows: 0,
            columns: Vec::new(),
        });
    }

    let mut reader = FileReader::try_new(fs::File::open(path)?, None)?;
    let mut global_row = 0u64;
    let mut sampled_rows = 0usize;
    for batch in &mut reader {
        let batch = batch?;
        for local_row in 0..batch.num_rows() {
            let desired_row = if sample_count == 1 {
                (total_rows - 1) / 2
            } else {
                (sampled_rows as u128 * (total_rows - 1) as u128 / (sample_count - 1) as u128)
                    as u64
            };
            if global_row == desired_row {
                for (column_index, _, values) in selected.iter_mut() {
                    let value =
                        array_value_to_string(batch.column(*column_index).as_ref(), local_row)
                            .unwrap_or_default()
                            .parse::<f64>()
                            .unwrap_or(f64::NAN);
                    values.push(value);
                }
                sampled_rows += 1;
                if sampled_rows == sample_count {
                    break;
                }
            }
            global_row = global_row.saturating_add(1);
        }
        if sampled_rows == sample_count {
            break;
        }
    }

    Ok(NumericSample {
        sampled_rows,
        columns: selected
            .into_iter()
            .map(|(_, name, values)| (name, values))
            .collect(),
    })
}

fn is_numeric(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float16
            | DataType::Float32
            | DataType::Float64
            | DataType::Decimal128(_, _)
            | DataType::Decimal256(_, _)
    )
}

fn schema_hash(schema: &Schema) -> String {
    schema_hash_from_debug(&format!("{schema:#?}"))
}
