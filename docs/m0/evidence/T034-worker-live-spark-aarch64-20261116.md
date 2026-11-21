# T034 live worker and export on spark-aarch64 — 2026-11-16

- Host / OS / kernel: spark-aarch64, Ubuntu 24.04.4, Linux 6.17.0-1029-nvidia; terminal session, no desktop.
- Versions: Rust 1.100.0; NSH core 0.5.1 in the lockfile; DuckDB v2.0.0; Duckomo v0.4.0, `github-release-unsigned`.
- Commands:
  - `RUSTUP_TOOLCHAIN=1.100.0 cargo build --locked -p omo-worker --bin omo-worker`
  - `python3 scripts/check-worker-live.py --artifact /tmp/omobench-duckomo-artifacts/duckomo-v0.4.0-duckdb-v2.0.0-linux_arm64.duckdb_extension.gz`
- Result: pass. Worker bootstrap matched the locked golden hash. A query returned 1,000 Arrow rows tagged generation 7. Export by result ID wrote a 1,000-row CSV; the protocol digest, sidecar digest, and independently computed file digest matched. The test then interrupted a long DuckDB query while the worker emitted a heartbeat and received `failed` with kind `cancelled`.
- Measurements: golden hash `bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f`; CSV SHA-256 `dfd4a0051c7302f58e9d894a9cab2223783c40deff33b1124a5b768c067c1469`; exported data had 1,000 rows with values 0 through 999.
- Output: [machine-readable run](T034-worker-live-spark-aarch64-20261116.json).
- Notes: artifact bytes were checked against the selected lock entry before the live run. The worker used a temporary data directory that was removed at process exit. This is Linux arm64 evidence for the unsigned channel only.
