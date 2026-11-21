# N5 on spark-aarch64/wayland — 2026-11-17

- Host / OS / session: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`; nested Weston 13.0.0 on Xvfb `:102`, Wayland socket `wayland-test2`. No native Wayland desktop was available.
- Application: `target/release/omo-app`, SHA-256 `508062f63348612804140e924a25253fa92f8ece93cde7fc05d115698934df8e`; the process used `GDK_BACKEND=wayland` and `XDG_RUNTIME_DIR=/tmp/omo-wayland/runtime2`.
- Result: **not-run. No N5 UI action was verified in this nested session.** The file chooser, save chooser, cancellation, and manual path behavior remain untested here.

## Input-path investigation

The app was visible in the Weston output. Attempts to activate “Browse for CSV” and enter a manual path with `xdotool` did not produce a verified UI state change. Pointer movement was visible, but the path field and status remained unchanged; this is not evidence that the controls received the clicks or keys. The `wtype` 0.4-3 binary was extracted under `/tmp/omo-wayland`; it reported `Compositor does not support the virtual keyboard protocol`. `orca-ide computer` reported `unsupported_capability` because its Python runtime could not load the required GTK/AT-SPI modules. The system Python could import extracted `python3-pyatspi`, but the app's exposed accessibility tree did not include its editor or source controls.

The nested session's D-Bus log shows portal services activating during application startup. No file-chooser request or portal dialog was verified, so portal availability for N5 is not inferred. No dialog was cancelled and no manual path was claimed as accepted.

See the [nested Weston N4 report](N4-spark-aarch64-wayland-20261117.md) for the compositor and application environment; it demonstrates N4 only and does not establish N5 behavior.
