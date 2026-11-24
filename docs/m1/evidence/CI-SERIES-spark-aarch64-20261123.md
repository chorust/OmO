# T063 — `scripts/ci-verify-series.py` live run — 2026-11-23

- **Host**: `lizhan-spark` (`aarch64-unknown-linux-gnu`), 2026-11-23 UTC; locked stack as in S1.
- **Command**: `python3 scripts/ci-verify-series.py`
- **Result**: `{"readiness": "ready", "status": "passed", "run": "2026-11-23T06:00:00Z", "cycle": "06", "rows": 327, "plan": "G"}`
  (3 stations × 109 valid times; `effective_window.truncated = true`; matches Arrow file present).
- Network failures are reported as `status = "skipped(network)"` (exit 0) and are never a pass.
- `.github/workflows/ci.yml` runs the script on the `duckomo_platform` jobs (`linux_amd64`, `linux_arm64`, `osx_arm64`). The amd64 and macOS jobs have **not** been run from this host.
