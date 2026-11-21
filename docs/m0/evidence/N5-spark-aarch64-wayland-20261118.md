# N5 portal open/save cancellation from a nested Wayland client on spark-aarch64 — 2026-11-18

- Host: `lizhan-spark`, Ubuntu 24.04.4 LTS, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Session: Weston 13.0.0 nested on Xvfb `:102`, socket `wayland-test2`; no native Wayland desktop was available.
- App: `target/release/omo-app`, SHA-256 `7bf424781804ec8894a488f6d41227f5b0af12ae56d2e9231bab830cb71eb0d7`; worker SHA-256 `f98819131ae5f984dde4aef0b5f2febb4427b3ae03f0a4e4a472942a32385e1b`.
- Runtime: `GDK_BACKEND=wayland`, `WAYLAND_DISPLAY=wayland-test2`, `XDG_RUNTIME_DIR=/tmp/omo-wayland/runtime2`, `GTK_USE_PORTAL=1`, `GIO_USE_PORTALS=1`; GTK4 `4.14.5`, NSH `0.5.1`, DuckDB `v2.0.0`, duckomo `v0.4.0` from `github-release-unsigned/linux_arm64`.

## Verified portal subcases

The app's Browse for CSV action opened the GNOME file chooser through the app's private D-Bus session. The Export action, after `SELECT 1 AS value` completed, opened the portal save chooser with `omobench-results.csv` as its suggested name. The portal backend window was displayed on the host X11 display (`:1`), outside nested Weston, because its activated desktop service did not associate a parent Wayland window. The application itself was confirmed as a Wayland client in the nested Weston session. This evidence therefore covers Wayland-client portal requests and responses; it does not claim native Wayland-desktop dialog placement.

Both choosers were cancelled. After open cancellation the workbench showed `No station CSV selected`; after save cancellation the one-row preview remained complete, the chooser closed, and the status remained `No file action yet`.

Evidence images:

- [Open chooser header](n5-spark-aarch64-wayland-20261118/open-dialog-header.png)
- [Open cancellation state](n5-spark-aarch64-wayland-20261118/open-cancelled-workbench.png)
- [Save chooser header](n5-spark-aarch64-wayland-20261118/save-dialog-header.png)
- [Save cancellation state](n5-spark-aarch64-wayland-20261118/save-cancelled-workbench.png)

The open chooser was activated by `DISPLAY=:102 xdotool mousemove 160 418 click 1`; cancellation used `DISPLAY=:1 xdotool mousemove 47 54 click 1`. The save path was exercised by `DISPLAY=:102 xdotool key ctrl+Return`, clicking Export at `(1413,135)`, and cancelling at `(47,54)` on `:1`. The private D-Bus log records activation of `org.freedesktop.portal.Desktop`, `org.freedesktop.impl.portal.desktop.gnome`, and `org.freedesktop.impl.portal.desktop.gtk` for this app session.

## Scope and remaining work

This is partial evidence only. No-portal chooser behavior and the manual-path UI after a portal failure were not exercised from this Wayland client. The portal backend window was rendered outside Weston, so this does not establish the full native Wayland-session N5 flow. Linux x86_64 and macOS remain not-run. Keep the full N5 Wayland matrix cell open.

Screenshot SHA-256:

| File | SHA-256 |
|---|---|
| `open-dialog-header.png` | `2de352c0993b45acfddf761da4537b84de4324bec49114a620411fe920380b8b` |
| `open-cancelled-workbench.png` | `9b243d0f2e6570d80426c18073ed4793768d12176482bdf1ad23a2f115906ff2` |
| `save-dialog-header.png` | `a2905d6e1069d085882cf38bafb9c2c799c1691713e6414e6030b0d4e8a83909` |
| `save-cancelled-workbench.png` | `e113d2a8b09b96352c70b57682d40611eaf7e96e8c1f756abd419dc480b84799` |
