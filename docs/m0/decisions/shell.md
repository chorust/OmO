# NSH shell decision for M0

## Final decision (user, FR-022, 2026-11-23)

**Accept NSH 0.5.1 for M0. Do not trigger the §7.3 GPUI Kit fallback.** The user classified the remaining N6 Wayland item as non-blocking: the IBus candidate popup does not map in nested Weston 13, because Weston implements only `zwp_text_input_v1` and GTK4 implements only `zwp_text_input_v3`. This closes SPIKE-1 and EXIT-3. Non-reference platforms are closed by [GitHub CI/release evidence](../evidence/CI-CROSS-PLATFORM-20261123.md) under Constitution 1.3.0. See the [final EXIT-3 record](../evidence/EXIT-3-spark-aarch64-n-a-20261123.md). The provisional analysis below is kept as the evidence basis. Its "remain open" statements are superseded by this decision.

## Provisional decision (2026-11-20)

Retain NSH 0.5.1 for the M0 implementation. Do not trigger the §7.3 GPUI Kit fallback on the evidence from the verified spark sessions. This is a **provisional implementation decision**, not a claim that the full N1–N9 matrix or M0 exit has passed. Revisit it if an outstanding host, session, IME, or packaging run establishes a blocking result.

## FR-022 classification

The user-approved D8 examples identify three blocking conditions. None has been observed in the tested scope:

| Blocking example | Evidence and classification |
| --- | --- |
| Spark source build failure | Required GTK4/Pango/GStreamer development packages are installed in the host package database; the current Rust 1.100.0 locked host debug/release builds and 52 workspace tests pass without `/tmp/gtkdev`. No Spark build failure was observed. Linux x86_64 and macOS host builds remain not-run; see [current N1 evidence](../evidence/N1-spark-aarch64-current-20261120.md). |
| Unusable GTK N6 IME path with no viable fix | Spark X11 IBus/libpinyin and Fcitx5 Pinyin both completed composition, candidate selection, and commit. Nested Weston Wayland Fcitx candidate selection/commit was observed with an input-panel error. The 2026-11-21 isolated nested Weston IBus run confirmed `libpinyin` composition and candidate-2 (`时`) commit via `GTK_IM_MODULE=ibus`, but disproved visible candidate-panel display under Weston 13 (default `weston-keyboard` hits `zwp_input_panel_v1 ... interface object already bound`, and replacing it with `ibus-ui-gtk3 --enable-wayland-im` still leaves the panel unmapped because Weston 13 only implements `zwp_text_input_v1` while GTK4 4.14.5 only implements `zwp_text_input_v3`). That is a nested Weston 13 compositor-protocol limitation rather than a demonstrated native Wayland GTK failure; IBus Wayland candidate selection, native Wayland, Linux x86_64, and macOS coverage remain open. |
| Unusable million-row N2 table | The 1,000,000×20 result loaded, paged, and scrolled on spark X11 and in a nested Weston Wayland client. Sorting caused a worker re-fetch and ascending values were displayed in both. These tests show a usable table in those sessions; other platforms and native Wayland remain untested. |

Other N-results are recorded as limitations or not-run items:

- **N2 responsiveness and memory:** the X11 callback probe recorded a maximum callback gap of 192.268 ms (167.268 ms over its nominal 25 ms interval); the nested Weston Wayland probe recorded 80.842 ms (55.842 ms over nominal). Both include scheduling delay and callback work and do not measure input-to-render latency or CPU-only blocking. The separate captures record app/worker RSS and are linked below. No numeric N2 stall threshold is specified.
- **N3 plot:** the 20×10,000 chart rendered with its cap label and hover interaction in spark X11 and nested Weston Wayland. Five activation trials yielded maximum conservative upper bounds of 281.08 ms and 292.74 ms respectively to first visible chart content. These are activation-to-visible bounds, not isolated application-only render durations. Set the M0 cap to 20 numeric series × 10,000 points (`MAX_PLOT_SERIES = 20`, `MAX_PLOT_POINTS = 10_000`), the measured workload size.
- **N4 editor:** multiline editing, highlighting, undo/redo, and copy/paste passed in spark X11 and nested Weston Wayland. Linux x86_64 and macOS remain not-run.
- **N5 dialogs:** Spark X11 open/save cancellation passed with portal services available and unavailable. In nested Weston, successful portal requests were previously cancelled while the portal UI rendered on host X11. The 2026-11-20 Wayland-client run verified mocked `org.freedesktop.portal.Error.Failed` replies and GTK-native open/save chooser fallback inside Weston; cancellation returned no selection or file action, and manual path entry was accepted after the open error. A second private D-Bus session with no portal service verified `ServiceUnknown` open/save fallback and cancellation. Native Wayland desktop, Linux x86_64, and macOS remain not-run; see [current nested Weston evidence](../evidence/N5-spark-aarch64-wayland-20261120.md).
- **N6 IME:** both Pinyin subcases pass on spark X11. Nested Weston Fcitx candidate 2 selection and commit were observed alongside a Wayland input-panel error. The 2026-11-21 isolated nested Weston IBus run reproduced `zwp_input_panel_v1` `already bound` under default `weston-keyboard`, and with `ibus-ui-gtk3 --enable-wayland-im` as Weston `[input-method]` committed `时` on key `2` (after preedit flush), but Weston 13 (`zwp_text_input_v1`-only) never mapped the candidate popup for GTK4 4.14.5 (`zwp_text_input_v3`-only). Visible IBus Wayland candidate selection therefore remains `not-run` pending a native `zwp_text_input_v3`/IBus-capable Wayland desktop; Linux x86_64 and macOS remain not-run.
- **N7 threading:** the spark X11 ten-minute run covered 604.260 seconds, displayed 30,021 counter increments, allowed tab switching, and restarted the worker at generation 2. Its maximum GTK main-context callback gap was 64.945 ms (39.945 ms over nominal). The nested Weston Wayland run covered 610.146 seconds, displayed 30,304 increments, allowed Plot/Results switching, and restarted to generation 2; its maximum callback gap was 45.072 ms (20.072 ms over nominal). The Wayland results and plot views were empty during this progress workload; no completed query result is claimed. Both probe gaps include scheduling delay and callback work. The full captures had matching monotonic and wall-clock coverage and no recorded wall-clock regressions. No numeric stall threshold is imposed by M0.
- **N8 memory:** empty-workbench and N2 RSS are recorded for spark X11 and nested Weston Wayland. The qualitative assessment is that `DataTableRows` receives only a requested visible page plus 48 overscan rows; the app does not build UI rows for the full million-row result. The current result has 20 headers, and the source/task panels are fixed shell content. Expected node growth therefore tracks the visible table window and fixed UI, though the approximate 2.7 KB/node issue could matter for unusually tall viewports or expanded per-row UI. No runtime node census is available, so the RSS measurements cannot quantify NSH's per-node cost.
- **N9:** dependency boundaries pass for the inspected trees and local checks; target dependency metadata is not a substitute for host builds.

All callback intervals and memory values apply only to the named runs and host. Linux x86_64 and macOS 14 arm64 host evidence remains unavailable. The nested Weston results are Wayland-client evidence, not a native Wayland desktop run. Keep their matrix cells and the other unverified sessions `not-run` as recorded in [acceptance.md](../acceptance.md).

## Gate status

- No user-approved blocking example has been observed in the verified spark scope.
- Retain NSH 0.5.1 for this implementation; do not offer NSH↔GPUI embedding.
- No later compatible NSH release was evaluated. The locked 0.5.1 API supported table sorting after Omobench set `DataTableColumn.sortable = true`.
- No GPUI Kit fallback was triggered or implemented.
- The full N1–N9 platform/session matrix, T019 package installation, cross-platform channel repeats, and EXIT-3 remain open. This decision does not close M0.

## Evidence

- [Acceptance matrix](../acceptance.md)
- [N1 build and environment](../evidence/N1-spark-aarch64-n_a-20261117.md)
- [N2 table and sort, spark X11](../evidence/N2-spark-aarch64-x11-20261117.md); [nested Weston Wayland](../evidence/N2-spark-aarch64-wayland-20261118.md)
- [N3 plot](../evidence/N3-spark-aarch64-x11-20261117.md); [nested Weston Wayland](../evidence/N3-spark-aarch64-wayland-20261117.md)
- [N4 editor, spark X11](../evidence/N4-spark-aarch64-x11-20261117.md); [nested Weston Wayland](../evidence/N4-spark-aarch64-wayland-20261117.md)
- [N5 portal and fallback rerun](../evidence/N5-spark-aarch64-x11-20261118.md); [nested Weston portal rerun](../evidence/N5-spark-aarch64-wayland-20261118.md); [nested Weston real D-Bus error and no-portal fallback](../evidence/N5-spark-aarch64-wayland-20261120.md)
- [N6 IME, spark X11](../evidence/N6-spark-aarch64-x11-20261117.md); [nested Weston Wayland Fcitx follow-up](../evidence/N6-spark-aarch64-wayland-20261118.md); [nested Weston Wayland IBus verification](../evidence/N6-spark-aarch64-wayland-20261121.md)
- [N7 dual-clock threading, spark X11](../evidence/N7-v2-spark-aarch64-x11-20261118.md); [nested Weston Wayland](../evidence/N7-spark-aarch64-wayland-20261118.md)
- [N8 memory, spark X11](../evidence/N8-spark-aarch64-x11-20261117.md); [nested Weston Wayland](../evidence/N8-spark-aarch64-wayland-20261118.md)
- [N9 dependencies](../evidence/N9-spark-aarch64-n_a-20261116.md)
