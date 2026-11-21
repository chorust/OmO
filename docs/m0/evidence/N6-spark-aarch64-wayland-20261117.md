# N6 on spark-aarch64/wayland — 2026-11-17

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Session: nested Weston `13.0.0` using its X11 backend on Xvfb `:101`, 1600×1000. Omobench connected to the nested Wayland socket with `GDK_BACKEND=wayland` and `WAYLAND_DISPLAY=wayland-test`. This is a Wayland client session hosted by X11, not the host's native desktop session.
- Versions: Rust `1.100.0`, NSH `0.5.1`, GTK4 runtime `4.14.5`; Fcitx5 `5.1.7`, Pinyin add-on `5.1.3`, GTK4 frontend `5.1.1`.
- App: `target/release/omo-app`, SHA-256 `408618c446268bbb0d30eda32189e17071011020064b1df96be78f3840ab218b`.
- Command(s): Weston and Fcitx5 packages were extracted under `/tmp`; Omobench ran in a private D-Bus session with `GTK_IM_MODULE=fcitx5` and `GTK_PATH=/tmp/omo-fcitx5/root/usr/lib/aarch64-linux-gnu/gtk-4.0`. The IBus attempt used `GTK_IM_MODULE=ibus`, the extracted GTK4 IBus module, and `ibus engine libpinyin`.
- Result: **not-run (the complete Wayland cell is not established).** The nested Fcitx5 attempt displayed a Pinyin candidate list; clicking the second candidate dismissed the list and left a committed character in the editor. IBus inserted Chinese text after typing `shi`, but no candidate list/selection was captured. The nested Weston session does not substitute for the native Wayland desktop session.
- Measurements: no frame-time, stall, or RSS measurements were taken in this session.
- Output / artifacts: [Fcitx5 composition and candidates](n6-spark-aarch64-wayland-20261117/01-fcitx5-candidates.png), [Fcitx5 after candidate click](n6-spark-aarch64-wayland-20261117/02-fcitx5-committed.png), [IBus input attempt](n6-spark-aarch64-wayland-20261117/03-ibus-input-attempt.png), [Fcitx5 log](n6-spark-aarch64-wayland-20261117/fcitx5.log), and [Weston log](n6-spark-aarch64-wayland-20261117/weston.log).

## Observations

- `/proc/<omo-app-pid>/maps` showed the extracted `libim-fcitx5.so` loaded while the Omobench process had `GDK_BACKEND=wayland`. The Fcitx candidate list was visible below the SQL editor preedit `shi`; after clicking candidate 2, the preedit and list disappeared and one character remained in the editor.
- The Fcitx log also reports `zwp_input_panel_v1@9: error 0: interface object already bound`, followed by `Wayland connection got error: 71` and `Connection removed`. The visible editor interaction is recorded, but this compositor-integration diagnostic remains a limitation.
- For IBus, `/proc/<omo-app-pid>/maps` showed the extracted `libim-ibus.so` loaded, and `ibus engine` reported `libpinyin`. Chinese text appeared after typing `shi`, but a candidate list and deliberate candidate selection were not established.
- A clipboard check after the Fcitx candidate click returned a literal escape representation and was not treated as verification. The recorded commit evidence is the editor state shown in the screenshot.
- `OMOBENCH_BOOTSTRAP_CONFIG` was unset, so the engine status showed failure; the SQL editor remained available and is the surface tested.

This partial nested-session attempt does not close N6, T048, or the full N1–N9 shell gate. The N6 IBus Wayland candidate-selection path, native spark Wayland session, Linux x86_64 cases, and macOS case remain open.
