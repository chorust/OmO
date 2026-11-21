# N5 manual-path fallback from a nested Wayland client on spark-aarch64 — 2026-11-19

- Host / OS / kernel: `lizhan-spark`, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Session: Weston 13.0.0 nested on Xvfb `:102`, Wayland socket `wayland-test2`. The app used `GDK_BACKEND=wayland`; this is nested Weston client evidence, not a native Wayland desktop run.
- Versions: Rust `1.100.0`, NSH `0.5.1`, GTK4 `4.14.5`, Pango `1.52.1`, DuckDB `v2.0.0`, duckomo `v0.4.0` (`github-release-unsigned/linux_arm64`).
- Validation build: `PKG_CONFIG_PATH=/tmp/gtkdev/pc RUSTUP_TOOLCHAIN=1.100.0 cargo build -p omo-app --release --locked --features dialog-test-seams` — passed. App SHA-256: `d4fd98fb02ff28430a3bc22ec2bd8fc4ec9876634966cbaf7d9b33c54237190f`.
- Launch: `dbus-run-session -- /tmp/omo-wayland/launch-n5-manual-20261119.sh`; environment set `GDK_BACKEND=wayland`, `WAYLAND_DISPLAY=wayland-test2`, `GTK_USE_PORTAL=0`, `GIO_USE_PORTALS=0`, and `OMOBENCH_TEST_PORTAL_ERROR=1`.

## Verified subcase

The Wayland client started with `ENGINE READY`. Clicking **Browse for CSV** ran the validation seam and displayed: “File chooser unavailable (simulated portal failure: no file chooser service). Enter a path below.” The absolute path `/home/blizhan/repo/github/OmO/fixtures/stations/m0.csv` was entered and **Use path** selected. The workbench then reported: “Path saved in this window only; CSV ingestion is not connected yet.” This verifies the application error callback and manual-path UI from a Wayland client.

- [Initial Wayland workbench](n5-spark-aarch64-wayland-20261119/initial.png), SHA-256 `15b17c31e5cad48f20a724ee389f8931c7691e88553e6310a30493b72495fd98`.
- [Simulated picker error and fallback prompt](n5-spark-aarch64-wayland-20261119/simulated-portal-error.png), SHA-256 `208d5b76c13010fdb455cc2d3278d56d51cba82f90a30e1cef100a30844fab10`.
- [Entered path](n5-spark-aarch64-wayland-20261119/manual-path-entered.png), SHA-256 `e5df41734f8efcbe12f60f6225d1fc81f0214bd378266c1044892f9968b49be0`.
- [Accepted path state](n5-spark-aarch64-wayland-20261119/manual-path-accepted.png), SHA-256 `39b56471e92249f8ffc50d3d79fa00d9a71789a341ff466db8430be2481a56b0`.

## Limits

This uses the `dialog-test-seams` validation hook; it does not inject an actual portal D-Bus error or exercise GTK's response to one. A separate launch of the normal app binary (`8c335c28c76570cfe3aaa56e0042231669734d3679dd2d8e05293d2b0e840bc8`) used `GTK_USE_PORTAL=0` and `GIO_USE_PORTALS=0`; clicking Browse did not produce a verifiable chooser or cancellation state in the nested display. The [capture](n5-spark-aarch64-wayland-20261119/no-portal-browse-no-chooser.png), SHA-256 `4546e4c541bc53019c4282b083d0982c169c8d85647bbe6557c1d57a0f5ff3c2`, shows the workbench after the attempt; no cancellation is claimed, and save was not exercised. The portal open/save cancellation evidence from 2026-11-18 used a Wayland client, but the chooser windows rendered on host X11 outside Weston. Therefore the complete spark Wayland N5 cell remains **not-run**. Linux x86_64, macOS, and native Wayland desktop cases remain open.
