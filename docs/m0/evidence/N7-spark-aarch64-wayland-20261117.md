# N7 on spark-aarch64/wayland — 2026-11-17

- Host / OS / session: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`; a native Wayland desktop session was unavailable. Nested Weston 13.0.0 on Xvfb `:102` was used for N3/N4 and an unverified N5 input attempt; a separate `:101` session was used for partial N6. No N7 workload was run.
- Versions: Rust `1.100.0`, GTK4 `4.14.5`, NSH `0.5.1`, Weston `13.0.0`; no N7 workload was run.
- Command(s): none for N7; see [N3](N3-spark-aarch64-wayland-20261117.md), [N4](N4-spark-aarch64-wayland-20261117.md), [N5](N5-spark-aarch64-wayland-20261117.md), and [N6](N6-spark-aarch64-wayland-20261117.md) for the nested sessions.
- Result: **not-run (no N7 workload was run; N3/N4 and the N5 input investigation do not satisfy N7).**
- Measurements: none.
- Notes: no N7 behavior, progress, stall, RSS, or blocking classification is inferred from the separate N6 attempt. The later nested Weston run is recorded separately in [N7 2026-11-18 evidence](N7-spark-aarch64-wayland-20261118.md); this file preserves the earlier not-run status and is not the current matrix record.
