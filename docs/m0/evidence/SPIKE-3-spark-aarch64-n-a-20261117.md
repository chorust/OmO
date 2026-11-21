# SPIKE-3 on spark-aarch64/n/a — 2026-11-17

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel 6.17.0-1029-nvidia.
- Versions: Rust 1.100.0; DuckDB v2.0.0; duckomo v0.4.0.
- Commands: locked bootstrap probes for the GitHub-release-unsigned channel; separate signature-enforced local load attempts for community-signed. Detailed commands and outputs are in [signed-local result](SPIKE-3-signed-local-spark-aarch64-20261116.md) and [EXIT-1](EXIT-1-spark-aarch64-n-a-20261117.md).
- Result: **pass for the channel evaluation on spark.** The community-signed artifact failed both tested local loading forms without unsigned fallback. The separate GitHub-release-unsigned artifact passed bootstrap and the independent golden query; the decision remains per-host and provisional.
- Measurements: release-unsigned 100/100 fresh connections passed; cross-channel equality is not applicable because the signed artifact did not load.
- Output / artifacts: [channel decision](../decisions/channel.md), [signed-local evidence](SPIKE-3-signed-local-spark-aarch64-20261116.md), [unsigned repeat evidence](EXIT-1-spark-aarch64-n-a-20261117.md).
