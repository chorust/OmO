//! Lead-time view window (US3): a range slider plus two numeric fields, clamped to the received
//! run range. Narrowing re-queries the received result; it never resubmits `series`.

use nsh_base::{SliderValue, number_input, slider};
use nsh_core::prelude::*;

use super::series_panel::SeriesController;

pub(super) fn panel(controller: &SeriesController) -> impl IntoElement {
    let start = controller.view_start;
    let end = controller.view_end;
    let note = controller.view_note;
    let outcome = controller.outcome;
    let bounds = computed(move || {
        outcome
            .get()
            .map(|outcome| outcome.body.effective_window.effective)
            .unwrap_or([0, 168])
    });
    let number =
        |text: String, fallback: u32| text.trim().parse::<f64>().unwrap_or(fallback as f64);
    view()
        .app_class("flex-col gap-2 rounded-lg border border-[#dedacf] p-3 dark:border-[#343b36]")
        .child(text("LEAD-TIME VIEW WINDOW").app_class(
            "text-[10px] font-semibold tracking-widest text-[#7d877f] dark:text-[#9ba69e]",
        ))
        .child(
            slider()
                .min(move || bounds.get()[0] as f64)
                .max(move || bounds.get()[1] as f64)
                .step(1.0)
                .accessibility_label("Lead-time end")
                .start_accessibility_label("Lead-time start")
                .track_class("h-6 justify-center")
                .indicator_class("h-1.5 rounded-full bg-[#d6d1c5] dark:bg-[#404740]")
                .range_class("rounded-full bg-[#24584c]")
                .thumb_class("size-4 rounded-full border border-[#24584c] bg-white")
                .value(move || {
                    let bounds = bounds.get();
                    Some(SliderValue::Range(
                        number(start.get(), bounds[0]),
                        number(end.get(), bounds[1]),
                    ))
                })
                .on_change(move |value: &SliderValue| {
                    start.set(format!("{}", value.start().round() as u32));
                    end.set(format!("{}", value.end().round() as u32));
                }),
        )
        .child(
            view()
                .app_class("flex-row items-center gap-2 text-xs")
                .child(text("From"))
                .child(
                    number_input()
                        .value(move || Some(number(start.get(), bounds.get()[0])))
                        .step(1.0)
                        .accessibility_label("View start hour")
                        .on_change(move |value: &Option<f64>| {
                            if let Some(value) = *value {
                                start.set(format!("{}", value.max(0.0).round() as u32));
                            }
                        })
                        .app_class("h-8 w-20 rounded-md border border-[#d6d1c5] bg-white px-2 dark:border-[#404740] dark:bg-[#202722]"),
                )
                .child(text("to"))
                .child(
                    number_input()
                        .value(move || Some(number(end.get(), bounds.get()[1])))
                        .step(1.0)
                        .accessibility_label("View end hour")
                        .on_change(move |value: &Option<f64>| {
                            if let Some(value) = *value {
                                end.set(format!("{}", value.max(0.0).round() as u32));
                            }
                        })
                        .app_class("h-8 w-20 rounded-md border border-[#d6d1c5] bg-white px-2 dark:border-[#404740] dark:bg-[#202722]"),
                )
                .child(text(move || {
                    let bounds = bounds.get();
                    format!("h (received {}–{} h)", bounds[0], bounds[1])
                })),
        )
        .child(text(move || note.get()).app_class("text-xs text-[#b05243]"))
}
