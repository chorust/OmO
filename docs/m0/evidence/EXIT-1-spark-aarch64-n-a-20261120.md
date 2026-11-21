# EXIT-1 current-route bootstrap on spark-aarch64/n-a — 2026-11-20

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Versions: Rust `1.100.0 (0ef1bc386 2026-11-09)`; DuckDB `v2.0.0`; Duckomo `v0.4.0`; platform `linux_arm64`.
- Current probe: `target/debug/omo-probe`, SHA-256 `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d`.
- The exact current source passed Rust 1.100.0 locked workspace tests (52/52), debug/release builds, and owner-only cache checks; see [N1 current-source evidence](N1-spark-aarch64-current-20261120.md).

## Community-signed

- Result: **pass**, 100/100 fresh connections with the pinned local-repository route and standard network-enabled command.
- Every attempt reports Duckomo `v0.4.0`, decompressed artifact SHA-256 `03afe7177e1ebd4bf288e056f2a3013627db1084c2968911bc8c5d135fab6542`, and a 64-row golden SHA-256 `bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f`.
- Every attempt keeps `allow_unsigned_extensions=false`, `allow_community_extensions=false`, and custom repository access forbidden after loading Duckomo. HTTPFS installed on the first attempt and loaded from cache on the remaining 99.
- Current reports: [100-run JSON](EXIT-1-spark-aarch64-community-hardened-final-20261120.json), [run log](EXIT-1-spark-aarch64-community-hardened-final-20261120-run.log).
- A separate fresh-cache `--no-network` run inside `bwrap --unshare-net` also passed and rejected a remote task; see [network-isolation evidence](COMMUNITY-LOCAL-NETWORK-ISOLATION-spark-aarch64-20261120.md).

## GitHub-release-unsigned

- Result: **pass**, 100/100 fresh connections with the standard network-enabled command.
- Every attempt reports Duckomo `v0.4.0`, decompressed artifact SHA-256 `2189a68bebc26f2eb7fa7b7b0f1fb2e5db9bd4cea543d5941ec165f05046a83e`, and the same 64-row golden SHA-256. The unsigned loading window closes with `allow_unsigned_extensions=false`; built-in community loading remains disabled and custom repositories are forbidden. HTTPFS installed once and loaded from cache on the remaining 99 attempts.
- Current reports: [100-run JSON](EXIT-1-spark-aarch64-unsigned-hardened-final-20261120.json), [run log](EXIT-1-spark-aarch64-unsigned-hardened-final-20261120-run.log).
- A separate fresh-cache `--no-network` run in a network namespace also passed the local golden query; HTTPFS was intentionally unavailable in that mode: [JSON](EXIT-1-spark-aarch64-unsigned-hardening-smoke-20261120.json), [log](EXIT-1-spark-aarch64-unsigned-hardening-smoke-20261120.log).

## Scope

The two channels use distinct compressed and installed artifact hashes; no fallback occurred. Both match the locked golden result. Under research D5's per-platform rule, `community-signed` is selected for Spark Linux arm64. Linux x86_64 and macOS arm64 remain `not-run`, and global channel selection remains open. See the current [EXIT-5 comparison](EXIT-5-spark-aarch64-current-20261120.md), [channel decision](../decisions/channel.md), and [host availability check](PLATFORM-AVAILABILITY-20261120.md).
