# EXIT-5 fresh-connection repeat on spark-aarch64/n-a — 2026-11-17

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Versions: Rust `1.100.0`; DuckDB `v2.0.0`; duckomo `v0.4.0`; selected channel `github-release-unsigned`.
- Commands: selected-channel repeat command and measurements are in [EXIT-1](EXIT-1-spark-aarch64-n-a-20261116.md); community channel failure is in [SPIKE-3 signed-local evidence](SPIKE-3-signed-local-spark-aarch64-20261116.md).
- Result: **pass for the selected channel, with the alternate channel failing before a repeat run.**
- Measurements: GitHub-release-unsigned completed 100/100 fresh connections; community-signed failed both tested local load paths, so no 100-connection result or cross-channel hash comparison exists for it.
- Output / artifacts: [unsigned repeat report](EXIT-1-spark-aarch64-n-a-20261116.json), [community failure](SPIKE-3-signed-local-spark-aarch64-20261116.md).
- Notes: Linux x86_64 and macOS arm64 are not-run; no equivalence is inferred from compressed/decompressed artifact sizes or hashes.
