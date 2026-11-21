# N7 on spark-aarch64/Wayland — 2026-11-18

- Host / OS / session: `lizhan-spark`, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`; nested Weston 13.0.0 on Xvfb `:102`, Wayland socket `wayland-test2`, 1600×1000. The app used `GDK_BACKEND=wayland`. A native Wayland desktop was unavailable; this result applies to the nested Weston client.
- Versions: Rust `1.100.0`, GTK `4.14.5`, NSH `0.5.1`, DuckDB `v2.0.0`, duckomo `v0.4.0`, channel `github-release-unsigned`, platform `linux_arm64`.
- Workload: `OMOBENCH_PROGRESS_HZ=50` and `SELECT sum(hash(a * 2000000 + b)) FROM range(2000000) t1(a) CROSS JOIN range(2000000) t2(b)`. `OMOBENCH_MAIN_THREAD_LOG` enabled the GTK main-context probe. `NSH_FPS=1` was also set.
- Result: **pass for the nested Weston N7 scope.** The UI stayed operable during the ten-minute progress run and after the worker was killed and restarted.

The [start marker](n7-spark-aarch64-wayland-20261118/start-time.txt) at `2026-11-18T17:31:40.666877931+08:00` follows the [active preview](n7-spark-aarch64-wayland-20261118/start-progress.png), which shows sequence 2,024 and worker generation 1. The [end marker](n7-spark-aarch64-wayland-20261118/end-time.txt) at `2026-11-18T17:41:50.812736444+08:00` follows the [ten-minute capture](n7-spark-aarch64-wayland-20261118/ten-minute.png), which shows sequence 32,328 on the Results tab. The markers span **610.146 seconds**; the displayed counter advanced by **30,304**. Counter differences show displayed activity, not an independently counted event-delivery rate. During the interval the UI switched to [Plot](n7-spark-aarch64-wayland-20261118/plot-interaction.png) and [Results](n7-spark-aarch64-wayland-20261118/results-interaction.png) while progress continued.

Worker PID 1763224 was then killed. The app [restarted at generation 2](n7-spark-aarch64-wayland-20261118/restarted.png) with preview progress continuing, and the [Plot tab remained usable](n7-spark-aarch64-wayland-20261118/post-restart-plot.png). File → Quit stopped the app. The [session log](n7-spark-aarch64-wayland-20261118/session.log) contains no panic or `RefCell already borrowed`.

The [schema v2 JSONL](n7-spark-aarch64-wayland-20261118/main-thread.jsonl) contains 27,178 GTK main-thread callbacks over 694.146313 seconds. Separate monotonic and wall-clock readings both report a maximum callback gap of **45.072 ms**, or **20.072 ms** beyond the nominal 25 ms interval. The maximum sampled ongoing gap was 36.133 ms. There were zero wall-clock regressions, coverage matched, and the terminal record says `stopped`. This gap includes callback work and scheduling delay; it is neither CPU-only blocking nor input-to-render latency.

Validate with:

```sh
python3 scripts/summarize-main-thread-probe.py docs/m0/evidence/n7-spark-aarch64-wayland-20261118/main-thread.jsonl --min-coverage-seconds 600
```

- App SHA-256: `7bf424781804ec8894a488f6d41227f5b0af12ae56d2e9231bab830cb71eb0d7`
- Worker SHA-256: `f98819131ae5f984dde4aef0b5f2febb4427b3ae03f0a4e4a472942a32385e1b`
- JSONL SHA-256: `b73d2364e8cd3d30095e1dba92ce2131f1e3436afcb7160aa2ef6329e07e15d4`
- Session log SHA-256: `651cea46d25f6f1a94b8b0c6e76ec243f61f60bc1035a3256567235cef7f5adb`

Linux x86_64, macOS, and a native Wayland desktop remain untested. This nested session does not close those cells.
