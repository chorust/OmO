# M0 N-checklist

Record each applicable item in `docs/m0/acceptance.md` and write an evidence file under
`docs/m0/evidence/`. A missing host or prerequisite is recorded as `not-run (<reason>)`.
Only the user may classify N-results as blocking under FR-022.

## N1 — Build

- [x] Build with `cargo build --workspace --locked` on macOS 14 arm64, Linux x86_64, and spark aarch64. Spark passes locally. Linux x86_64 and macOS arm64 (`macos-15` runner) pass in [GitHub CI](evidence/CI-CROSS-PLATFORM-20261123.md) (2026-11-23, Constitution 1.3.0).
- [x] Record exact Rust, NSH, GTK/Pango/GStreamer versions and required package list for the available Spark host run; see the [Spark N1 record](evidence/N1-spark-aarch64-n_a-20261119.md). Linux x86_64 and macOS remain not-run.
- [x] On spark, confirm a source build after prerequisites are available; locked debug/test/release builds pass against installed system packages, without `/tmp/gtkdev` (see [N1](evidence/N1-spark-aarch64-n_a-20261119.md)).

## N2 — DataTable

- [ ] Load a 1,000,000 × 20 generated result through visible-range paging.
- [ ] Scroll and sort; confirm sort requests re-fetch pages.
- [ ] Record frame time p50/p95, main-thread stall, and RSS.

Spark X11 and nested Weston Wayland recorded million-row paging, scrolling, and a successful sort re-fetch. The nested Wayland run also recorded rolling frame p50/p95 fields, app/worker RSS, and a maximum GTK main-context callback gap of 80.842 ms; the earlier X11 instrumented run recorded 192.268 ms. Both callback gaps include scheduling and are not input latency. Native Wayland, Linux x86_64, and macOS cells remain open. See [X11 evidence](evidence/N2-spark-aarch64-x11-20261117.md) and [nested Wayland evidence](evidence/N2-spark-aarch64-wayland-20261118.md).

## N3 — Plot

- [ ] Plot 20 series × 10,000 points.
- [ ] Record render time and measured series/point cap.
- [ ] Confirm any downsampling is labelled.

Spark X11 and the nested Weston Wayland client record the 20×10,000 labelled chart, hover cross-line/dot, and five Plot-tab activation-to-visible trials. Maximum conservative upper bounds were 281.08 ms on X11 and 292.74 ms in nested Wayland; the measured M0 cap is 20×10,000. Linux x86_64 and macOS arm64 N3 cells remain open, and no native Wayland desktop was available. See [X11 evidence](evidence/N3-spark-aarch64-x11-20261117.md) and [nested Wayland evidence](evidence/N3-spark-aarch64-wayland-20261117.md).

## N4 — Editor

- [ ] Complete multiline SQL/highlighting across all required platforms and sessions.
- [ ] Complete undo/redo and copy/paste across all required platforms and sessions.

Spark X11 and the nested Weston Wayland client pass for multiline editing, keyword/number/comment and string highlighting, undo/redo, and clipboard copy/paste. The Wayland process was verified on `GDK_BACKEND=wayland` and `wayland-test2`; this is nested compositor evidence, not a native desktop run. Linux x86_64 and macOS cells remain not-run. See [X11 evidence](evidence/N4-spark-aarch64-x11-20261117.md) and [nested Wayland evidence](evidence/N4-spark-aarch64-wayland-20261117.md).

## N5 — File dialogs

- [ ] Test open and save dialogs on macOS and Linux with portal available and unavailable.
- [ ] Confirm cancellation returns no selection.
- [ ] Confirm a portal error provides manual path entry.

Spark X11 open/save cancellation passes with portal services available and unavailable; the validation seam also verifies manual path acceptance. In nested Weston, successful portal open/save requests and cancellation were previously recorded, though the portal UI rendered on host X11. The 2026-11-20 Wayland-client run used a mock portal service returning D-Bus `org.freedesktop.portal.Error.Failed` for open and save, confirmed GTK-native chooser fallback inside Weston, cancelled both dialogs, and accepted the absolute station CSV path after the open error. A separate private D-Bus session with no service directories returned `ServiceUnknown`; GTK displayed nested open/save choosers, and cancelling each left no selection or file action. Native Wayland desktop, Linux x86_64, and macOS remain not-run. See [current nested Weston evidence](evidence/N5-spark-aarch64-wayland-20261120.md), [successful portal Wayland-client run](evidence/N5-spark-aarch64-wayland-20261118.md), [X11 fallback rerun](evidence/N5-spark-aarch64-x11-20261118.md), and [portal-available X11 evidence](evidence/N5-spark-aarch64-x11-20261117.md).

## N6 — CJK IME

- [x] On spark X11, verify composition, candidate 2 selection, commit, and copied text in the Omobench SQL editor with ibus-libpinyin; see [N6 evidence](evidence/N6-spark-aarch64-x11-20261117.md).
- [ ] Test macOS system Pinyin.
- [ ] Test ibus and fcitx5 on Linux Wayland and X11; both Pinyin subcases pass on spark X11. The 2026-11-18 nested Weston Fcitx run displayed and selected candidate 2, then committed it, while logging a Wayland input-panel error. The 2026-11-21 isolated nested Weston IBus run reproduced `zwp_input_panel_v1@4: error 0: interface object already bound` under default `weston-keyboard`; launching `ibus-ui-gtk3 --enable-wayland-im` as Weston `[input-method]` avoided that error and key `2` committed `时` via `libpinyin` (visible after preedit flush), but Weston 13 (`zwp_text_input_v1`-only) never displayed the committed `zwp_input_panel_surface_v1` candidate popup for GTK4 4.14.5 (`zwp_text_input_v3`-only). See [IBus Weston verification](evidence/N6-spark-aarch64-wayland-20261121.md), [Fcitx Wayland follow-up](evidence/N6-spark-aarch64-wayland-20261118.md), and [prior attempt](evidence/N6-spark-aarch64-wayland-20261117.md); IBus Wayland candidate display, native Wayland, Linux x86_64, and macOS cells remain open.

## N7 — Threading

- [ ] Push progress at 50 Hz for 10 minutes while using the UI across the required platform/session matrix.
- [ ] Include worker kill/restart and record maximum main-thread callback interval.

Spark X11 has a completed ten-minute rerun on the current release app: 604.260 seconds elapsed between markers, 30,021 displayed sequence increments occurred, tabs were switched during the query, and killing the worker led to generation 2 with progress continuing. The dual-clock GTK main-context probe recorded a maximum callback gap of 64.945 ms (39.945 ms beyond its nominal 25 ms interval); both clocks measured the same 701.795-second full capture. This includes scheduling delay and is not CPU-only blocking or input-to-render latency. Linux x86_64, macOS, and full spark Wayland N7 cells remain not-run. See [N7 v2](evidence/N7-v2-spark-aarch64-x11-20261118.md).

## N8 — Memory

- [ ] Record empty-workbench RSS.
- [ ] Record RSS during N2 and assess per-node memory effects.

## N9 — Dependencies

- [x] Capture `cargo tree -p omo-app` and each core crate for each target. Refreshed with Rust 1.100.0 and `--locked` on 2026-11-19 after manifest cleanup; these are dependency trees, not host builds. See the N9 records and current tree outputs linked from [acceptance.md](acceptance.md).
- [ ] Confirm app has no DuckDB, GPUI, Tauri, or QuickJS and core crates have no UI dependencies.
