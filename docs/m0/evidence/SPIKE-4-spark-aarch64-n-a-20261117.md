# SPIKE-4 on spark-aarch64/n/a — 2026-11-17

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel 6.17.0-1029-nvidia.
- Versions: Rust 1.100.0; DuckDB v2.0.0; duckomo v0.4.0; GitHub-release-unsigned linux_arm64 artifact.
- Commands: `omo-probe --r4` and `omo-probe --no-network`; JSON reports are linked below.
- Result: **pass for the measured R4 behavior on the unsigned spark connection.** The unsigned setting was closed before config lock; the signed spatial extension was provisioned before lock and loaded after lock. With network disabled, local golden query passed, HTTPFS was unavailable, and a remote task returned `CapabilityUnavailable(remote)`.
- Measurements: `unsigned_close: ok`, `spatial_preinstall: ok`, `spatial_after_lock: ok`; no-network remote capability rejection observed.
- Output / artifacts: [R4 JSON](SPIKE-4-r4-spark-aarch64-n-a-20261116.json), [no-network JSON](SPIKE-4-no-network-spark-aarch64-n-a-20261116.json), [decision](../decisions/r4.md).
