# Cross-platform GitHub CI and release evidence — 2026-11-23

This file closes the non-reference (`linux-x86_64`, `macos14-arm64`) M0 acceptance cells under
the Constitution 1.3.0 cross-platform policy (2026-11-21). Full interactive GUI acceptance (N2–N8)
stays on the Spark reference host (`linux_arm64`). The other target platforms are accepted on
passing `ci.yml` and `release.yml` runs. Dedicated hardware is needed only if CI or release
exposes a platform-specific issue. On 2026-11-23 the user decided to close T029, T040, T049,
T050, and T061 with this CI evidence.

## Runs

| run | workflow | trigger | head SHA | result |
|---|---|---|---|---|
| [45104328529](https://github.com/chorust/OmO/actions/runs/45104328529) | CI (`.github/workflows/ci.yml`) | push `main`, 2026-11-23T03:09:55Z | `a778808e951ab41b46014e6daa1da5559bd014bd` | success |
| [45031395194](https://github.com/chorust/OmO/actions/runs/45031395194) | Release assets (`.github/workflows/release.yml`) | release `v0.1.0-m0`, 2026-11-22T14:10:48Z | `434c7a0e901e823e7c8230533e5cba512808ee0f` | success |

`a778808` differs from `434c7a0` only in README documentation, so the CI run covers the released
source tree.

## CI jobs (run 45104328529)

All jobs used Rust `1.100.0` and ran `cargo build --workspace --locked`, `cargo test --workspace
--locked`, `scripts/check-boundaries.sh`, and `scripts/check-boundaries.sh --self-test`.

| job id | runner / image | target | Duckomo platform | workspace tests | Duckomo bootstrap + live worker |
|---|---|---|---|---|---|
| 137462031081 | `ubuntu-24.04` / ubuntu24 20261116.376 | `x86_64-unknown-linux-gnu` | `linux_amd64` | 63 passed, 0 failed | passed |
| 137462030879 | `ubuntu-24.04-arm` / Ubuntu2404-Arm64 20261116.188 | `aarch64-unknown-linux-gnu` | `linux_arm64` | 63 passed, 0 failed | passed |
| 137462030573 | `macos-15` / macos-15-arm64 20261118.0550 | `aarch64-apple-darwin` | `osx_arm64` | 62 passed, 0 failed | passed |
| 137462030872 | `macos-15-intel` / macos-15 20260824.0482 | `x86_64-apple-darwin` | — (build-only) | 62 passed, 0 failed | not applicable |

`scripts/ci-verify-duckomo.py --repeat 3` does the following for each locked platform:
1. Downloads both channel artifacts and checks size and SHA-256 against `artifacts/duckomo.lock.json`.
2. Runs `omo-probe --channel <channel> --repeat 3` with the locked golden check.
3. Runs `scripts/check-worker-live.py` with the unsigned artifact.

The output was identical on all three locked platforms except for the `platform` field:

```json
{
  "status": "passed",
  "platform": "linux_amd64 | linux_arm64 | osx_arm64",
  "channels": {
    "community-signed":        { "golden_mode": "locked", "attempts": 3, "golden_sha256": "bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f" },
    "github-release-unsigned": { "golden_mode": "locked", "attempts": 3, "golden_sha256": "bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f" }
  },
  "live_worker": { "result_rows": 1000, "cancellation": "interrupt" }
}
```

The golden hash equals the locked `golden.result_sha256` for all six channel × platform entries.
It also matches the Spark 100/100 result.

## Release jobs (run 45031395194)

| job id | job | asset | bytes |
|---|---|---|---|
| 137235563586 | Build linux-x86_64 | `omobench-v0.1.0-m0-linux-x86_64.tar.gz` | 39015620 |
| 137235563509 | Build linux-aarch64 | `omobench-v0.1.0-m0-linux-aarch64.tar.gz` | 37505105 |
| 137235563607 | Build macos-aarch64 | `omobench-v0.1.0-m0-macos-aarch64.tar.gz` | 32544626 |
| 137235563152 | Build macos-x86_64 | `omobench-v0.1.0-m0-macos-x86_64.tar.gz` | 35339343 |

## What CI exercised and what is accepted by policy

These are two separate kinds of result. Cells marked `ci-accepted` were closed by the
Constitution 1.3.0 policy and the user's decision; no target-host run was made for them.

| item | CI-exercised on linux-x86_64 / macOS arm64 | acceptance result |
|---|---|---|
| N1 build | Locked debug build and workspace tests | `pass (GitHub CI)` |
| N2–N8 GUI | Not exercised: no interactive desktop session; the GUI crate builds and its unit tests pass | `ci-accepted` (interactive acceptance on Spark only) |
| SPIKE-1 NSH gate | Build/test only | `ci-accepted` |
| SPIKE-2 station-series plans | Worker series-plan contract tests; no oracle comparison run | `ci-accepted` |
| SPIKE-3 signed channel | Community-signed artifact hash checked, bootstrapped, and golden-verified 3/3; no wrong-key or cache-isolation check | `ci-accepted` (negative-path checks verified on Spark only) |
| SPIKE-4 R4 / spatial | No `--r4` run | `ci-accepted` |
| SPIKE-5 dependency coexistence | Boundary check and self-test passed | `pass (GitHub CI)` |
| EXIT-1 channel reliability | Both channels 3/3, not 100/100 | `ci-accepted` |
| EXIT-2 station-series oracle | No oracle comparison run | `ci-accepted` |
| EXIT-3 NSH decision | Build/test only | `ci-accepted` (still gated on Spark EXIT-3) |
| EXIT-4 crash/cancel | Supervisor crash, heartbeat, stale-generation, and cancel-escalation tests; live worker interrupt cancellation | `ci-accepted` (no `omo-harness` run) |
| EXIT-5 golden match | Both channels match the locked golden hash | `pass (GitHub CI)` (3 repeats) |
| EXIT-6 export commit | Export unit tests only; no `omo-harness export-kill` run | `ci-accepted` |

## Known scope differences

- The macOS arm64 runner is `macos-15`, not macOS 14. The matrix label `macos14-arm64` is kept
  for continuity.
- Linux x86_64 Wayland and X11 desktop sessions were not exercised.
- Repetition counts are 3, not 100.
- If a future CI or release run fails on a non-reference platform, the affected cells reopen and
  need dedicated hardware.
