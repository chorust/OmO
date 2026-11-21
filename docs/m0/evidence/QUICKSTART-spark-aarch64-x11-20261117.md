# M0 quickstart smoke run — spark-aarch64/x11 — 2026-11-17

- Host: lizhan-spark, Ubuntu 24.04, Linux aarch64, X11.
- Toolchain: Rust `1.100.0`; NSH `0.5.1`; DuckDB `v2.0.0`; duckomo `v0.4.0`.
- Scope: the six sections of `specs/001-m0-skeleton-engine/quickstart.md` were exercised on the available spark host/session. Platform or interaction gaps remain recorded as `not-run`; this smoke run does not close the full feature acceptance matrix.

## Quickstart steps

| Step | Result | Evidence and limits |
|---|---|---|
| 1. Build and boundaries | Pass on spark with `/tmp/gtkdev` pkg-config metadata | Locked workspace build/test and release build passed; boundary check and its 10-case self-test passed. This is not a system package installation. See [N1](N1-spark-aarch64-n_a-20261117.md), [T044 verification](T044-worker-plot-sampling-20261117.md), and [Linux prerequisites](../linux-prereqs.md). |
| 2. Artifacts and bootstrap | Pass for the selected Linux arm64 channel; alternate signed route failed explicitly | GitHub-release-unsigned passed the independent golden query and 100/100 fresh connections. Community-signed failed both tested local load forms; no unsigned fallback occurred. R4 and no-network checks passed. Other platforms and cross-channel equality remain not-run. See [EXIT-1](EXIT-1-spark-aarch64-n-a-20261117.md), [EXIT-5](EXIT-5-spark-aarch64-n-a-20261117.md), [SPIKE-3](SPIKE-3-signed-local-spark-aarch64-20261116.md), and [SPIKE-4](SPIKE-4-spark-aarch64-n-a-20261117.md). |
| 3. Worker supervision | Pass for the spark harness and live worker scope | Crash injection recovered 20/20 fake-worker crashes; the supervisor tests passed 5/5; live query, Arrow result, interrupt, and worker restart evidence is retained. Cancel levels 2/3 in the fake harness exercise state-machine paths, not real drain timing. See [EXIT-4](EXIT-4-spark-aarch64-n-a-20261117.md), [T039](T039-harness-20261116.md), and [T034](T034-worker-live-spark-aarch64-20261116.md). |
| 4. NSH workbench | Partial; per-item records govern | Spark X11 N2 paging/scrolling/sort, N3 rendering, N4 editor interactions, N5 portal-available cancellation, N7 progress exercise, and N8 RSS evidence are linked from [acceptance.md](../acceptance.md). Separate N2 runs recorded GTK main-context callback gaps. The later X11 and nested Weston N7 probes recorded maximum callback gaps of 64.945 ms and 45.072 ms respectively, including scheduling delay; neither measures input-to-render latency or CPU-only blocking. N3 isolated application-only render duration and behavior after a real portal D-Bus error remain unverified. The N5 manual path field accepted a local path on spark X11. N3 pointer hover cross-line/dot interaction is recorded in its evidence. N6 IBus/libpinyin and Fcitx5 Pinyin composition, candidate selection, and commit pass on spark X11 using extracted GTK4 frontends. The full Wayland N6 cell remains not-run after a partial nested Weston Fcitx attempt. Nested Weston N2 and N7 workload evidence and N8 memory evidence were added on 2026-11-18; N5 Wayland no-portal/manual-fallback and N6 IBus candidate selection remain open. Linux x86_64 and macOS cases remain not-run. See [Wayland evidence](N6-spark-aarch64-wayland-20261117.md). |
| 5. Nearest-series spike | Pass for both plans on the fixture | Plan A and B each matched all 10 semantic oracle checks for 27 rows; Plan B was selected for eligible regular spherical grids. See [EXIT-2](EXIT-2-spark-aarch64-20261117.md) and [decision](../decisions/series-plan.md). |
| 6. Export kill injection | Pass on spark | Steps 1 and 2 reported incomplete; step 3 reported finished. Linux x86_64 and macOS remain not-run. See [EXIT-6](EXIT-6-spark-aarch64-n-a-20261117.md). |

## Final local verification

```text
RUSTUP_TOOLCHAIN=1.100.0 PKG_CONFIG_PATH=/tmp/gtkdev/pc cargo build --locked --workspace   PASS
RUSTUP_TOOLCHAIN=1.100.0 PKG_CONFIG_PATH=/tmp/gtkdev/pc cargo test --locked --workspace    PASS
RUSTUP_TOOLCHAIN=1.100.0 PKG_CONFIG_PATH=/tmp/gtkdev/pc cargo build --release --locked --workspace PASS
RUSTUP_TOOLCHAIN=1.100.0 cargo fmt --all -- --check                                            PASS
scripts/check-boundaries.sh                                                                    PASS
scripts/check-boundaries.sh --self-test                                                        PASS (10 injected cases)
python3 scripts/verify-duckling-snapshot.py                                                     PASS (25 files)
```

The first verification build reported unused `sha2` in `omo-project` and `anyhow` / `tokio` in `omo-worker`; those manifest entries and lockfile references were removed. The final locked workspace tests and release build then passed without dependency warnings. GTK4/GStreamer development packages remain absent system-wide; `sudo -n` requires a password and the earlier `pkexec` install attempt timed out, as recorded in [Linux prerequisites](../linux-prereqs.md).
