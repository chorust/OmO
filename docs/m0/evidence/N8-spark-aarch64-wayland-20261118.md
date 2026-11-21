# N8 on spark-aarch64/Wayland — 2026-11-18

- Host / OS / session: `lizhan-spark`, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`; nested Weston 13.0.0 on Xvfb `:102`, Wayland socket `wayland-test2`. The app used `GDK_BACKEND=wayland`. A native Wayland desktop was unavailable.
- Versions: Rust `1.100.0`, NSH `0.5.1`, DuckDB `v2.0.0`, duckomo `v0.4.0`, channel `github-release-unsigned`, platform `linux_arm64`.
- Empty-workbench state: release app and ready worker, no station CSV, and neither `OMOBENCH_FIXTURE_RESULT` nor `OMOBENCH_INITIAL_SQL` set. The editor shows the default `SELECT 1 AS value`. Screenshot: [empty workbench](n8-spark-aarch64-wayland-20261118/empty.png).
- Result: **pass for recording the empty-workbench baseline and the separate N2 per-process RSS peaks.** Per-node memory cost is not isolated.

At `2026-11-18T17:28:48+0800`, while the screenshot showed engine generation 1 and `ENGINE READY`, `/proc/<pid>/status` reported:

| Process | PID | RSS |
| --- | ---: | ---: |
| `omo-app` | 1759154 | 95,800 KiB |
| `omo-worker` | 1759841 | 54,640 KiB |
| Combined | — | 150,440 KiB |

The separate nested Weston N2 run sampled RSS every 200 ms. Its peaks were app 223,264 KiB and worker 73,404 KiB while paging the 1,000,000 × 20 fixture. See [N2 evidence](N2-spark-aarch64-wayland-20261118.md). The empty and N2 samples use different runs, so their difference is not a per-node attribution and does not measure NSH issue #1345 (~2.7 KB/node). No runtime node census was collected.

Qualitative assessment: the app passes the current visible row range plus 48 overscan rows into `DataTableRows` and updates that data from the requested page (`crates/omo-app/src/ui/table_view.rs`); it does not construct a million-row UI list. The table has 20 result headers, and the source/task panels are fixed shell content. Therefore NSH node growth should track the visible table window and fixed UI rather than total result row count. The 2.7 KB/node issue could still make unusually tall viewports or expanded per-row UI costly. Without an observed NSH node count, these RSS samples cannot quantify that cost or establish a per-node memory budget.

- App SHA-256: `7bf424781804ec8894a488f6d41227f5b0af12ae56d2e9231bab830cb71eb0d7`
- Worker SHA-256: `f98819131ae5f984dde4aef0b5f2febb4427b3ae03f0a4e4a472942a32385e1b`
- Machine-readable sample: [rss.json](n8-spark-aarch64-wayland-20261118/rss.json)
- Session log: [session.log](n8-spark-aarch64-wayland-20261118/session.log)

Linux x86_64, macOS, and a native Wayland desktop remain untested.
