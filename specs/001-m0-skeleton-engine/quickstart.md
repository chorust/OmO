# Quickstart: validating M0

This guide defines the validation runs and their evidence outputs. Record each run according to
`contracts/acceptance-record.md`; unavailable hosts or sessions remain explicitly `not-run`.

## Prerequisites (per host)
- Rust `1.100.0` from `rust-toolchain.toml`.
- On Linux, GTK4/Pango/GStreamer development packages (see `docs/m0/linux-prereqs.md`). When
  system packages are unavailable, the documented `/tmp/gtkdev` extraction can supply build
  metadata with `PKG_CONFIG_PATH=/tmp/gtkdev/pc`; this does not count as system installation.
- For N6, ibus or fcitx5 with a usable Chinese engine; for N5, a desktop with a portal and one
  without.
- On macOS 14+ arm64, Xcode CLT and a pinyin input source.
- Network access to fetch Duckomo artifacts (or an already verified local copy). The signed channel uses a private local repository and does not fetch Duckomo during bootstrap.

## 1. Build and boundaries (US1, N1, N9)
```bash
RUSTUP_TOOLCHAIN=1.100.0 cargo build --workspace --locked
RUSTUP_TOOLCHAIN=1.100.0 cargo test  --workspace --locked
scripts/check-boundaries.sh             # expect: all checks empty/true
scripts/check-boundaries.sh --self-test # expect: every injected violation detected
```

If GTK4 or GStreamer development packages are not installed system-wide but the extraction
workaround is available, prefix the Cargo commands with `PKG_CONFIG_PATH=/tmp/gtkdev/pc`. Record
that as a workaround, not as package installation.

## 2. Artifacts and bootstrap (US2)
```bash
scripts/fetch-duckomo.sh community-signed        <platform>   # prints lock entry; review diff
scripts/fetch-duckomo.sh github-release-unsigned <platform>
cargo run -p omo-worker --bin omo-probe --locked -- --channel community-signed --artifact <community-artifact> --repeat 100
cargo run -p omo-worker --bin omo-probe --locked -- --channel github-release-unsigned --artifact <release-artifact> --repeat 100
cargo run -p omo-worker --bin omo-probe --locked -- --channel <channel> --artifact <artifact> --r4
cargo run -p omo-worker --bin omo-probe --locked -- --channel <channel> --artifact <artifact> --no-network
```
For `community-signed`, the `.gz` input is verified and staged in a private repository whose
public key is pinned in `artifacts/duckomo.lock.json`; installed bytes are checked against the
locked decompressed hash before `LOAD`. Unsigned and built-in community loading stay disabled,
and repository access closes after the signed load. `scripts/fetch-duckomo.sh` verifies the
community signature against that pinned key. A signature or repository failure never retries
through the unsigned channel.

Expected: the bootstrap report shows DuckDB v2.0.0, a matching platform, Duckomo v0.4.0, and
the golden hash equal to the lock in all 100 runs. Both channels give an identical golden
result on each target platform. → `docs/m0/decisions/channel.md`, `r4.md`.

## 3. Worker supervision (US3)
```bash
cargo test -p omo-app --locked --no-default-features --features in-process-engine --test supervisor
cargo run  -p omo-app --bin omo-harness --locked -- crash-inject --iterations 20
cargo run  -p omo-app --bin omo-harness --locked -- cancel --levels 1,2,3
```
Expected: recovery every time; 0 stale-generation results accepted; the cancel level that
ended each task is logged.

## 4. NSH workbench (US4, N2–N8)
```bash
cargo run -p omo-app --release --locked
```
Work through `docs/m0/n-checklist.md` on each platform/session. The table fixture is
`omo-harness gen-result --rows 1000000 --cols 20`; the plot fixture is
`omo-harness gen-result --series 20 --points 10000`. Set `OMOBENCH_FIXTURE_RESULT` to the
fixture's Arrow IPC path to seed the initial app result. Set `NSH_FPS=1` to enable frame telemetry.
The UI sends `progress_hz` from `OMOBENCH_PROGRESS_HZ`; the worker defaults to 2 Hz and caps the
requested rate at 200 Hz. On Linux, set `OMOBENCH_MAIN_THREAD_LOG` to an absolute JSONL path to
record callback gaps from a 25 ms GLib timeout on GTK's native main context using both
monotonic and wall-clock timestamps. The callback updates atomic counters only; a background
writer flushes one snapshot per second. `max_callback_gap_us` and
`max_callback_wall_gap_us` include callback work, the nominal interval, and scheduler delay;
the corresponding overrun fields subtract the nominal 25 ms interval. These values measure
observed main-thread callback gaps, not CPU-only blocking or input-to-render latency. The
summarizer reports both clock coverages and their difference. Summarize and validate a capture
with `python3 scripts/summarize-main-thread-probe.py <log>`. For probe
validation only, build with `--features main-thread-probe-test` and set
`OMOBENCH_MAIN_THREAD_TEST_STALL_MS=250`; this injects one controlled delay into the GLib main
context. Do not set the test variable during acceptance workloads. For Linux picker-error-path validation
only, build with `--features dialog-test-seams` and set `OMOBENCH_TEST_PORTAL_ERROR=1`;
this routes the station picker through the app's error callback so the manual path fallback can be
checked. This seam does not replace testing a real portal implementation. Run the ten-minute
`omo-harness progress --hz 50 --minutes 10` workload only when its required measurement has not
already been captured. → `docs/m0/decisions/shell.md`.

## 5. Nearest-series spike (US5)
```bash
cargo run -p omo-worker --bin omo-series-spike --locked -- --plan A --artifact <release-artifact>
cargo run -p omo-worker --bin omo-series-spike --locked -- --plan B --artifact <release-artifact>
```
Expected: a per-item semantic comparison plus bytes/time. → `docs/m0/decisions/series-plan.md`.

## 6. Export kill-injection (US6)
```bash
cargo run -p omo-app --no-default-features --features in-process-engine --bin omo-harness --locked -- export-kill --at step1
cargo run -p omo-app --no-default-features --features in-process-engine --bin omo-harness --locked -- export-kill --at step2
cargo run -p omo-app --no-default-features --features in-process-engine --bin omo-harness --locked -- export-kill --at step3
```
Expected verdicts: incomplete, incomplete, finished. A cross-filesystem target gives an explicit
error.
