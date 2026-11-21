# EXIT-5 current channel comparison on spark-aarch64/n-a — 2026-11-20

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Versions: Rust `1.100.0`; DuckDB `v2.0.0`; Duckomo `v0.4.0`; platform `linux_arm64`.
- Both repeats used current probe SHA-256 `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d`.

| Channel | Runs | Passed | HTTPFS state | Locked decompressed artifact SHA-256 | Golden rows / SHA-256 |
|---|---:|---:|---|---|---|
| `community-signed` | 100 | 100 | installed 1; loaded 99 | `03afe7177e1ebd4bf288e056f2a3013627db1084c2968911bc8c5d135fab6542` | 64 / `bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f` |
| `github-release-unsigned` | 100 | 100 | installed 1; loaded 99 | `2189a68bebc26f2eb7fa7b7b0f1fb2e5db9bd4cea543d5941ec165f05046a83e` | 64 / `bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f` |

Both channels pass the Spark comparison rule: all required probes and golden queries pass, and golden result hashes match. Artifact bytes remain distinct. Every run reports unsigned and built-in community loading disabled and custom repository access forbidden after the channel-specific load/close sequence. No fallback occurred.

Reports and logs:

- Community-signed: [EXIT-1 JSON](EXIT-1-spark-aarch64-community-hardened-final-20261120.json), [run log](EXIT-1-spark-aarch64-community-hardened-final-20261120-run.log).
- GitHub-release-unsigned: [EXIT-1 JSON](EXIT-1-spark-aarch64-unsigned-hardened-final-20261120.json), [run log](EXIT-1-spark-aarch64-unsigned-hardened-final-20261120-run.log).
- Wrong-key rejection/no-fallback: [SPIKE-3 evidence](SPIKE-3-community-wrong-key-spark-aarch64-20261120.md).

## Decision scope

Under research D5's per-platform rule, `community-signed` is selected for Spark Linux arm64. Linux x86_64 and macOS arm64 have no current host/channel evidence, so they remain undecided; the global channel preference remains open under OQ-4. See [host availability](PLATFORM-AVAILABILITY-20261120.md).
