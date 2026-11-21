use std::{fs::File, sync::Arc};

use arrow::{
    array::{ArrayRef, Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use arrow_ipc::writer::FileWriter;
use omo_app::host::ipc_pages::{IpcPageReader, MAX_PLOT_POINTS, MAX_PLOT_SERIES};
use tempfile::tempdir;

#[test]
fn plot_sample_is_evenly_distributed_and_bounded_over_the_complete_result() {
    let row_count = 20_000usize;
    let fields = (0..22)
        .map(|series| Field::new(format!("series_{series:02}"), DataType::Float64, false))
        .chain([Field::new("label", DataType::Utf8, false)])
        .collect::<Vec<_>>();
    let schema = Arc::new(Schema::new(fields));
    let arrays = (0..22)
        .map(|series| {
            Arc::new(Float64Array::from_iter_values(
                (0..row_count).map(|row| (series * row_count + row) as f64),
            )) as ArrayRef
        })
        .chain([Arc::new(StringArray::from_iter_values((0..row_count).map(|_| "x"))) as ArrayRef])
        .collect::<Vec<_>>();
    let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
    let dir = tempdir().unwrap();
    let path = dir.path().join("result.arrow");
    let mut writer = FileWriter::try_new(File::create(&path).unwrap(), &schema).unwrap();
    writer.write(&batch.slice(0, row_count / 2)).unwrap();
    writer
        .write(&batch.slice(row_count / 2, row_count - row_count / 2))
        .unwrap();
    writer.finish().unwrap();

    let reader = IpcPageReader::open(path).unwrap();
    let sample = reader
        .read_numeric_sample(MAX_PLOT_POINTS, MAX_PLOT_SERIES)
        .unwrap();

    assert_eq!(sample.total_rows, row_count as u64);
    assert_eq!(sample.sampled_rows, MAX_PLOT_POINTS);
    assert_eq!(sample.columns.len(), MAX_PLOT_SERIES);
    assert_eq!(sample.columns[0].0, "series_00");
    assert_eq!(sample.columns[MAX_PLOT_SERIES - 1].0, "series_19");
    assert_eq!(sample.columns[0].1.len(), MAX_PLOT_POINTS);
    assert_eq!(sample.columns[0].1[0], 0.0);
    assert_eq!(sample.columns[0].1[1], 2.0);
    assert_eq!(
        sample.columns[0].1[MAX_PLOT_POINTS - 1],
        (row_count - 1) as f64
    );
    assert_eq!(
        sample.columns[MAX_PLOT_SERIES - 1].1[MAX_PLOT_POINTS - 1],
        (19 * row_count + row_count - 1) as f64
    );
}
