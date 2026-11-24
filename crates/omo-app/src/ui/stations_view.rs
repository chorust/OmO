//! Filterable checkbox list of the confirmed stations (US3).
//!
//! The selection lives in the controller's comma-separated `station_filter`, which is what the
//! project document persists as `view.selected_station_ids`.

use nsh_base::{button, checkbox, input};
use nsh_core::prelude::*;

use super::series_panel::SeriesController;

/// Most rows rendered at once; narrowing the search shows the rest.
const MAX_LISTED: usize = 300;

const BTN: &str = "h-7 rounded-md border border-[#d6d1c5] px-2 text-xs font-semibold hover:bg-[#eeece5] dark:border-[#404740] dark:hover:bg-[#252c27]";

fn matches(search: &str, id: &str, name: Option<&str>) -> bool {
    let needle = search.trim().to_lowercase();
    needle.is_empty()
        || id.to_lowercase().contains(&needle)
        || name.is_some_and(|name| name.to_lowercase().contains(&needle))
}

pub(super) fn panel(controller: &SeriesController) -> impl IntoElement {
    let snapshot = controller.snapshot;
    let search = controller.list_search;
    let listed = {
        let controller = controller.clone();
        computed(move || {
            let _ = snapshot.get();
            let query = search.get();
            controller
                .station_rows()
                .into_iter()
                .filter(|(id, name)| matches(&query, id, name.as_deref()))
                .take(MAX_LISTED)
                .map(|(id, name)| {
                    let label = match &name {
                        Some(name) if !name.trim().is_empty() => format!("{name} · {id}"),
                        _ => id.clone(),
                    };
                    (id, label)
                })
                .collect::<Vec<_>>()
        })
    };
    let summary = {
        let controller = controller.clone();
        computed(move || {
            let total = snapshot.get().map_or(0, |snapshot| snapshot.rows.len());
            let selected = controller.selected_ids().len();
            if selected == 0 {
                format!("{total} stations · none selected (the chart uses the first 20)")
            } else {
                format!("{total} stations · {selected} selected")
            }
        })
    };
    let select_all = {
        let controller = controller.clone();
        move || {
            let query = controller.list_search.get_untracked();
            let ids = controller
                .station_rows()
                .into_iter()
                .filter(|(id, name)| matches(&query, id, name.as_deref()))
                .map(|(id, _)| id)
                .collect::<Vec<_>>();
            controller.set_selected(ids);
        }
    };
    let clear = {
        let controller = controller.clone();
        move || controller.set_selected(Vec::new())
    };
    let row_controller = controller.clone();
    view()
        .app_class("flex-col gap-2 rounded-lg border border-[#dedacf] p-3 dark:border-[#343b36]")
        .child(text("STATION SELECTION").app_class(
            "text-[10px] font-semibold tracking-widest text-[#7d877f] dark:text-[#9ba69e]",
        ))
        .child(text(move || summary.get()).app_class("text-xs"))
        .child(
            view()
                .app_class("flex-row gap-2")
                .child(
                    input()
                        .value(move || search.get())
                        .placeholder("Search id or name")
                        .on_change(move |value: &str| search.set(value.to_owned()))
                        .app_class("h-7 min-w-0 flex-1 rounded-md border border-[#d6d1c5] bg-white px-2 text-xs dark:border-[#404740] dark:bg-[#202722]"),
                )
                .child(
                    button()
                        .app_class(BTN)
                        .on_press(select_all)
                        .child(text("Select all")),
                )
                .child(
                    button()
                        .app_class(BTN)
                        .on_press(clear)
                        .child(text("Clear")),
                ),
        )
        .child(
            scroll().app_class("max-h-[180px]").child(
                view().app_class("flex-col gap-1").child(each(
                    move || listed.get(),
                    |(id, _)| id.clone(),
                    move |item, _| {
                        let toggle = row_controller.clone();
                        let checked = row_controller.clone();
                        checkbox()
                            .app_class("flex-row items-center gap-2 text-xs")
                            .indicator_class("size-4 shrink-0 rounded border border-[#a9b3ac] checked:border-[#24584c] checked:bg-[#24584c]")
                            .checked(move || {
                                let id = item.get().0;
                                checked.selected_ids().contains(&id)
                            })
                            .on_change(move |on: &bool| {
                                toggle.toggle_station(&item.get_untracked().0, *on)
                            })
                            .child(text(move || item.get().1))
                    },
                )),
            ),
        )
}
