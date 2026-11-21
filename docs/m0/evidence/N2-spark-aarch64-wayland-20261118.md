# N2 on spark-aarch64/Wayland — 2026-11-18

- Host / OS / session: `lizhan-spark`, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`; nested Weston 13.0.0 on Xvfb `:102`, Wayland socket `wayland-test2`, 1600×1000. The app used `GDK_BACKEND=wayland`. A native Wayland desktop was unavailable; this result applies to the nested Weston session.
- Versions: Rust `1.100.0`, GTK `4.14.5`, NSH `0.5.1`, DuckDB `v2.0.0`, duckomo `v0.4.0`, channel `github-release-unsigned`, platform `linux_arm64`.
- Fixture: `/tmp/omo-accept/n2-table.arrow`, 1,000,000 rows × 20 columns (`series_00`…`series_19`), 123 record batches, 162,640,410 bytes, SHA-256 `26adc15234ea8ef36b44601f8c4b2dfd104ece34149aba8a04bd012c19639d4a`.
- Result: **pass for the nested Weston N2 scope**. The Results tab showed the million-row, 20-column fixture. Scrolling advanced the visible range from the first page to rows around 1,967 and to rows around 1,573 in separate captures. A separate focused sort run used `SELECT * FROM range(1000000) AS t(series_00)` so the fixture header and query schema matched. Sorting `series_00` dispatched a worker preview; the task panel reported one request and `Preview 1 completed with 100000 rows`, and the table displayed ascending values. The 100,000-row refreshed query reflects the app's preview row limit; it is separate from the 1,000,000-row paging fixture.

## Measurements

- The measured scrolling session retained 29 `nsh::fps` log samples. Across those rolling per-second windows, the displayed `frame` p50 field ranged from 5.08 to 7.18 ms, p95 from 6.24 to 16.81 ms, and maximum frame-work from 6.33 to 71.67 ms. NSH-reported app RSS ranged from 112.4 to 228.6 MB. These are ranges across rolling samples, not one workload-wide percentile.
- The separate 200 ms `/proc` sampler recorded a peak app RSS of 223,264 KiB and worker RSS of 73,404 KiB. This is process RSS sampled every 200 ms; the worker was idle during fixture paging. The sort run is retained separately and is not included in this peak.
- The GTK main-context probe recorded 1,218 callbacks over 30.941 seconds. The maximum callback gap was **80.842 ms** (55.842 ms above its nominal 25 ms interval); the maximum sampled ongoing gap was 48.785 ms. Monotonic and wall-clock coverage matched, with zero wall-clock regressions and a clean `stopped` terminal record. These callback gaps include callback work and scheduling delay; they are not CPU-only blocking or input-to-render latency.
- App SHA-256: `7bf424781804ec8894a488f6d41227f5b0af12ae56d2e9231bab830cb71eb0d7`; worker SHA-256: `f98819131ae5f984dde4aef0b5f2febb4427b3ae03f0a4e4a472942a32385e1b`.

## Artifacts

- [First page, measured scrolling run](n2-spark-aarch64-wayland-20261118/first-page-main-thread-run.png)
- [After scrolling, measured run](n2-spark-aarch64-wayland-20261118/scrolled-main-thread-run.png)
- [First page, sort run](n2-spark-aarch64-wayland-20261118/first-page-sort-run.png)
- [After scrolling, sort run](n2-spark-aarch64-wayland-20261118/scrolled-sort-run.png)
- [Sorted worker re-fetch](n2-spark-aarch64-wayland-20261118/sorted-refetch.png)
- [GTK main-context JSONL](n2-spark-aarch64-wayland-20261118/main-thread.jsonl)
- [RSS samples](n2-spark-aarch64-wayland-20261118/rss.csv)
- [Measured session log](n2-spark-aarch64-wayland-20261118/measured-session.log)
- [Sort session log](n2-spark-aarch64-wayland-20261118/sort-session.log)

The native Wayland desktop, Linux x86_64 Wayland/X11 sessions, and macOS sessions remain untested. This nested session result does not close those matrix cells or the full N2–N8 gate.
