use std::{fs::File, sync::Arc};

use arrow::{
    array::{ArrayRef, Float64Array, Int64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use arrow_ipc::reader::FileReader;
use omo_engine::{plot::create_plot_artifact, results::write_arrow_batches};
use omo_protocol::{
    plot::{MAX_PLOT_POINTS, MAX_PLOT_SERIES},
    result_ref::ResultLimits,
};

fn batch(rows: usize, text_len: usize) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("text", DataType::Utf8, false),
    ]));
    let ids = Int64Array::from_iter_values(0..rows as i64);
    let values = StringArray::from_iter_values(
        (0..rows).map(|index| format!("{index}:{}", "x".repeat(text_len))),
    );
    RecordBatch::try_new(schema, vec![Arc::new(ids), Arc::new(values)]).unwrap()
}

#[test]
fn writer_streams_batches_and_truncates_at_the_row_limit() {
    let temp = tempfile::tempdir().unwrap();
    let first = batch(7, 8);
    let schema = first.schema();
    let second = batch(7, 8);
    let result = write_arrow_batches(
        schema,
        [Ok::<_, String>(first), Ok(second)].into_iter(),
        temp.path(),
        "preview-1",
        4,
        ResultLimits {
            rows: 5,
            bytes: 64 * 1024,
        },
    )
    .unwrap();

    assert_eq!(result.generation, 4);
    assert_eq!(result.rows, 5);
    assert!(result.truncated);
    assert_eq!(result.path, temp.path().join("g4/preview-1.arrow"));
    let reader = FileReader::try_new(File::open(&result.path).unwrap(), None).unwrap();
    assert_eq!(
        reader.map(|batch| batch.unwrap().num_rows()).sum::<usize>(),
        5
    );
}

#[test]
fn writer_respects_the_byte_limit_without_collecting_the_result() {
    let temp = tempfile::tempdir().unwrap();
    let first = batch(16, 512);
    let schema = first.schema();
    let result = write_arrow_batches(
        schema,
        [Ok::<_, String>(first)].into_iter(),
        temp.path(),
        "preview-2",
        0,
        ResultLimits {
            rows: 100,
            bytes: 5_000,
        },
    )
    .unwrap();

    assert!(result.truncated);
    assert!(result.rows < 16);
    assert!(std::fs::metadata(&result.path).unwrap().len() <= 5_000);
}

#[test]
fn worker_plot_artifact_is_evenly_sampled_and_does_not_replace_the_full_result() {
    let temp = tempfile::tempdir().unwrap();
    let row_count = 20_000usize;
    let fields = (0..22)
        .map(|series| Field::new(format!("series_{series:02}"), DataType::Float64, false))
        .collect::<Vec<_>>();
    let schema = Arc::new(Schema::new(fields));
    let arrays = (0..22)
        .map(|series| {
            Arc::new(Float64Array::from_iter_values(
                (0..row_count).map(|row| (series * row_count + row) as f64),
            )) as ArrayRef
        })
        .collect::<Vec<_>>();
    let source_batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
    let source = write_arrow_batches(
        schema,
        [Ok::<_, String>(source_batch)].into_iter(),
        temp.path(),
        "preview-plot-test",
        7,
        ResultLimits {
            rows: row_count as u64,
            bytes: 64 * 1024 * 1024,
        },
    )
    .unwrap();

    let plot = create_plot_artifact(&source, temp.path()).unwrap();
    let plot_ref = plot.reference.unwrap();
    assert_eq!(plot.source_rows, row_count as u64);
    assert_eq!(plot_ref.generation, source.generation);
    assert_eq!(plot_ref.rows, MAX_PLOT_POINTS as u64);
    assert!(plot_ref.truncated);
    assert_eq!(source.rows, row_count as u64);
    assert!(source.path.exists());

    let mut reader = FileReader::try_new(File::open(&plot_ref.path).unwrap(), None).unwrap();
    let sampled = reader.next().unwrap().unwrap();
    assert_eq!(sampled.num_rows(), MAX_PLOT_POINTS);
    assert_eq!(sampled.num_columns(), MAX_PLOT_SERIES);
    assert_eq!(sampled.schema().field(0).name(), "series_00");
    assert_eq!(
        sampled.schema().field(MAX_PLOT_SERIES - 1).name(),
        "series_19"
    );
    let values = sampled
        .column(0)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    assert_eq!(values.value(0), 0.0);
    assert_eq!(values.value(1), 2.0);
    assert_eq!(values.value(MAX_PLOT_POINTS - 1), (row_count - 1) as f64);
    assert!(reader.next().is_none());
}
