# N6 follow-up on spark-aarch64/Wayland — 2026-11-18

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Session: nested Weston `13.0.0` on its X11 backend, hosted by Xvfb display `:102` at 1600×1000. The app connected to `wayland-test2` with `GDK_BACKEND=wayland`; this is a nested Wayland client, not the host's native Wayland session.
- Versions: Rust `1.100.0`, NSH `0.5.1`, GTK4 runtime `4.14.5`, Pango `1.52.1`, DuckDB `v2.0.0`, duckomo `v0.4.0` (`github-release-unsigned`, `linux_arm64`); Fcitx5 `5.1.7`, Pinyin add-on `5.1.3`, GTK4 frontend `5.1.1`; IBus `1.5.29`, libpinyin `1.15.7`.
- Commands: `dbus-run-session -- /tmp/omo-wayland/launch-n6-fcitx-current.sh` and, for the separate IBus attempt, `dbus-run-session -- /tmp/omo-wayland/launch-n6-ibus-current.sh`. In the Fcitx session Pinyin was activated with Ctrl+Space, the SQL editor received `shi`, and key `2` selected candidate 2; the Fcitx log records those key events.
- App: `target/release/omo-app`, SHA-256 `7bf424781804ec8894a488f6d41227f5b0af12ae56d2e9231bab830cb71eb0d7`.
- Overall result: **not-run for the complete N6 Wayland cell**. This follow-up verifies the Fcitx5 candidate-2 selection and commit subcase in nested Weston. The IBus candidate list/selection was not captured, and no native Wayland session was available.

## Fcitx5 / Pinyin

Fcitx5 and its GTK4 frontend were extracted under `/tmp/omo-fcitx5` and run in a private D-Bus session. The app used `GTK_IM_MODULE=fcitx5`, the extracted GTK4 module through `GTK_PATH`, `GDK_BACKEND=wayland`, and `WAYLAND_DISPLAY=wayland-test2`. The Fcitx log records activation of Pinyin, the `shi` composition keys, and key `2`.

The candidate screenshot shows the Pinyin candidate list while `shi` was in the SQL editor, including candidate 2 (`祇`). After pressing `2`, the candidate list closed and the editor displayed the committed character `祇`:

- [Candidate list](n6-spark-aarch64-wayland-20261118/01-fcitx5-shi-candidates.png), SHA-256 `9a639e94ea75fc8041691ffafbc8591b2351e1e0490bce3735d64b5cb3028ef5`.
- [Candidate 2 committed](n6-spark-aarch64-wayland-20261118/02-fcitx5-candidate-2-committed.png), SHA-256 `076d31e880f93b361379a5c67e7785e5267f6da227c94edd81183d73d004fc56`.
- [Fcitx5 log](n6-spark-aarch64-wayland-20261118/fcitx5.log), SHA-256 `6698057d6b623153f493af4390c918844b9457572c4b63caf158e1c5acefa4ab`.

The Fcitx log also reports `zwp_input_panel_v1@9: error 0: interface object already bound`, followed by `Wayland connection got error: 71` and `Connection removed`. The visible candidate selection and commit succeeded in this nested session despite that addon diagnostic. This does not establish behavior in a native Wayland desktop session.

## IBus / libpinyin

A separate nested Weston app launch used `GTK_IM_MODULE=ibus`, the extracted GTK4 IBus module, and `ibus engine libpinyin`. The app process reported `GDK_BACKEND=wayland` and `WAYLAND_DISPLAY=wayland-test2`; `/proc/<pid>/maps` showed the extracted `libim-ibus.so` loaded, and the session read back `libpinyin` as the active engine. See the [process record](n6-spark-aarch64-wayland-20261118/ibus-process.txt). The [input attempt screenshot](n6-spark-aarch64-wayland-20261118/03-ibus-input-attempt.png) shows `ENGINE READY` and CJK text inserted into the SQL editor. A slower follow-up also produced committed CJK text, but no captured frame showed the lookup candidates or a deliberate candidate selection.

A later retry extracted Ubuntu `ibus-gtk4` and `ibus-wayland` 1.5.29-2 under `/tmp/omo-ibus-wayland` without installing them. The app again loaded that GTK4 module, and the IBus daemon and libpinyin engine were running. Starting `/usr/libexec/ibus-wayland` against the nested compositor failed immediately with `zwp_input_panel_v1@4: error 0: interface object already bound`, because Weston's own virtual keyboard already owns that interface. This identifies why the candidate panel cannot attach in this nested Weston session; it does not verify IBus candidate display or selection.

## Limits

No clipboard readback or timing/RSS measurements were taken. Extracted packages were not installed system-wide. The nested Weston result does not replace native spark Wayland, Linux x86_64 Wayland/X11, or macOS 14 arm64 runs. N6 remains open for those cells and for IBus candidate selection in Wayland.
