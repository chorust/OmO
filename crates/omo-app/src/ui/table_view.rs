//! Paged result table. Result files are mapped and decoded off the UI
//! engine thread; a page is published only if both its ResultRef and request
//! ticket are still current.

use std::rc::Rc;

use arrow::util::display::array_value_to_string;
use nsh_base::{DataTableColumn, DataTableRange, DataTableRows, DataTableSort, data_table};
use nsh_core::prelude::*;
use omo_protocol::result_ref::ResultRef;

use crate::host::ipc_pages::{IpcPageReader, MAX_PAGE_ROWS, ResultPage};

const PAGE_OVERSCAN_ROWS: u64 = 48;

struct DecodedPage {
    start_row: u64,
    row_count: u64,
    columns: Vec<(String, Vec<String>)>,
}

impl DecodedPage {
    fn empty(start_row: u64) -> Self {
        Self {
            start_row,
            row_count: 0,
            columns: Vec::new(),
        }
    }
}

fn decode_page(page: ResultPage) -> DecodedPage {
    let Some(schema) = page.batches.first().map(|batch| batch.schema()) else {
        return DecodedPage::empty(page.start_row);
    };

    let mut columns = Vec::with_capacity(schema.fields().len());
    for (column_index, field) in schema.fields().iter().enumerate() {
        let mut display_values = Vec::with_capacity(page.rows as usize);
        for batch in &page.batches {
            let array = batch.column(column_index);
            for row in 0..batch.num_rows() {
                let value = array_value_to_string(array.as_ref(), row).unwrap_or_default();
                display_values.push(value);
            }
        }
        columns.push((field.name().to_owned(), display_values));
    }

    DecodedPage {
        start_row: page.start_row,
        row_count: page.rows,
        columns,
    }
}

fn columns_of(page: &DecodedPage) -> Vec<DataTableColumn> {
    page.columns
        .iter()
        .map(|(name, _)| {
            let mut column = DataTableColumn::default();
            column.key = name.clone().into();
            column.title = Some(name.clone().into());
            column.sortable = true;
            column
        })
        .collect()
}

fn table_rows(page: &DecodedPage) -> DataTableRows {
    page.columns.iter().fold(
        DataTableRows::new(page.row_count as usize),
        |rows, (key, values)| rows.texts(key, values.iter()),
    )
}

fn truncation_label(reference: &ResultRef) -> Option<String> {
    reference.truncated.then(|| {
        format!(
            "TRUNCATED (limit: {} rows / {} B)",
            reference.limits.rows, reference.limits.bytes
        )
    })
}

fn format_page_status(start_row: u64, row_count: u64, reference: Option<&ResultRef>) -> String {
    let total_rows = reference.map_or(0, |current| current.rows);
    let truncated_suffix = if reference.is_some_and(|current| current.truncated) {
        " (truncated)"
    } else {
        ""
    };
    if row_count == 0 {
        format!("Showing 0 of {total_rows} rows{truncated_suffix}")
    } else {
        let page_end = start_row.saturating_add(row_count);
        format!(
            "Showing rows {}–{} of {total_rows}{truncated_suffix}",
            start_row.saturating_add(1),
            page_end,
        )
    }
}

pub(crate) fn panel(
    result: Signal<Option<ResultRef>>,
    stale_result: Signal<Option<String>>,
    on_sort: Rc<dyn Fn(Option<DataTableSort>)>,
) -> impl IntoElement {
    let rows = signal(DataTableRows::new(0));
    let columns = signal(Vec::<DataTableColumn>::new());
    let data_start = signal(0u32);
    let page_status = signal(String::from("No result file has been published."));
    let sort = signal(None::<DataTableSort>);
    let request_ticket = signal(0u64);

    let load_page: Rc<dyn Fn(u64, u64)> = Rc::new(move |start_row, requested_rows| {
        let Some(reference) = result.get_untracked() else {
            return;
        };
        let ticket = request_ticket.get_untracked().saturating_add(1);
        request_ticket.set(ticket);
        let limit = requested_rows.clamp(1, MAX_PAGE_ROWS);
        let expected_id = reference.id.clone();
        let expected_generation = reference.generation;
        page_status.set(format!(
            "Loading rows {}–{}…",
            start_row + 1,
            start_row + limit
        ));

        let path = reference.path.clone();
        let rows_signal = rows;
        let columns_signal = columns;
        let start_signal = data_start;
        let status_signal = page_status;
        let current_result = result;
        let current_ticket = request_ticket;
        spawn(async move {
            let decoded = blocking(move || {
                let reader = IpcPageReader::open(path).map_err(|error| error.to_string())?;
                let page = reader
                    .read_rows(start_row, limit)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>(decode_page(page))
            })
            .await;

            if current_ticket.get_untracked() != ticket {
                return;
            }
            let still_current = current_result.get_untracked().is_some_and(|current| {
                current.id == expected_id && current.generation == expected_generation
            });
            if !still_current {
                return;
            }

            match decoded {
                Ok(Ok(page)) => {
                    rows_signal.set(table_rows(&page));
                    if !page.columns.is_empty() {
                        columns_signal.set(columns_of(&page));
                    }
                    start_signal.set(page.start_row.min(u64::from(u32::MAX)) as u32);
                    let current = current_result.get_untracked();
                    status_signal.set(format_page_status(
                        page.start_row,
                        page.row_count,
                        current.as_ref(),
                    ));
                }
                Ok(Err(error)) => status_signal.set(format!("Could not read result page: {error}")),
                Err(error) => status_signal.set(format!("Result page task failed: {error}")),
            }
        });
    });

    let initial_load = load_page.clone();
    effect(move || match result.get() {
        Some(_) => {
            rows.set(DataTableRows::new(0));
            columns.set(Vec::new());
            data_start.set(0);
            initial_load(0, MAX_PAGE_ROWS);
        }
        None => {
            rows.set(DataTableRows::new(0));
            columns.set(Vec::new());
            data_start.set(0);
            page_status.set("No result file has been published.".into());
        }
    });

    let visible_load = load_page.clone();
    let sort_signal = sort;
    let on_sort_change = on_sort.clone();
    view()
        .app_class("flex-col size-full bg-[#fbfaf6] text-[#24312c] dark:bg-[#171c19] dark:text-[#e6e9e2]")
        .child(
            view()
                .app_class("h-12 shrink-0 flex-row items-center justify-between border-b border-[#e0ddd3] px-4 dark:border-[#343b36]")
                .child(
                    view()
                        .app_class("flex-row items-center gap-2")
                        .child(text("RESULTS").app_class("text-xs font-semibold tracking-widest text-[#5c6e65] dark:text-[#a6b2a9]"))
                        .child(
                            text(move || {
                                result
                                    .get()
                                    .as_ref()
                                    .and_then(truncation_label)
                                    .unwrap_or_default()
                            })
                            .app_class("text-[10px] font-semibold text-[#9b623a] dark:text-[#d4ad7f]"),
                        ),
                )
                .child(text(move || stale_result.get().unwrap_or_default()).app_class("max-w-[35%] truncate text-[10px] text-[#9b623a] dark:text-[#d4ad7f]"))
                .child(text(move || page_status.get()).app_class("text-xs text-[#7d877f] dark:text-[#9ba69e]")),
        )
        .child(
            view()
                .app_class("min-h-0 flex-1")
                .child(
                    data_table()
                        .columns(move || columns.get())
                        .data(move || rows.get())
                        .data_start(move || data_start.get())
                        .row_count(move || {
                            result
                                .get()
                                .map(|reference| reference.rows.min(u64::from(u32::MAX)) as u32)
                        })
                        .sort(move || sort_signal.get())
                        .accessibility_label("Query result rows")
                        .empty(text("Run a preview to populate this result table."))
                        .on_visible_rows_change(move |range: &DataTableRange| {
                            let visible = u64::from(range.end.saturating_sub(range.start));
                            visible_load(
                                u64::from(range.start),
                                visible.saturating_add(PAGE_OVERSCAN_ROWS),
                            );
                        })
                        .on_sort_change(move |next: &Option<DataTableSort>| {
                            sort_signal.set(next.clone());
                            on_sort_change(next.clone());
                        })
                        .size_full(),
                ),
        )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use omo_protocol::result_ref::{ResultLimits, ResultRef};

    use super::{DecodedPage, columns_of, format_page_status, truncation_label};

    fn sample_ref(truncated: bool) -> ResultRef {
        ResultRef {
            id: "r1".into(),
            generation: 1,
            path: PathBuf::from("/tmp/r1.arrow"),
            rows: 100_000,
            schema_hash: "0".repeat(64),
            truncated,
            limits: ResultLimits {
                rows: 100_000,
                bytes: 268_435_456,
            },
        }
    }

    #[test]
    fn result_columns_enable_sorting() {
        let page = DecodedPage {
            start_row: 0,
            row_count: 1,
            columns: vec![("temperature".to_owned(), vec!["1".to_owned()])],
        };

        let columns = columns_of(&page);
        assert_eq!(columns.len(), 1);
        assert!(columns[0].sortable);
    }

    #[test]
    fn truncated_results_are_explicitly_labelled() {
        let truncated_ref = sample_ref(true);
        let full_ref = sample_ref(false);

        assert!(
            truncation_label(&truncated_ref)
                .as_deref()
                .is_some_and(|label| label.contains("TRUNCATED") && label.contains("100000"))
        );
        assert!(truncation_label(&full_ref).is_none());

        assert!(
            format_page_status(0, 500, Some(&truncated_ref)).contains("(truncated)"),
            "page status must label truncation"
        );
        assert!(
            !format_page_status(0, 500, Some(&full_ref)).contains("truncated"),
            "non-truncated results must not be labelled truncated"
        );
    }
}
