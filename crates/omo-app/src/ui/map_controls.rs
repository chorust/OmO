//! Map toolbar: valid-time stepper, palette and colour-range lock, zoom and framing buttons.

use nsh_base::{SliderValue, button, checkbox, input, slider};
use nsh_core::prelude::*;
use omo_project::map::{LayerMove, MapLayerKind};

use super::{dialogs, map_view::MapController};

const BTN: &str = "h-7 rounded-md border border-[#d6d1c5] bg-white px-2 text-xs font-semibold text-[#24312c] hover:bg-[#eeece5] disabled:opacity-50 dark:border-[#404740] dark:bg-[#202722] dark:text-[#e6e9e2] dark:hover:bg-[#252c27]";
const CANCEL_BTN: &str = "h-7 rounded-md border border-[#b05243] bg-[#b05243]/10 px-2 text-xs font-semibold text-[#b05243] hover:bg-[#b05243]/20 dark:border-[#d47566] dark:text-[#e28e80]";
const FIELD: &str = "h-7 w-16 rounded-md border border-[#d6d1c5] bg-white px-2 text-xs dark:border-[#404740] dark:bg-[#202722]";
const LABEL: &str = "text-[10px] font-semibold tracking-wide text-[#7d877f] dark:text-[#9ba69e]";

pub(super) fn toolbar(controller: MapController) -> impl IntoElement {
    let field = controller.field;
    let palette = controller.palette;
    let locked_min = controller.locked_min;
    let locked_max = controller.locked_max;

    let prev = controller.clone();
    let next = controller.clone();
    let reload = controller.clone();
    let cycle = controller.clone();
    let zoom_out = controller.clone();
    let zoom_in = controller.clone();
    let fit = controller.clone();
    let fit_check = controller.clone();
    let can_fit = computed(move || fit_check.has_in_projection_stations());
    let min_changed = controller.clone();
    let max_changed = controller.clone();
    let vp_label = controller.viewport;
    let layers_open = controller.layers_open;
    let export = controller.clone();
    let export_typed = controller.clone();
    let export_manual_open = controller.export_manual_open;
    let export_path_input = controller.export_path_input;
    let exporting = controller.exporting;
    let loading = controller.loading;
    let cancel = controller.clone();

    view()
        .app_class("shrink-0 flex-row flex-wrap items-center gap-2 border-b border-[#e0ddd3] bg-[#f4f2eb] px-3 py-2 dark:border-[#343b36] dark:bg-[#1d241f]")
        .child(text("VALID TIME").app_class(LABEL))
        .child(
            button()
                .app_class(BTN)
                .on_press(move || prev.step_valid_time(-1))
                .child(text("◀")),
        )
        .child(
            text(move || {
                field
                    .get()
                    .map(|f| f.result.valid_time_utc)
                    .unwrap_or_else(|| "—".into())
            })
            .app_class("min-w-[9rem] text-xs font-semibold"),
        )
        .child(
            button()
                .app_class(BTN)
                .on_press(move || next.step_valid_time(1))
                .child(text("▶")),
        )
        .child(view().app_class("mx-1 h-5 w-px bg-[#d6d1c5] dark:bg-[#404740]"))
        .child(text("PALETTE").app_class(LABEL))
        .child(
            button()
                .app_class(BTN)
                .on_press(move || cycle.palette_cycle())
                .child(text(move || palette.get())),
        )
        .child(text("MIN").app_class(LABEL))
        .child(
            input()
                .value(move || locked_min.get())
                .placeholder("auto")
                .on_change(move |value: &str| {
                    locked_min.set(value.to_owned());
                    min_changed.on_color_input_changed();
                })
                .app_class(FIELD),
        )
        .child(text("MAX").app_class(LABEL))
        .child(
            input()
                .value(move || locked_max.get())
                .placeholder("auto")
                .on_change(move |value: &str| {
                    locked_max.set(value.to_owned());
                    max_changed.on_color_input_changed();
                })
                .app_class(FIELD),
        )
        .child(view().app_class("flex-1"))
        .child(
            button()
                .app_class(BTN)
                .on_press(move || zoom_out.zoom_by(-1, None))
                .child(text("−")),
        )
        .child(
            text(move || format!("z{}", vp_label.get().zoom)).app_class("w-6 text-center text-xs"),
        )
        .child(
            button()
                .app_class(BTN)
                .on_press(move || zoom_in.zoom_by(1, None))
                .child(text("+")),
        )
        .child(
            button()
                .app_class(BTN)
                .on_press(move || layers_open.update(|open| *open = !*open))
                .child(text("Layers")),
        )
        .child(
            view()
                .tooltip(
                    text(move || {
                        if can_fit.get() {
                            "Frame confirmed in-projection stations"
                        } else {
                            "Import and confirm in-projection stations to enable framing"
                        }
                    })
                    .app_class("px-2 py-1 text-xs"),
                )
                .child(
                    button()
                        .disabled(move || !can_fit.get())
                        .app_class(BTN)
                        .on_press(move || fit.fit_to_stations())
                        .child(text("Fit to stations")),
                ),
        )
        .child(
            button()
                .app_class(BTN)
                .on_press(move || reload.request_field(None))
                .child(text("Load field")),
        )
        .child(when(
            move || loading.get() || exporting.get(),
            move || {
                let cancel = cancel.clone();
                button()
                    .app_class(CANCEL_BTN)
                    .on_press(move || {
                        cancel.cancel_active();
                    })
                    .child(text("Cancel"))
            },
        ))
        .child(
            button()
                .app_class(BTN)
                .on_press(move || export.choose_export())
                .child(text(move || {
                    if exporting.get() {
                        "Exporting…"
                    } else {
                        "Export Map PNG…"
                    }
                })),
        )
        .child(when(
            move || export_manual_open.get(),
            move || {
                let export_typed = export_typed.clone();
                view()
                    .app_class("w-full flex-row items-center gap-2")
                    .child(text("PNG PATH").app_class(LABEL))
                    .child(
                        input()
                            .value(move || export_path_input.get())
                            .placeholder("/absolute/path/map.png")
                            .on_change(move |value: &str| export_path_input.set(value.to_owned()))
                            .app_class("h-7 flex-1 rounded-md border border-[#d6d1c5] bg-white px-2 text-xs dark:border-[#404740] dark:bg-[#202722]"),
                    )
                    .child(
                        button()
                            .app_class(BTN)
                            .on_press(move || export_typed.export_to_typed_path())
                            .child(text("Export")),
                    )
            },
        ))
}

/// One row of the Layer Stack, top of the stack first.
#[derive(Clone, Debug, PartialEq)]
struct LayerRow {
    id: String,
    kind: RowKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowKind {
    Forecast,
    Coastlines,
    Archive,
    Stations,
}

const ROW_BTN: &str = "h-6 w-6 rounded border border-[#d6d1c5] bg-white text-[11px] font-semibold text-[#24312c] hover:bg-[#eeece5] dark:border-[#404740] dark:bg-[#202722] dark:text-[#e6e9e2] dark:hover:bg-[#252c27]";
const TAG: &str = "rounded-full bg-[#e8e5da] px-1.5 py-0.5 text-[9px] font-semibold text-[#55625a] dark:bg-[#2b332d] dark:text-[#b4beb6]";

/// The Layer Stack panel (`FR-013`..`FR-016`): visibility, opacity, z-order, mounting and
/// removing local `.pmtiles` archives, per-layer diagnostics and attribution.
pub(super) fn layer_stack_panel(controller: MapController) -> impl IntoElement {
    let state = controller.state;
    let meta = controller.layer_meta;
    let note = controller.layer_note;
    let path_input = controller.path_input;
    let rows = computed(move || {
        state.with(|state| {
            state
                .layers
                .iter()
                .rev()
                .map(|layer| LayerRow {
                    id: layer.id.clone(),
                    kind: match layer.kind {
                        MapLayerKind::ForecastField => RowKind::Forecast,
                        MapLayerKind::BuiltinCoastlines => RowKind::Coastlines,
                        MapLayerKind::LocalPmtiles { .. } => RowKind::Archive,
                        MapLayerKind::StationOverlay => RowKind::Stations,
                    },
                })
                .collect::<Vec<_>>()
        })
    });

    let rows_controller = controller.clone();
    let pick_controller = controller.clone();
    let add_controller = controller.clone();
    let input_controller = controller.clone();

    view()
        .app_class("absolute right-2 top-2 w-72 flex-col gap-2 rounded-md border border-[#d6d1c5] bg-white/95 p-2 text-[11px] text-[#24312c] dark:border-[#404740] dark:bg-[#202722]/95 dark:text-[#e6e9e2]")
        .child(text("LAYER STACK · top to bottom").app_class(LABEL))
        .child(
            scroll().app_class("max-h-[300px]").child(
                view().app_class("flex-col gap-1").child(each(
                    move || rows.get(),
                    |row: &LayerRow| row.id.clone(),
                    move |item, _| layer_row(rows_controller.clone(), item.get_untracked()),
                )),
            ),
        )
        .child(
            button()
                .app_class(BTN)
                .on_press(move || {
                    let add = pick_controller.clone();
                    let report = pick_controller.clone();
                    dialogs::choose_pmtiles(
                        move |path| add.add_archive(path),
                        move |error| {
                            report
                                .layer_note
                                .set(format!("The file picker is unavailable ({error}); type the path below."))
                        },
                    );
                })
                .child(text("Add local PMTiles…")),
        )
        .child(
            view()
                .app_class("flex-row items-center gap-1")
                .child(
                    input()
                        .value(move || path_input.get())
                        .placeholder("/path/to/layer.pmtiles")
                        .on_change(move |value: &str| path_input.set(value.to_owned()))
                        .app_class("h-7 min-w-0 flex-1 rounded-md border border-[#d6d1c5] bg-white px-2 text-xs dark:border-[#404740] dark:bg-[#202722]"),
                )
                .child(
                    button()
                        .app_class(BTN)
                        .on_press(move || add_controller.add_archive_from_input())
                        .child(text("Add path")),
                ),
        )
        .child(text(move || note.get()).app_class("text-[10px] text-[#b05243]"))
        .child(
            text(move || {
                let _ = &input_controller;
                let meta = meta.get();
                let errors = meta.values().filter(|m| m.error.is_some()).count();
                if errors == 0 {
                    String::new()
                } else {
                    format!("{errors} layer(s) failed; the others are unaffected.")
                }
            })
            .app_class("text-[10px] text-[#b05243]"),
        )
}

fn layer_row(controller: MapController, row: LayerRow) -> impl IntoElement {
    let state = controller.state;
    let meta = controller.layer_meta;
    let id = row.id.clone();

    let name_id = id.clone();
    let name = move || {
        state.with(|state| {
            let Some(layer) = state.layer(&name_id) else {
                return String::new();
            };
            match &layer.kind {
                MapLayerKind::ForecastField => "Forecast field".to_owned(),
                MapLayerKind::BuiltinCoastlines => "Coastlines & borders".to_owned(),
                MapLayerKind::StationOverlay => "Stations".to_owned(),
                MapLayerKind::LocalPmtiles { path } => path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| name_id.clone()),
            }
        })
    };
    let tag = match row.kind {
        RowKind::Forecast => "native numeric".to_owned(),
        _ => "display only".to_owned(),
    };
    let detail_id = id.clone();
    let detail = move || {
        let meta = meta.get();
        let Some(entry) = meta.get(&detail_id) else {
            return String::new();
        };
        if let Some(error) = &entry.error {
            return format!("⚠ {error}");
        }
        entry
            .info
            .as_ref()
            .map(|info| {
                format!(
                    "{:?} · z{}–{}{}",
                    info.tile_kind,
                    info.min_zoom,
                    info.max_zoom,
                    info.attribution
                        .as_deref()
                        .map(|a| format!(" · {a}"))
                        .unwrap_or_default()
                )
            })
            .unwrap_or_else(|| "checking…".to_owned())
    };
    let has_error_id = id.clone();
    let has_error =
        move || meta.with(|meta| meta.get(&has_error_id).is_some_and(|m| m.error.is_some()));

    let visible_id = id.clone();
    let toggle_id = id.clone();
    let toggle = controller.clone();
    let opacity_id = id.clone();
    let opacity_label_id = id.clone();
    let set_opacity = controller.clone();
    let up_id = id.clone();
    let up = controller.clone();
    let down_id = id.clone();
    let down = controller.clone();
    let remove_id = id.clone();
    let remove = controller.clone();
    let retry_id = id.clone();
    let retry = controller.clone();
    let pinned = row.kind == RowKind::Stations;
    let removable = row.kind == RowKind::Archive;
    let label = format!("Visible: {}", row.id);

    view()
        .app_class("flex-col gap-1 rounded border border-[#e6e2d6] p-1.5 dark:border-[#343b36]")
        .child(
            view()
                .app_class("flex-row items-center gap-1")
                .child(
                    checkbox()
                        .app_class("min-w-0 flex-1 flex-row items-center gap-1.5")
                        .accessibility_label(label)
                        .indicator_class("size-4 shrink-0 rounded border border-[#a9b3ac] checked:border-[#24584c] checked:bg-[#24584c]")
                        .checked(move || state.with(|s| s.layer(&visible_id).is_some_and(|l| l.visible)))
                        .on_change(move |on: &bool| toggle.set_layer_visible(&toggle_id, *on))
                        .child(text(name).app_class("truncate text-xs font-semibold")),
                )
                .child(text(tag).app_class(TAG))
                .child(when(
                    move || !pinned,
                    move || {
                        let up_id = up_id.clone();
                        let up = up.clone();
                        let down_id = down_id.clone();
                        let down = down.clone();
                        view()
                            .app_class("flex-row gap-1")
                            .child(
                                button()
                                    .app_class(ROW_BTN)
                                    .accessibility_label("Move layer up")
                                    .on_press(move || up.move_layer(&up_id, LayerMove::Up))
                                    .child(text("▲")),
                            )
                            .child(
                                button()
                                    .app_class(ROW_BTN)
                                    .accessibility_label("Move layer down")
                                    .on_press(move || down.move_layer(&down_id, LayerMove::Down))
                                    .child(text("▼")),
                            )
                    },
                ))
                .child(when(
                    move || removable,
                    move || {
                        let remove_id = remove_id.clone();
                        let remove = remove.clone();
                        button()
                            .app_class(ROW_BTN)
                            .accessibility_label("Remove layer")
                            .on_press(move || remove.remove_layer(&remove_id))
                            .child(text("✕"))
                    },
                )),
        )
        .child(
            view()
                .app_class("flex-row items-center gap-2")
                .child(
                    slider()
                        .min(0.0)
                        .max(100.0)
                        .step(1.0)
                        .accessibility_label("Layer opacity")
                        .track_class("h-5 flex-1 justify-center")
                        .indicator_class("h-1.5 rounded-full bg-[#d6d1c5] dark:bg-[#404740]")
                        .range_class("rounded-full bg-[#24584c]")
                        .thumb_class("size-3.5 rounded-full border border-[#24584c] bg-white")
                        .value(move || {
                            Some(SliderValue::Single(f64::from(
                                state.with(|s| s.layer(&opacity_id).map_or(1.0, |l| l.opacity)),
                            ) * 100.0))
                        })
                        .on_change({
                            let id = id.clone();
                            move |value: &SliderValue| {
                                set_opacity.set_layer_opacity(&id, (value.start() / 100.0) as f32)
                            }
                        }),
                )
                .child(
                    text(move || {
                        format!(
                            "{:.0}%",
                            state.with(|s| s.layer(&opacity_label_id).map_or(1.0, |l| l.opacity))
                                * 100.0
                        )
                    })
                    .app_class("w-8 text-right text-[10px]"),
                ),
        )
        .child(text(detail).app_class("text-[10px] text-[#6b786f] dark:text-[#a4ada6]"))
        .child(when(has_error, move || {
            let retry_id = retry_id.clone();
            let retry = retry.clone();
            button()
                .app_class(BTN)
                .on_press(move || retry.retry_layer(&retry_id))
                .child(text("Retry"))
        }))
}
