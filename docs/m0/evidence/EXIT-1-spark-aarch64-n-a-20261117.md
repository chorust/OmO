# EXIT-1 selected-channel bootstrap on spark-aarch64/n-a — 2026-11-17

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Versions: Rust `1.100.0`; DuckDB `v2.0.0`; duckomo `v0.4.0`; platform `linux_arm64`.
- Commands: the locked `omo-probe --channel github-release-unsigned --repeat 100 --verbose` run in [EXIT-1](EXIT-1-spark-aarch64-n-a-20261116.md); the separate community local install/load attempts in [SPIKE-3 signed-local evidence](SPIKE-3-signed-local-spark-aarch64-20261116.md).
- Result: **pass for the selected GitHub-release-unsigned channel; community-signed fails local loading.** The selected channel passed all probes and the golden query in 100/100 fresh connections. The community artifact failed direct `LOAD` and the controlled local `INSTALL` before a connection became task-capable. No unsigned fallback occurred.
- Measurements: unsigned 100/100; golden SHA-256 `bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f`; community local load 0 successful routes in the two tested forms.
- Output / artifacts: [unsigned run](EXIT-1-spark-aarch64-n-a-20261116.md), [community failure](SPIKE-3-signed-local-spark-aarch64-20261116.md), [channel decision](../decisions/channel.md).
- Notes: this is Linux arm64 only. It does not establish cross-channel result equality because the community artifact did not load. Other platform hosts remain untested.
