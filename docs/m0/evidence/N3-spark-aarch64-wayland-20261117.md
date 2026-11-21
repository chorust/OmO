# N3 on spark-aarch64/wayland — 2026-11-17

- Host / OS: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Session: the host had no native Wayland desktop. The Omobench process ran as a Wayland client under nested Weston 13.0.0, using Weston’s X11 backend on Xvfb `:102` (1600×1000) and Wayland socket `wayland-test2`. Client environment included `GDK_BACKEND=wayland`, `GSK_RENDERER=cairo`, and `WAYLAND_DISPLAY=wayland-test2`. Weston and its runtime files were extracted under `/tmp/omo-wayland`; no system package installation is claimed.
- Versions: Rust `1.100.0`, GTK4 `4.14.5`, NSH `0.5.1`, Weston `13.0.0`, DuckDB `v2.0.0`, duckomo `v0.4.0`, `github-release-unsigned`, `linux_arm64`.
- App binary SHA-256: `508062f63348612804140e924a25253fa92f8ece93cde7fc05d115698934df8e`.
- Input: `/tmp/omo-accept/n3-plot.arrow`, 10,000 rows × 20 numeric columns in two record batches, 1,630,338 bytes, SHA-256 `3b284ec7a08071d377d6d5ed337444f7228ce562914ec437c9d828ba27f7a2fc`.
- Result: **pass for the N3 plot workload in this nested Weston Wayland session.** The plot showed its 20-series/10,000-point cap label; pointer hover displayed the cross-line and dot. The N3 native Wayland desktop run remains unavailable, and this result does not close the spark Wayland N2–N8 matrix.

## Visible-render timing and cap

The fixture result was loaded before timing. The timer covers selecting the Plot tab through visible chart content; it excludes query execution and worker result preparation. Input was sent to the Wayland client through the outer X11-backed Weston window. The pointer was pre-positioned before each timed click.

The screen was sampled with `PIL.ImageGrab` over the outer Xvfb coordinates `(650,316)-(801,557)`. The dark SQL view had RGB `(23,28,25)` at both the grid probe `(650,316)` and series probe `(800,556)`. A visible plot sample had grid RGB `(228,225,216)` and series RGB `(118,119,115)`. Each trial recorded the last dark capture and first visible capture. The lower bound is last-dark capture start minus click-command return; the upper bound is first-visible capture end minus click-command start. These conservative intervals include input and capture overhead. The gap between the last dark capture end and first visible capture start was 2.56–10.94 ms.

| Trial | Click command (ms) | Conservative interval (ms) | Observation gap (ms) |
|---:|---:|---:|---:|
| 1 | 1.98 | 216.61–271.69 | 7.96 |
| 2 | 7.24 | 252.77–277.10 | 2.56 |
| 3 | 9.80 | 189.82–257.02 | 9.73 |
| 4 | 6.28 | 227.34–292.74 | 10.94 |
| 5 | 7.81 | 190.81–235.68 | 6.15 |

The median per-trial upper bound was 271.69 ms; the maximum was 292.74 ms. This is a Plot-tab activation-to-visible-content bound for this nested session, not an application-only render duration or a tail-latency guarantee. Retain the measured M0 cap of 20 numeric series × 10,000 points per series; the cap constants are `MAX_PLOT_SERIES = 20` and `MAX_PLOT_POINTS = 10_000` in `crates/omo-protocol/src/plot.rs`. Larger limits are not benchmarked.

The [before image](n3-spark-aarch64-wayland-20261117/plot-render-before.png) records the SQL view; the [after image](n3-spark-aarch64-wayland-20261117/plot-render-after.png) shows the chart and all 20 series labels; the [hover image](n3-spark-aarch64-wayland-20261117/plot-render-hover.png) shows the cross-line and hover dot. The five trials are in [CSV](n3-spark-aarch64-wayland-20261117/plot-activation-timing.csv).

| Artifact | SHA-256 |
|---|---|
| `plot-activation-timing.csv` | `2060057d22026bb079613f126a51c5d13f6ece182dba2c835c6cdaacf9954b96` |
| `plot-render-before.png` | `b52a0e9c1553666a8ac0cdb732c6bdd01ee8c9495ca06fbfe47ac3cdd1a84ff9` |
| `plot-render-after.png` | `60d50deb09bb8d627fd66331e8f0a5fc01797484fac60554e049bc1a7784abe2` |
| `plot-render-hover.png` | `9d7050ab7eaa66a0b485c1ac74ab8c5e21b8d3de88eab1f108ec724d578e4e05` |

The earlier nested Weston N6 attempt used a separate session on `:101`; its input-panel error and incomplete IME interaction remain in [N6 Wayland evidence](N6-spark-aarch64-wayland-20261117.md).
