# SPIKE-4 current configuration-lock behavior — spark-aarch64 — 2026-11-20

- Host: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Probe SHA-256: `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d`.
- Each channel ran `--r4 --repeat 1` against a fresh data directory and extension cache.

| Channel | Attempt | HTTPFS | Spatial preinstall | `LOAD spatial` after lock | Unsigned close | Repository policy after Duckomo |
|---|---|---|---|---|---|---|
| `community-signed` | pass | installed | `ok` | `ok` | `not_applicable`; unsigned disabled from open | forbidden after load |
| `github-release-unsigned` | pass | installed | `ok` | `ok` | `ok`; final `allow_unsigned_extensions=false` | forbidden before Duckomo load |

Both attempts matched the 64-row golden SHA-256 `bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f`. Reports and logs:

- Community-signed: [JSON](EXIT-1-spark-aarch64-community-r4-hardened-final-20261120.json), [log](EXIT-1-spark-aarch64-community-r4-hardened-final-20261120.log).
- GitHub-release-unsigned: [JSON](EXIT-1-spark-aarch64-unsigned-r4-hardened-final-20261120.json), [log](EXIT-1-spark-aarch64-unsigned-r4-hardened-final-20261120.log).

A separate signed run with `bwrap --unshare-net` and `--no-network` passed the local golden query and rejected remote work with `rejected_with_capability_unavailable_remote`; see [network-isolation evidence](COMMUNITY-LOCAL-NETWORK-ISOLATION-spark-aarch64-20261120.md).

The preinstall-before-lock mitigation works for both locked channels on this Spark Linux arm64 host. This does not establish the same behavior on Linux x86_64 or macOS arm64; their cells remain `not-run`.
