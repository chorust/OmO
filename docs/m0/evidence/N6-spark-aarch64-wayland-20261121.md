# N6 on spark-aarch64/wayland (isolated nested Weston IBus verification) — 2026-11-21

- Host / OS / session: `lizhan-spark`, Ubuntu 24.04, `aarch64`, kernel `6.17.0-1029-nvidia`; isolated Xvfb `:115` (`1600×1000`), private D-Bus session, unique runtime directory `/tmp/omo-n6-ibus-20261121/runtime`, and isolated nested Weston `13.0.0` (`--backend=x11 --renderer=pixman`, socket `wayland-n6-ibus-20261121`). Existing displays (`:99`–`:108`), `wayland-test2`, and running `omo-app`/`omo-worker` processes were untouched.
- Versions: Rust `1.100.0 (0ef1bc386 2026-11-09)`, `libgtk-4-1` `4.14.5+ds-0ubuntu0.10`, `libpango-1.0-0` `1.52.1+ds-1build1`, `ibus` `1.5.29-2`, `ibus-libpinyin` `1.15.7-1build2`, `libpinyin-data` `2.8.1-1build3`, extracted Ubuntu `ibus-gtk4` / `ibus-wayland` `1.5.29-2` under `/tmp/omo-ibus-gtk4` and `/tmp/omo-ibus-wayland`, and extracted Weston `13.0.0` under `/tmp/omo-wayland`. No system packages were installed.
- App tested: `target/release/omo-app`, SHA-256 `7bf424781804ec8894a488f6d41227f5b0af12ae56d2e9231bab830cb71eb0d7`.
- Extracted component SHA-256:
  - `/tmp/omo-ibus-gtk4/root/usr/lib/aarch64-linux-gnu/gtk-4.0/4.0.0/immodules/libim-ibus.so`: `9796aa32ad11fcdea23ee1082a265a298406eff353eb05df8afce4ca532e4075`
  - `/tmp/omo-ibus-wayland/root/usr/libexec/ibus-wayland`: `a816541f82a3e6cf506858a16a0e14fc90ce83cdb045686a0e0473ae92d0e89d`
  - `/tmp/omo-wayland/weston-run/usr/bin/weston`: `4a8db312f9c92f5f27937311cd3d962557a2060ed7a6718c183f9e207eb0ae62`
- Verdict: **not-run (disproved visible IBus candidate display in nested Weston 13; IBus subcase and complete N6 Wayland cell remain not-run).** Neither Fcitx5 nor X11 candidate-panel evidence is substituted.

## Summary of findings

Three isolated sub-experiments were executed against the unique nested Weston session (`:115`, `wayland-n6-ibus-20261121`):

1. **Default Weston (`--no-config`, `weston-keyboard` active) + `ibus-wayland` reproduces the `zwp_input_panel_v1` conflict.**
   - Default Weston launches `/usr/libexec/weston-keyboard`, which binds `zwp_input_panel_v1`.
   - Running `/tmp/omo-ibus-wayland/root/usr/libexec/ibus-wayland --verbose` against `WAYLAND_DISPLAY=wayland-n6-ibus-20261121` failed immediately (exit code `133` / `SIGTRAP`) with:
     ```text
     zwp_input_panel_v1@4: error 0: interface object already bound
     ** (process:1983563): ERROR **: 14:01:32.710: Lost connection to wayland compositor
     ```
   - Weston logged `[14:01:32.710] libwayland: error in client communication (pid 1983563)`.

2. **Custom Weston `[input-method]` running `/usr/libexec/ibus-ui-gtk3 --enable-wayland-im` eliminates the `already bound` conflict, but Weston 13 still never displays the candidate panel.**
   - Configured Weston via `[input-method] path=.../weston-im-ibus.sh` to launch `/usr/libexec/ibus-ui-gtk3 --enable-wayland-im --verbose` directly (replacing `weston-keyboard`, matching `/usr/share/applications/org.freedesktop.IBus.Panel.Wayland.Gtk3.desktop`) against a private `ibus-daemon --daemonize --panel=disable --xim --replace --verbose` with active engine `libpinyin`.
   - Launched `target/release/omo-app` as a pure Wayland client (`GDK_BACKEND=wayland`, `XDG_SESSION_TYPE=wayland`, `WAYLAND_DISPLAY=wayland-n6-ibus-20261121`, `GTK_IM_MODULE=ibus`, `GTK_PATH=/tmp/omo-ibus-gtk4/root/usr/lib/aarch64-linux-gnu/gtk-4.0`, no `DISPLAY` in `/proc/<pid>/environ`). `/proc/<pid>/maps` confirmed `libim-ibus.so` and `libibus-1.0.so.5.0.529` loaded, and `ibus engine` read back `libpinyin`.
   - With `WAYLAND_DEBUG=1`, `ibus-ui-gtk3 --enable-wayland-im` cleanly bound `zwp_input_panel_v1` (no `interface object already bound` error).
   - Typing `s`, `h`, `i` in OmO's SQL editor triggered `libpinyin` `UpdateLookupTable` (`["是", "时", "时间", "十四时三分", "14时3分", "14:03"]`) and `UpdatePreeditText("是")` over the private IBus D-Bus. `ibus-ui-gtk3` created a Wayland surface, bound it as an overlay input panel, and attached/committed the rendered candidate buffer:
     ```text
     -> wl_compositor@4.create_surface(new id wl_surface@19)
     -> zwp_input_panel_v1@20.get_input_panel_surface(new id zwp_input_panel_surface_v1@22, wl_surface@19)
     -> zwp_input_panel_surface_v1@22.set_overlay_panel()
     -> wl_surface@19.attach(wl_buffer@24, 0, 0)
     -> wl_surface@19.commit()
     ```
   - **Result on screen:** `01-composing-shi-no-panel.png` shows `ENGINE READY` and underlined preedit `是` in line 1 of the SQL editor, with **no visible candidate popup anywhere on the 1600×1000 Weston output**. Because candidates were not visible during composition, the N6 pass requirement for visible candidate display and deliberate visual candidate choice is not met.
   - Pressing `2` caused `libpinyin` to emit `CommitText("时")`, `HideLookupTable`, and `HidePreeditText`; `PostProcessKeyEvent` returned `byte 99` (`'c'`, `"时"`) and `byte 104` (`'h'`, `""`). `omo-app` logged `IBUS-WARNING: ibus_input_context_post_process_key_event: Type 'h' is not supported.`, leaving the stale underlined preedit `是` visible in `02-after-key2-stale-preedit.png` (byte-identical to `01-composing-shi-no-panel.png`). Flushing preedit via `ibus engine xkb:us::eng` revealed the committed candidate-2 character `时` on line 1 in `03-after-preedit-flush-committed.png`.

3. **Root cause verified via GTK4 `WAYLAND_DEBUG=1` and Weston `--xwayland` check.**
   - Launching `omo-app` without `GTK_IM_MODULE` under `WAYLAND_DEBUG=1` (`03-gtk4-no-immodule-wayland-debug.log`) confirms Weston `13.0.0` advertises `zwp_input_panel_v1`, `zwp_input_method_v1`, and `zwp_text_input_manager_v1` — **not** `zwp_text_input_manager_v3`. `libgtk-4.so.1` (`4.14.5`) only implements `zwp_text_input_manager_v3` / `zwp_text_input_v3` and never binds `zwp_text_input_manager_v1`.
   - In Weston 13 (`desktop-shell.so`), `zwp_input_panel_surface_v1` surfaces are only mapped when a client activates `zwp_text_input_v1::show_input_panel`. Because `omo-app` (GTK4) talks to IBus over D-Bus via `GTK_IM_MODULE=ibus` and cannot activate `zwp_text_input_v1`, Weston never maps `ibus-ui-gtk3`'s committed `zwp_input_panel_surface_v1` surface.
   - Running `ibus-ui-gtk3` on Wayland without `--enable-wayland-im` fails with `CRITICAL: ibus_wayland_im_set_surface: assertion 'wlim' failed` and commits an unroled `wl_surface` that Weston cannot display.
   - Attempting Weston `--xwayland` (`[xwayland] path=/usr/bin/Xwayland`) failed at module load (`03-weston-xwayland-error.log`): `Failed to load module: libxcb-composite.so.0: cannot open shared object file: No such file or directory`.

## Retained evidence and SHA-256

All files are stored under [n6-spark-aarch64-wayland-20261121](n6-spark-aarch64-wayland-20261121/):

- [01-default-weston.log](n6-spark-aarch64-wayland-20261121/01-default-weston.log) — SHA-256 `2376cc0da5489e046e7ff8ff0701313ce366f92963021790d58f6ce05ee0f6da`
- [01-default-weston-ibus-wayland.log](n6-spark-aarch64-wayland-20261121/01-default-weston-ibus-wayland.log) — SHA-256 `674d7ed4e2ee41607e9c32caa34e241a73ebf9b75f222955c92c590969bfffd9`
- [02-weston.log](n6-spark-aarch64-wayland-20261121/02-weston.log) — SHA-256 `1724a10e417417b297b6d49baeb9b0eb55b65b038c382e43a7782a9c61ffc016`
- [02-process-and-ibus-state.txt](n6-spark-aarch64-wayland-20261121/02-process-and-ibus-state.txt) — SHA-256 `7c3a5061ab48303bfefbbb289ad0efaaebdaff0cb3043f86200c2508248ee31d`
- [02-ibus-panel-wayland-debug.log](n6-spark-aarch64-wayland-20261121/02-ibus-panel-wayland-debug.log) — SHA-256 `900d352c9c7ceb33a45bac13a6b1e22d9951ed00ac7ece1563132bba9c76bb89`
- [02-ibus-dbus-trace.txt](n6-spark-aarch64-wayland-20261121/02-ibus-dbus-trace.txt) — SHA-256 `9f4f832b6b67082eda5fad1ec76d9c7cafcbfee6577b9967c354e514636a1ef4`
- [02-omo-app.log](n6-spark-aarch64-wayland-20261121/02-omo-app.log) — SHA-256 `6c4550b18e4ac216ddbf0797ae8ecc5feaecd7d40ff2b44524164eac524ca6c1`
- [01-composing-shi-no-panel.png](n6-spark-aarch64-wayland-20261121/01-composing-shi-no-panel.png) — SHA-256 `5220625cf3264f3aa58a4cc6a30162b2eae9de8500071afabfe9797b207f45cb`
- [02-after-key2-stale-preedit.png](n6-spark-aarch64-wayland-20261121/02-after-key2-stale-preedit.png) — SHA-256 `5220625cf3264f3aa58a4cc6a30162b2eae9de8500071afabfe9797b207f45cb`
- [03-after-preedit-flush-committed.png](n6-spark-aarch64-wayland-20261121/03-after-preedit-flush-committed.png) — SHA-256 `f15c372371732d63a3e9eae2975e397d0d1e6e9e474a9c30b8b72d2e775a7e80`
- [03-gtk4-no-immodule-wayland-debug.log](n6-spark-aarch64-wayland-20261121/03-gtk4-no-immodule-wayland-debug.log) — SHA-256 `db40abf5ac9d6fb23b4feb5e8dd11121fa393d6cefbad10be640b6999cd7b889`
- [03-weston-xwayland-error.log](n6-spark-aarch64-wayland-20261121/03-weston-xwayland-error.log) — SHA-256 `b614b361aaa3eb3dca07929a5bc14707b66a24e7053e427bc3b47b980383577e`

## Limitations and open scope

- The IBus Wayland candidate-display and visual candidate-selection subcase remains **not-run** in nested Weston 13 due to the Weston 13 (`zwp_text_input_v1` / `zwp_input_panel_v1`) vs. GTK4 4.14.5 (`zwp_text_input_v3`-only) protocol mismatch, the default `weston-keyboard` `zwp_input_panel_v1` exclusivity error, and missing `libxcb-composite.so.0` for Weston XWayland.
- Verifying IBus Wayland candidate display and visual candidate selection on Spark requires a native Wayland desktop compositor that supports `zwp_text_input_v3` or integrates IBus panel rendering (e.g., GNOME Shell / Mutter or KDE KWin), which is unavailable in this headless session.
- Native spark Wayland, Linux x86_64 Wayland/X11, and macOS 14 arm64 N6 cells remain `not-run`.
