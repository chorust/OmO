# EXIT-1 bootstrap on spark aarch64 — 2026-11-16

- Host: Ubuntu 24.04.4, Linux aarch64; DuckDB v2.0.0, duckomo v0.4.0, Rust 1.100.0.
- Command: `target/debug/omo-probe --channel github-release-unsigned --artifact /tmp/omobench-duckomo-artifacts/duckomo-v0.4.0-duckdb-v2.0.0-linux_arm64.duckdb_extension.gz --repeat 100 --verbose`.
- Result: 100 of 100 fresh unsigned-channel connections passed version, platform, extension, required functions and locked golden query checks; HTTPFS loaded in all 100. The measured per-attempt median was 3887.5 ms and maximum 3903 ms, including verification/decompression and teardown.
- Output: [full JSON](EXIT-1-spark-aarch64-n-a-20261116.json). Golden hash matches the independently decoded fixture reference in [T024](T024-golden-fixture-20261116.md).
- Scope: this establishes the unsigned channel only on this host. The community-signed channel failed the separately recorded local signature checks in [SPIKE-3](SPIKE-3-signed-local-spark-aarch64-20261116.md), so both-channel equivalence and the full platform matrix remain unproven.
