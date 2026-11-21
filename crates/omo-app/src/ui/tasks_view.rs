//! Live worker, preview queue, bootstrap report, and manual export controls.

use std::rc::Rc;

use nsh_base::{button, input};
use nsh_core::prelude::*;
use serde_json::Value;

pub(crate) fn panel(
    status: Signal<String>,
    request_count: Signal<u64>,
    worker_state: Signal<String>,
    worker_generation: Signal<u64>,
    bootstrap_report: Signal<Option<Value>>,
    manual_export_path: Signal<String>,
    on_cancel: Rc<dyn Fn()>,
    on_retry: Rc<dyn Fn()>,
    on_manual_export: Rc<dyn Fn()>,
) -> impl IntoElement {
    let cancel = on_cancel.clone();
    let retry = on_retry.clone();
    let export_manual = on_manual_export.clone();
    view()
        .app_class("flex-col size-full bg-[#f5f3ed] text-[#24312c] dark:bg-[#1b201c] dark:text-[#e6e9e2]")
        .child(
            view()
                .app_class("h-11 shrink-0 flex-row items-center justify-between border-b border-[#ddd9cf] px-4 dark:border-[#343b36]")
                .child(text("TASKS & ENGINE").app_class("text-xs font-semibold tracking-widest text-[#5c6e65] dark:text-[#a6b2a9]"))
                .child(text(move || worker_state.get()).app_class("rounded-full bg-[#e3e9df] px-2 py-1 text-[10px] font-semibold tracking-wide text-[#40594d] dark:bg-[#2a382f] dark:text-[#bfd0c3]")),
        )
        .child(
            view()
                .app_class("min-h-0 flex-1 flex-row gap-5 overflow-hidden p-4")
                .child(
                    view()
                        .app_class("min-w-0 flex-1 flex-col gap-2")
                        .child(view().app_class("flex-row items-center justify-between")
                            .child(text("Preview queue").app_class("text-sm font-semibold"))
                            .child(text(move || format!("{} requests", request_count.get())).app_class("text-xs text-[#7d877f] dark:text-[#9ba69e]")))
                        .child(text(move || status.get()).app_class("text-xs leading-5 text-[#64736a] dark:text-[#a4ada6]"))
                        .child(text(move || format!("Worker generation {}", worker_generation.get()))
                            .app_class("text-xs text-[#7d877f] dark:text-[#9ba69e]"))
                        .child(text("Retry submits the current SQL revision as the latest preview. Cancel interrupts the active preview.")
                            .app_class("text-xs leading-5 text-[#7d877f] dark:text-[#9ba69e]")),
                )
                .child(view().app_class("w-px shrink-0 bg-[#ddd9cf] dark:bg-[#343b36]"))
                .child(
                    view()
                        .app_class("w-[300px] shrink-0 flex-col gap-2")
                        .child(text("Bootstrap report").app_class("text-sm font-semibold"))
                        .child(view().app_class("flex-row justify-between text-xs")
                            .child(text("DuckDB").app_class("text-[#7d877f] dark:text-[#9ba69e]"))
                            .child(text(move || report_text(bootstrap_report.get(), "duckdb_version"))))
                        .child(view().app_class("flex-row justify-between text-xs")
                            .child(text("Duckomo").app_class("text-[#7d877f] dark:text-[#9ba69e]"))
                            .child(text(move || report_text(bootstrap_report.get(), "duckomo_version"))))
                        .child(view().app_class("flex-row justify-between text-xs")
                            .child(text("Channel / platform").app_class("text-[#7d877f] dark:text-[#9ba69e]"))
                            .child(text(move || format!("{} / {}", report_text(bootstrap_report.get(), "channel"), report_text(bootstrap_report.get(), "platform")))))
                        .child(view().app_class("flex-row justify-between text-xs")
                            .child(text("httpfs").app_class("text-[#7d877f] dark:text-[#9ba69e]"))
                            .child(text(move || report_text(bootstrap_report.get(), "httpfs"))))
                        .child(view().app_class("flex-row justify-between text-xs")
                            .child(text("Spatial").app_class("text-[#7d877f] dark:text-[#9ba69e]"))
                            .child(text(move || report_text(bootstrap_report.get(), "spatial"))))
                        .child(text(move || full_report(bootstrap_report.get()))
                            .app_class("max-h-[55px] overflow-auto break-all text-[9px] leading-3 text-[#7d877f] dark:text-[#9ba69e]"))
                        .child(text("Manual export path").app_class("mt-2 text-xs font-medium"))
                        .child(input()
                            .value(move || manual_export_path.get())
                            .placeholder("Absolute .csv or .parquet path")
                            .on_change(move |value: &str| manual_export_path.set(value.to_owned()))
                            .app_class("h-8 rounded-md border border-[#d6d1c5] bg-white px-2 text-xs dark:border-[#404740] dark:bg-[#202722]"))
                        .child(button()
                            .app_class("h-7 self-start rounded-md bg-[#e7e9df] px-3 text-xs font-semibold text-[#43584d] hover:bg-[#dde2d6] dark:bg-[#2d3931] dark:text-[#c3d0c5]")
                            .on_press(move || export_manual())
                            .child(text("Export to path")))
                        .child(view().app_class("mt-1 flex-row gap-2")
                            .child(button()
                                .app_class("h-7 rounded-md border border-[#d7d2c7] px-3 text-xs font-medium text-[#707a72] hover:bg-[#eeece5] dark:border-[#404740] dark:hover:bg-[#252c27]")
                                .on_press(move || cancel())
                                .child(text("Cancel")))
                            .child(button()
                                .app_class("h-7 rounded-md border border-[#d7d2c7] px-3 text-xs font-medium text-[#707a72] hover:bg-[#eeece5] dark:border-[#404740] dark:hover:bg-[#252c27]")
                                .on_press(move || retry())
                                .child(text("Retry")))),
                ),
        )
}

fn full_report(report: Option<Value>) -> String {
    report.map_or_else(
        || "Full bootstrap report not reported".into(),
        |report| report.to_string(),
    )
}

fn report_text(report: Option<Value>, field: &str) -> String {
    report
        .and_then(|report| report.get(field).cloned())
        .map(|value| match value {
            Value::String(value) => value,
            other => other.to_string(),
        })
        .unwrap_or_else(|| "not reported".into())
}
