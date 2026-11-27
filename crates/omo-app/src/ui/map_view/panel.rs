//! NSH Map panel views: tile stack, responder-captured drag canvas, mutually exclusive
//! provenance badges, HUD readout, palette colorbar, station pins, and offline coastlines.

use nsh_core::{
    canvas::{LineCap, LineJoin},
    element::{ImageSrc, TouchClaim},
    prelude::*,
};
use omo_project::map::MapLayerKind;
use omo_protocol::{
    coastlines::{Polylines, builtin as builtin_coastlines},
    map::{DEFAULT_PALETTE, SourceLayout, palette_stops},
    mercator::{MAX_LATITUDE, TILE_SIZE},
};

use super::{MapController, hud_text};
use crate::{
    host::map_view::{Viewport, locked_range},
    ui::{map_controls, series_panel::SeriesController},
};

const BADGE: &str = "rounded-full bg-[#24312c]/80 px-2 py-0.5 text-[10px] font-semibold text-white";
const SAVED_BADGE: &str =
    "rounded-full bg-[#356b4a] px-2 py-0.5 text-[10px] font-semibold text-white";
const STALE_BADGE: &str =
    "rounded-full bg-[#b05243] px-2 py-0.5 text-[10px] font-semibold text-white";
const WARN_BADGE: &str =
    "rounded-full bg-[#9c7f32] px-2 py-0.5 text-[10px] font-semibold text-white";

#[derive(Clone, PartialEq)]
struct TileSlot {
    z: u8,
    ux: i64,
    y: u32,
    id: u64,
    image: ImageSrc,
}

#[derive(Clone, PartialEq)]
struct StationMark {
    id: String,
    lon: f64,
    lat: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SlotKind {
    Coastlines,
    Forecast,
    Archive,
    Stations,
}

#[derive(Clone, Debug, PartialEq)]
struct LayerSlot {
    id: String,
    kind: SlotKind,
    key: String,
}

pub(in crate::ui) fn panel(controller: MapController) -> impl IntoElement {
    let viewport = controller.viewport;
    let tiles = controller.tiles;

    let slots = computed(move || {
        let vp = viewport.get();
        tiles.with(|store| {
            vp.visible_tiles()
                .into_iter()
                .filter_map(|tile| {
                    let stored = store.get_touch(&(tile.coord.z, tile.coord.x, tile.coord.y))?;
                    Some(TileSlot {
                        z: tile.coord.z,
                        ux: tile.unwrapped_x,
                        y: tile.coord.y,
                        id: stored.id,
                        image: stored.image.clone()?,
                    })
                })
                .collect::<Vec<_>>()
        })
    });

    let series = controller.series.clone();
    let marks = computed(move || {
        series
            .snapshot
            .get()
            .map(|snapshot| {
                snapshot
                    .rows
                    .iter()
                    .filter(|row| row.latitude.abs() <= MAX_LATITUDE && row.longitude.is_finite())
                    .map(|row| StationMark {
                        id: row.station_id.clone(),
                        lon: row.longitude,
                        lat: row.latitude,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });

    let palette = controller.palette;
    let legend_stops = computed(move || {
        let active = palette.get();
        let stops = palette_stops(&active, 33)
            .or_else(|| palette_stops(DEFAULT_PALETTE, 33))
            .unwrap_or_default();
        stops
            .into_iter()
            .enumerate()
            .collect::<Vec<(usize, [u8; 3])>>()
    });

    let state = controller.state;
    let layer_list = computed(move || {
        state.with(|state| {
            state
                .layers
                .iter()
                .filter(|layer| layer.visible)
                .map(|layer| {
                    let (kind, key) = match &layer.kind {
                        MapLayerKind::BuiltinCoastlines => (SlotKind::Coastlines, layer.id.clone()),
                        MapLayerKind::ForecastField => (SlotKind::Forecast, layer.id.clone()),
                        MapLayerKind::StationOverlay => (SlotKind::Stations, layer.id.clone()),
                        MapLayerKind::LocalPmtiles { path } => (
                            SlotKind::Archive,
                            format!("{}={}", layer.id, path.to_string_lossy()),
                        ),
                    };
                    LayerSlot {
                        id: layer.id.clone(),
                        kind,
                        key,
                    }
                })
                .collect::<Vec<_>>()
        })
    });
    let archive_tiles = controller.layer_tiles;
    let marks_for_layers = marks;
    let marker_controller = controller.clone();
    let layer_stack = each(
        move || layer_list.get(),
        |slot: &LayerSlot| slot.key.clone(),
        move |item, _| {
            let slot = item.get_untracked();
            let id = slot.id.clone();
            let opacity_id = id.clone();
            let opacity = move || {
                state.with(|state| state.layer(&opacity_id).map_or(1.0, |layer| layer.opacity))
            };
            let station_ctl = marker_controller.clone();
            let content = match slot.kind {
                SlotKind::Coastlines => canvas(move || coast_painter(&viewport.get()))
                    .app_class("absolute inset-0")
                    .into_element(),
                SlotKind::Forecast => tile_images(move || slots.get(), viewport).into_element(),
                SlotKind::Archive => {
                    let layer_id = id.clone();
                    let archive_slots = computed(move || {
                        let vp = viewport.get();
                        archive_tiles.with(|store| {
                            vp.visible_tiles()
                                .into_iter()
                                .filter_map(|tile| {
                                    let stored = store.get_touch(&(
                                        layer_id.clone(),
                                        (tile.coord.z, tile.coord.x, tile.coord.y),
                                    ))?;
                                    Some(TileSlot {
                                        z: tile.coord.z,
                                        ux: tile.unwrapped_x,
                                        y: tile.coord.y,
                                        id: stored.id,
                                        image: stored.image.clone()?,
                                    })
                                })
                                .collect::<Vec<_>>()
                        })
                    });
                    tile_images(move || archive_slots.get(), viewport).into_element()
                }
                SlotKind::Stations => {
                    station_markers(move || marks_for_layers.get(), viewport, station_ctl)
                        .into_element()
                }
            };
            view()
                .app_class("absolute inset-0")
                .opacity(opacity)
                .child(content)
        },
    );

    let layout_handler = controller.clone();
    let move_handler = controller.clone();
    let leave_handler = controller.clone();
    let wheel_handler = controller.clone();
    let press_handler = controller.clone();
    let grant_handler = controller.clone();
    let drag_handler = controller.clone();
    let release_handler = controller.clone();
    let terminate_handler = controller.clone();

    let hud_ctl = controller.clone();
    let status = controller.status;
    let notice = controller.notice;
    let loading = controller.loading;
    let field = controller.field;
    let clamp_badge = controller.clamp_notice;
    let saved_ctl = controller.clone();
    let saved_path = computed(move || saved_ctl.active_saved_path());
    let stale_ctl = controller.clone();
    let is_stale = computed(move || stale_ctl.is_map_stale(true));
    let layers_open = controller.layers_open;
    let layer_panel_controller = controller.clone();
    let attribution_controller = controller.clone();
    let legend_title_ctl = controller.clone();
    let legend_lo = controller.clone();
    let legend_hi = controller.clone();

    view()
        .app_class("flex-col size-full bg-[#fbfaf6] dark:bg-[#171c19]")
        .child(map_controls::toolbar(controller.clone()))
        .child(
            view()
                .app_class("relative min-h-0 flex-1 overflow-hidden bg-[#dfe7ea] dark:bg-[#1c2326]")
                .on_layout(move |bounds: &Bounds<Points>| {
                    layout_handler.on_layout(
                        f64::from(bounds.size.width.0),
                        f64::from(bounds.size.height.0),
                    )
                })
                .child(
                    // Dedicated interactive surface for map layers so sibling HUD/Layer Stack
                    // controls never compete with responder-captured drag tracking (`FR-003`).
                    view()
                        .app_class("absolute inset-0")
                        .claim_touches(TouchClaim::on_move())
                        .keep_touches(true)
                        .on_responder_grant(move |event: &ResponderEvent| {
                            grant_handler.on_pan_grant(event)
                        })
                        .on_responder_move(move |event: &ResponderEvent| {
                            drag_handler.on_pan_move(event)
                        })
                        .on_responder_release(move |_: &ResponderEvent| release_handler.on_pan_end())
                        .on_responder_terminate(move |_: &ResponderEvent| {
                            terminate_handler.on_pan_end()
                        })
                        .on_mouse_move(move |event: &MouseEvent| move_handler.on_mouse_move(event))
                        .on_mouse_leave(move |_: &MouseEvent| leave_handler.on_mouse_leave())
                        .on_wheel(move |event: &WheelEvent| wheel_handler.on_wheel(event))
                        .on_press(move |event: &PressEvent| press_handler.on_press(event))
                        .child(layer_stack),
                )
                .child(
                    view()
                        .app_class("absolute left-2 top-2 flex-row flex-wrap gap-1")
                        .child(when(
                            move || saved_path.get().is_some(),
                            move || {
                                text(move || {
                                    format!("saved → {}", saved_path.get().unwrap_or_default())
                                })
                                .app_class(SAVED_BADGE)
                            },
                        ))
                        .child(when(
                            move || field.get().is_some() && saved_path.get().is_none(),
                            || text("received, not saved").app_class(BADGE),
                        ))
                        .child(when(
                            move || loading.get(),
                            || text("loading…").app_class(BADGE),
                        ))
                        .child(when(
                            move || is_stale.get(),
                            || text("stale").app_class(STALE_BADGE),
                        ))
                        .child(when(
                            move || clamp_badge.get().is_some(),
                            || text("valid time clamped").app_class(WARN_BADGE),
                        ))
                        .child(when(
                            move || {
                                field.get().is_some_and(|f| {
                                    f.result.source_layout == SourceLayout::DataRun
                                })
                            },
                            || text("data_run fallback").app_class(WARN_BADGE),
                        )),
                )
                .child(
                    view()
                        .app_class("absolute bottom-2 left-2 max-w-[70%] flex-col gap-1 rounded-md bg-white/90 px-2 py-1 text-[11px] text-[#24312c] dark:bg-[#202722]/90 dark:text-[#e6e9e2]")
                        .child(text(move || {
                            let layers = hud_ctl.active_display_only_layers();
                            hud_text(
                                hud_ctl.hover.get(),
                                hud_ctl.readout.get().as_ref(),
                                hud_ctl.is_map_stale(true),
                                &layers,
                            )
                        }))
                        .child(
                            text(move || status.get())
                                .app_class("text-[10px] text-[#6b786f] dark:text-[#a4ada6]"),
                        )
                        .child(text(move || notice.get()).app_class("text-[10px] text-[#9c6b2f]")),
                )
                .child(when(
                    move || layers_open.get(),
                    move || map_controls::layer_stack_panel(layer_panel_controller.clone()),
                ))
                .child(
                    view()
                        .app_class("absolute bottom-2 right-2 w-48 flex-col gap-1 rounded-md bg-white/90 p-2 text-[10px] text-[#24312c] dark:bg-[#202722]/90 dark:text-[#e6e9e2]")
                        .child(
                            view()
                                .app_class("w-full flex-row items-center justify-between")
                                .child(
                                    text(move || legend_title(&legend_title_ctl))
                                        .app_class("font-semibold tracking-wide text-[#55625a] dark:text-[#b4beb6]"),
                                ),
                        )
                        .child(
                            view()
                                .app_class("h-3 w-full flex-row overflow-hidden rounded-sm")
                                .child(each(
                                    legend_stops,
                                    |stop: &(usize, [u8; 3])| *stop,
                                    |item, _| {
                                        let (_, c) = item.get_untracked();
                                        view()
                                            .app_class("h-3 flex-1")
                                            .bg(rgb(
                                                (u32::from(c[0]) << 16)
                                                    | (u32::from(c[1]) << 8)
                                                    | u32::from(c[2]),
                                            ))
                                    },
                                )),
                        )
                        .child(
                            view()
                                .app_class("w-full flex-row justify-between")
                                .child(text(move || legend_label(&legend_lo, true)))
                                .child(text(move || legend_label(&legend_hi, false))),
                        ),
                ),
        )
        .child(
            text(move || attribution_controller.attributions().join("  ·  ")).app_class(
                "shrink-0 border-t border-[#e0ddd3] bg-[#f4f2eb] px-3 py-1 text-[9px] leading-tight text-[#6b786f] dark:border-[#343b36] dark:bg-[#1d241f] dark:text-[#a4ada6]",
            ),
        )
}

fn legend_title(controller: &MapController) -> String {
    let pal = controller.palette.get();
    let locked = locked_range(&controller.locked_min.get(), &controller.locked_max.get()).is_some();
    if locked {
        format!("{pal} · locked")
    } else {
        pal
    }
}

fn legend_label(controller: &MapController, low: bool) -> String {
    let field = controller.field.get();
    let Some(field) = field else {
        return String::new();
    };
    let _ = (controller.locked_min.get(), controller.locked_max.get());
    let (min, max) = controller.color_domain_now().unwrap_or((0.0, 0.0));
    if low {
        format!("{min:.1}")
    } else {
        format!("{max:.1} {}", field.result.stats.unit)
    }
}

fn tile_images(
    slots: impl Fn() -> Vec<TileSlot> + 'static,
    viewport: Signal<Viewport>,
) -> impl IntoElement {
    view().app_class("absolute inset-0").child(each(
        slots,
        |slot: &TileSlot| (slot.z, slot.ux, slot.y, slot.id),
        move |item, _| {
            image(item.get_untracked().image)
                .absolute()
                .w(px(TILE_SIZE as f32))
                .h(px(TILE_SIZE as f32))
                .left(move || {
                    let slot = item.get();
                    px(viewport.get().tile_screen_pos(slot.ux, slot.y).0 as f32)
                })
                .top(move || {
                    let slot = item.get();
                    px(viewport.get().tile_screen_pos(slot.ux, slot.y).1 as f32)
                })
        },
    ))
}

fn station_markers(
    marks: impl Fn() -> Vec<StationMark> + 'static,
    viewport: Signal<Viewport>,
    controller: MapController,
) -> impl IntoElement {
    let series: SeriesController = controller.series.clone();
    view().app_class("absolute inset-0").child(each(
        marks,
        |mark: &StationMark| mark.id.clone(),
        move |item, _| {
            let series = series.clone();
            let toggle_ctl = controller.clone();
            let id_for_press = item.get_untracked().id.clone();
            view()
                .absolute()
                .size(px(9.0))
                .rounded_full()
                .border_2()
                .left(move || {
                    let mark = item.get();
                    px(viewport
                        .get()
                        .lonlat_to_screen(mark.lon, mark.lat)
                        .map_or(-100.0, |p| p.0 - 4.5) as f32)
                })
                .top(move || {
                    let mark = item.get();
                    px(viewport
                        .get()
                        .lonlat_to_screen(mark.lon, mark.lat)
                        .map_or(-100.0, |p| p.1 - 4.5) as f32)
                })
                .bg(move || {
                    if series.selected_ids().contains(&item.get().id) {
                        rgb(0xbd7950)
                    } else {
                        rgb(0xffffff)
                    }
                })
                .border_color(rgb(0x24312c))
                .on_press(move |_: &PressEvent| {
                    toggle_ctl.toggle_station_selection(&id_for_press);
                })
        },
    ))
}

pub(super) fn coast_polylines(vp: &Viewport) -> Polylines {
    omo_protocol::coastlines::polylines(builtin_coastlines(), vp.geo_window(), |lon, lat| {
        vp.project_unwrapped(lon, lat)
    })
}

fn coast_painter(vp: &Viewport) -> Painter {
    let (coast, border) = coast_polylines(vp);
    let mut painter = Painter::new()
        .line_cap(LineCap::Round)
        .line_join(LineJoin::Round);
    for (lines, color, width) in [
        (border, rgb(0x8a8f87), 0.8_f32),
        (coast, rgb(0x34454d), 1.1_f32),
    ] {
        painter = painter.stroke_style(color).line_width(width).begin_path();
        for line in lines {
            for (index, (x, y)) in line.into_iter().enumerate() {
                painter = if index == 0 {
                    painter.move_to(x, y)
                } else {
                    painter.line_to(x, y)
                };
            }
        }
        painter = painter.stroke();
    }
    painter
}
