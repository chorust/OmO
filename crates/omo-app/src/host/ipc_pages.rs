use std::{fs::File, io::Cursor, ops::Range, path::Path};

use arrow::{
    array::RecordBatch, datatypes::DataType, error::ArrowError,
    util::display::array_value_to_string,
};
use arrow_ipc::reader::FileReader;
use memmap2::{Mmap, MmapOptions};
use thiserror::Error;

pub const MAX_PAGE_ROWS: u64 = 1_000;
pub use omo_protocol::plot::{MAX_PLOT_POINTS, MAX_PLOT_SERIES};

#[derive(Debug, Error)]
pub enum IpcPageError {
    #[error("IPC result file I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Arrow IPC file is invalid: {0}")]
    Arrow(#[from] ArrowError),
    #[error("page limit {requested} exceeds maximum {maximum}")]
    PageLimit { requested: u64, maximum: u64 },
    #[error("plot sample limit {requested} exceeds maximum {maximum}")]
    SampleLimit { requested: usize, maximum: usize },
}

pub struct IpcPageReader {
    _file: File,
    map: Mmap,
    batch_ranges: Vec<Range<u64>>,
    total_rows: u64,
}

pub struct ResultPage {
    pub start_row: u64,
    pub rows: u64,
    pub batches: Vec<RecordBatch>,
}

pub struct NumericSample {
    pub total_rows: u64,
    pub sampled_rows: usize,
    pub columns: Vec<(String, Vec<f64>)>,
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

impl IpcPageReader {
    /// Maps an immutable session result file and indexes batch row ranges.
    ///
    /// The worker must never mutate a published result file. Keeping the file descriptor open
    /// for the lifetime of the mapping prevents path replacement from invalidating the map.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, IpcPageError> {
        let file = File::open(path)?;
        // SAFETY: ResultRef files are immutable after publication and `_file` remains open while
        // the mapping is held. The worker writes to a separate staging file before publication.
        let map = unsafe { MmapOptions::new().map(&file)? };
        let mut reader = FileReader::try_new(Cursor::new(&map[..]), None)?;
        let mut batch_ranges = Vec::with_capacity(reader.num_batches());
        let mut row = 0u64;
        for batch in &mut reader {
            let batch = batch?;
            let end = row.saturating_add(batch.num_rows() as u64);
            batch_ranges.push(row..end);
            row = end;
        }
        Ok(Self {
            _file: file,
            map,
            batch_ranges,
            total_rows: row,
        })
    }

    pub fn total_rows(&self) -> u64 {
        self.total_rows
    }

    /// Samples at most the plot contract's point and numeric-series limits while scanning each
    /// Arrow batch once. Sample positions are evenly distributed over the complete result.
    pub fn read_numeric_sample(
        &self,
        max_points: usize,
        max_series: usize,
    ) -> Result<NumericSample, IpcPageError> {
        if max_points > MAX_PLOT_POINTS {
            return Err(IpcPageError::SampleLimit {
                requested: max_points,
                maximum: MAX_PLOT_POINTS,
            });
        }
        if max_series > MAX_PLOT_SERIES {
            return Err(IpcPageError::SampleLimit {
                requested: max_series,
                maximum: MAX_PLOT_SERIES,
            });
        }
        if max_points == 0 || max_series == 0 || self.total_rows == 0 {
            return Ok(NumericSample {
                total_rows: self.total_rows,
                sampled_rows: 0,
                columns: Vec::new(),
            });
        }

        let sample_count = self.total_rows.min(max_points as u64) as usize;
        let mut reader = FileReader::try_new(Cursor::new(&self.map[..]), None)?;
        let mut columns: Option<Vec<(usize, String, Vec<f64>)>> = None;
        let mut sampled_rows = 0usize;
        for (batch_index, range) in self.batch_ranges.iter().enumerate() {
            if range.is_empty() {
                continue;
            }
            reader.set_index(batch_index)?;
            let Some(batch) = reader.next().transpose()? else {
                continue;
            };
            let selected = columns.get_or_insert_with(|| {
                batch
                    .schema()
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
                    .collect()
            });
            for local_row in 0..batch.num_rows() {
                let global_row = range.start + local_row as u64;
                let desired_row = if sample_count == 1 {
                    (self.total_rows - 1) / 2
                } else {
                    (sampled_rows as u128 * (self.total_rows - 1) as u128
                        / (sample_count - 1) as u128) as u64
                };
                if global_row != desired_row {
                    continue;
                }
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
            if sampled_rows == sample_count {
                break;
            }
        }
        Ok(NumericSample {
            total_rows: self.total_rows,
            sampled_rows,
            columns: columns
                .unwrap_or_default()
                .into_iter()
                .map(|(_, name, values)| (name, values))
                .collect(),
        })
    }

    /// Reads only the batches intersecting a visible row range; returned batches are slices and
    /// are not concatenated into one in-memory result.
    pub fn read_rows(&self, start: u64, limit: u64) -> Result<ResultPage, IpcPageError> {
        if limit > MAX_PAGE_ROWS {
            return Err(IpcPageError::PageLimit {
                requested: limit,
                maximum: MAX_PAGE_ROWS,
            });
        }
        if limit == 0 || start >= self.total_rows {
            return Ok(ResultPage {
                start_row: start.min(self.total_rows),
                rows: 0,
                batches: Vec::new(),
            });
        }

        let end = start.saturating_add(limit).min(self.total_rows);
        let mut reader = FileReader::try_new(Cursor::new(&self.map[..]), None)?;
        let mut page_batches = Vec::new();
        let mut rows = 0u64;
        for (batch_index, range) in self.batch_ranges.iter().enumerate() {
            if range.end <= start || range.start >= end || range.is_empty() {
                continue;
            }
            reader.set_index(batch_index)?;
            let batch = reader
                .next()
                .transpose()?
                .expect("indexed Arrow IPC batch must exist");
            let local_start = start.saturating_sub(range.start) as usize;
            let local_end = (end.min(range.end) - range.start) as usize;
            let slice = batch.slice(local_start, local_end - local_start);
            rows += slice.num_rows() as u64;
            page_batches.push(slice);
        }

        Ok(ResultPage {
            start_row: start,
            rows,
            batches: page_batches,
        })
    }
}
