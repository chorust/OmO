//! Bounded, evenly sampled plot of the complete immutable Arrow result.

use std::collections::HashSet;

use nsh_base::checkbox;
use nsh_base::plot::{Curve, grid, line, plot};
use nsh_core::element::{RowAccessor, Rows};
use nsh_core::prelude::*;
use omo_protocol::result_ref::ResultRef;

use crate::host::ipc_pages::{IpcPageReader, MAX_PLOT_POINTS, MAX_PLOT_SERIES};

#[derive(Clone, PartialEq)]
pub(super) struct PlotInput {
    pub reference: ResultRef,
    pub source_rows: u64,
    pub worker_sampled: bool,
}

const SERIES_COLORS: [u32; MAX_PLOT_SERIES] = [
    0x286354, 0xbd7950, 0x5479a3, 0x9b5e78, 0x65843d, 0xb05243, 0x6f5aa8, 0x9c7f32, 0x3a8d8b,
    0xc45c8a, 0x698d54, 0x8a6d59, 0x4a6e8e, 0xa16c9c, 0x7a842d, 0x4b8270, 0xad6544, 0x6c6c9e,
    0x8b824b, 0x547e9e,
];

pub(crate) fn panel(
    rows: Signal<Rows>,
    result: Signal<Option<ResultRef>>,
    plot_input: Signal<Option<PlotInput>>,
) -> impl IntoElement {
    let status = signal(String::from("No result is available yet."));
    let sample_count = signal(0usize);
    let series_count = computed(move || rows.get().keys().count().min(MAX_PLOT_SERIES));
    let request_ticket = signal(0u64);
    let hidden = signal(HashSet::<String>::new());
    let legend = computed(move || {
        rows.get()
            .keys()
            .take(MAX_PLOT_SERIES)
            .enumerate()
            .map(|(index, key)| (index, key.to_owned()))
            .collect::<Vec<_>>()
    });

    effect(move || {
        let input = plot_input.get().or_else(|| {
            result
                .get()
                .filter(|reference| reference.id == "fixture-result")
                .map(|reference| PlotInput {
                    source_rows: reference.rows,
                    reference,
                    worker_sampled: false,
                })
        });
        match input {
            Some(input) => {
                let reference = input.reference;
                let ticket = request_ticket.get_untracked().saturating_add(1);
                request_ticket.set(ticket);
                hidden.set(HashSet::new());
                rows.set(Rows::default());
                sample_count.set(0);
                status.set(format!(
                    "Sampling up to {MAX_PLOT_POINTS} points per numeric series…"
                ));

                let path = reference.path.clone();
                let expected_id = reference.id.clone();
                let expected_generation = reference.generation;
                let source_rows = input.source_rows;
                let worker_sampled = input.worker_sampled;
                let current_result = result;
                let current_plot_input = plot_input;
                let current_ticket = request_ticket;
                let rows_signal = rows;
                let count_signal = sample_count;
                let status_signal = status;
                spawn(async move {
                    let sampled = blocking(move || {
                        let reader =
                            IpcPageReader::open(path).map_err(|error| error.to_string())?;
                        reader
                            .read_numeric_sample(MAX_PLOT_POINTS, MAX_PLOT_SERIES)
                            .map_err(|error| error.to_string())
                    })
                    .await;

                    if current_ticket.get_untracked() != ticket {
                        return;
                    }
                    let still_current = if worker_sampled {
                        current_plot_input.get_untracked().is_some_and(|current| {
                            current.reference.id == expected_id
                                && current.reference.generation == expected_generation
                        })
                    } else {
                        current_result.get_untracked().is_some_and(|current| {
                            current.id == expected_id && current.generation == expected_generation
                        })
                    };
                    if !still_current {
                        return;
                    }

                    match sampled {
                        Ok(Ok(sample)) => {
                            let sampled_rows = sample.sampled_rows;
                            let series_count = sample.columns.len();
                            count_signal.set(sampled_rows);
                            rows_signal.set(Rows::from_columns(
                                sampled_rows,
                                sample
                                    .columns
                                    .into_iter()
                                    .map(|(name, values)| (name.into(), values)),
                            ));
                            let downsampled = source_rows > sampled_rows as u64;
                            let label = if downsampled {
                                if worker_sampled {
                                    "Worker downsampled"
                                } else {
                                    "Fixture downsampled"
                                }
                            } else {
                                "Evenly sampled"
                            };
                            status_signal.set(format!(
                                "{label} {sampled_rows} of {source_rows} result rows across {series_count} numeric series; cap: {MAX_PLOT_SERIES} series × {MAX_PLOT_POINTS} points.",
                            ));
                        }
                        Ok(Err(error)) => {
                            status_signal.set(format!("Could not sample result for plot: {error}"))
                        }
                        Err(error) => {
                            status_signal.set(format!("Plot sampling task failed: {error}"))
                        }
                    }
                });
            }
            None => {
                request_ticket.update(|ticket| *ticket = ticket.saturating_add(1));
                rows.set(Rows::default());
                sample_count.set(0);
                status.set(if result.get_untracked().is_some() {
                    "This result has no numeric columns for plotting.".into()
                } else {
                    "No result is available yet.".into()
                });
            }
        }
    });

    view()
        .app_class("flex-col size-full bg-[#fbfaf6] text-[#24312c] dark:bg-[#171c19] dark:text-[#e6e9e2]")
        .child(
            view()
                .app_class("h-12 shrink-0 flex-row items-center justify-between border-b border-[#e0ddd3] px-4 dark:border-[#343b36]")
                .child(text("PLOT PREVIEW").app_class("text-xs font-semibold tracking-widest text-[#5c6e65] dark:text-[#a6b2a9]"))
                .child(text(move || format!(
                    "{} series · {} sampled points",
                    series_count.get(),
                    sample_count.get()
                )).app_class("text-xs text-[#7d877f] dark:text-[#9ba69e]")),
        )
        .child(
            view()
                .app_class("min-h-0 flex-1 flex-col gap-3 p-5")
                .child(text(move || status.get()).app_class("text-xs leading-5 text-[#6b786f] dark:text-[#a4ada6]"))
                .child(
                    plot()
                        .data(move || rows.get())
                        .accessibility_label("Numeric query result preview")
                        .cross_line_class("w-px bg-[#bd7950]/50")
                        .dot_class("size-2 rounded-full border-2 border-[#fbfaf6] bg-[#bd7950] dark:border-[#171c19]")
                        .w_full()
                        .h(px(360.0))
                        .child(grid().stroke_color(rgb(0xe4e1d8)))
                        .children((0..MAX_PLOT_SERIES).map(|index| {
                            let rows_for_series = rows;
                            line()
                                .y(move || {
                                    let keys = rows_for_series.get().keys().map(str::to_owned).collect::<Vec<_>>();
                                    let hidden_keys = hidden.get();
                                    let key = keys
                                        .get(index)
                                        .filter(|key| !hidden_keys.contains(key.as_str()))
                                        .cloned()
                                        .unwrap_or_else(|| {
                                        let mut missing = format!("__omobench_missing_series_{index}");
                                        while keys.contains(&missing) {
                                            missing.push('_');
                                        }
                                        missing
                                    });
                                    Some(RowAccessor::key(key))
                                })
                                .curve(Curve::Linear)
                                .stroke_color(rgb(SERIES_COLORS[index]))
                                .stroke_width(px(1.6))
                        })),
                )
                .child(
                    view()
                        .app_class("max-h-[120px] flex-row flex-wrap gap-x-4 gap-y-1")
                        .child(each(
                            move || legend.get(),
                            |(index, key)| (*index, key.clone()),
                            move |item, _| {
                                let key = move || item.get().1;
                                let color = move || SERIES_COLORS[item.get().0 % MAX_PLOT_SERIES];
                                checkbox()
                                    .app_class("flex-row items-center gap-2 text-xs")
                                    .indicator_class("size-4 shrink-0 rounded border border-[#a9b3ac] checked:border-[#24584c] checked:bg-[#24584c]")
                                    .checked(move || !hidden.get().contains(&key()))
                                    .on_change(move |on: &bool| {
                                        let name = item.get_untracked().1;
                                        hidden.update(|set| {
                                            if *on {
                                                set.remove(&name);
                                            } else {
                                                set.insert(name);
                                            }
                                        });
                                    })
                                    .child(
                                        view()
                                            .app_class("size-3 shrink-0 rounded-sm")
                                            .bg(move || rgb(color())),
                                    )
                                    .child(text(move || key()))
                            },
                        )),
                )
                .child(text(move || {
                    if legend.get().is_empty() {
                        "No numeric columns are present in this result.".to_owned()
                    } else {
                        format!(
                            "{} series (cap {MAX_PLOT_SERIES} series × {MAX_PLOT_POINTS} points).",
                            legend.get().len()
                        )
                    }
                }).app_class("text-xs text-[#77837b] dark:text-[#9ba69e]")),
        )
}
