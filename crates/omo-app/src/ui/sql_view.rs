//! SQL editor panel. Edits update the project revision and previews enter
//! the host's coalescing queue through the callback supplied by Workspace.

use std::rc::Rc;

use nsh_base::{button, editor::editor};
use nsh_core::prelude::*;

pub(crate) fn panel(
    sql: Signal<String>,
    on_edit: Rc<dyn Fn(String)>,
    on_run: Rc<dyn Fn()>,
) -> impl IntoElement {
    let run = on_run.clone();
    view()
        .app_class("flex-col size-full bg-[#fbfaf6] text-[#24312c] dark:bg-[#171c19] dark:text-[#e6e9e2]")
        .child(
            view()
                .app_class("h-12 shrink-0 flex-row items-center justify-between border-b border-[#e0ddd3] px-4 dark:border-[#343b36]")
                .child(
                    view()
                        .app_class("flex-row items-center gap-2")
                        .child(text("SQL PREVIEW").app_class("text-xs font-semibold tracking-widest text-[#5c6e65] dark:text-[#a6b2a9]"))
                        .child(text("DRAFT").app_class("rounded-full bg-[#eee9dc] px-2 py-1 text-[10px] font-semibold tracking-wide text-[#786b4e] dark:bg-[#393729] dark:text-[#d2c99c]")),
                )
                .child(
                    view()
                        .app_class("flex-row items-center gap-3")
                        .child(text("MOD + ENTER").app_class("text-[10px] font-medium tracking-wide text-[#7d877f] dark:text-[#9ba69e]"))
                        .child(
                            button()
                                .app_class("h-8 flex-row items-center gap-2 rounded-md bg-[#24584c] px-3 text-sm font-semibold text-white hover:bg-[#1c493e] focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#c6824b]")
                                .on_press(move || run())
                                .child(text("▶"))
                                .child(text("Run preview")),
                        ),
                ),
        )
        .child(
            view()
                .app_class("min-h-0 flex-1 p-3")
                .child(
                    editor()
                        .language(Some(SharedString::from("sql")))
                        .value(move || SharedString::from(sql.get()))
                        .on_change(move |value: &SharedString| on_edit(value.to_string()))
                        .accessibility_label("SQL query editor")
                        .size_full(),
                ),
        )
}
