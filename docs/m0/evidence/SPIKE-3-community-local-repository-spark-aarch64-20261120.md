# SPIKE-3 pinned local community repository — spark-aarch64 — 2026-11-20

- Host / OS / kernel: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Versions: Rust `1.100.0 (0ef1bc386 2026-11-09)`; DuckDB `v2.0.0`; Duckomo `v0.4.0`; platform `linux_arm64`.
- Current probe: `target/debug/omo-probe`, SHA-256 `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d`.
- Lock: `community-signed` / `v0.4.0` / DuckDB `v2.0.0` / `linux_arm64`. Compressed SHA-256 `a5429c62ec03142f4190d16959dd15584387da240bdf7dbe34e1b3d99007c9ac`; decompressed SHA-256 `03afe7177e1ebd4bf288e056f2a3013627db1084c2968911bc8c5d135fab6542`.
- Signer: the lock pins PEM key fingerprint `b9aac70a9b1de896079041d76691284aceec44c4eef75a50c90ab2e99f9d54d1` (SHA-256 of the DER public key). The same key is the first key in fetched community repository metadata and independently verifies the locked community artifacts.

## Route

The current engine does not use direct `INSTALL '<file>'` or built-in `INSTALL duckomo FROM community`. Those older observations remain historical and are not evidence for this route. For the signed channel it:

1. verifies the compressed input against the lock and decodes it under the declared decompressed size;
2. stages the exact compressed bytes in an owner-only local repository below the configured extension cache;
3. registers that repository with only the pinned public key;
4. runs `FORCE INSTALL duckomo FROM <repository>` while unsigned and built-in community loading are disabled;
5. verifies the installed repository-scoped binary against the decompressed lock hash before `LOAD`;
6. sets `allow_extension_repositories='forbidden'` before configuration lock.

## Current network-isolated smoke

The current debug probe used a fresh data directory/cache, the locked `.gz` artifact, `--no-network`, and a bubblewrap namespace with network access removed:

```bash
root=/tmp/omo-probe-community-hardening-smoke-20261120
mkdir -p "$root/data" "$root/extensions"
bwrap --unshare-net \
  --ro-bind / / --dev-bind /dev /dev --proc /proc \
  --bind "$root" "$root" \
  --chdir /home/blizhan/repo/github/OmO \
  ./target/debug/omo-probe --channel community-signed --repeat 1 --no-network --verbose \
  --artifact /tmp/omo-probe-community-route-20261120/artifact/duckomo.duckdb_extension.gz \
  --lock artifacts/duckomo.lock.json \
  --data-dir "$root/data" --ext-cache-dir "$root/extensions"
```

- Result: **pass**; golden query returned 64 rows with SHA-256 `bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f`.
- Remote task result: `rejected_with_capability_unavailable_remote`.
- Final settings: `allow_unsigned_extensions=false`, `allow_community_extensions=false`, `allow_extension_repositories=forbidden`, and both autoinstall/autoload disabled.
- Cache root, metadata directories, staged repository tree, and installed repository tree were mode `0700`; artifact and lock files were mode `0600`.
- Report: [current JSON](EXIT-1-spark-aarch64-community-hardening-smoke-20261120.json), [probe log](EXIT-1-spark-aarch64-community-hardening-smoke-20261120.log).

`--no-network` by itself is an application mode that skips HTTPFS provisioning; this result also used an external network namespace. See [network-isolation evidence](COMMUNITY-LOCAL-NETWORK-ISOLATION-spark-aarch64-20261120.md).

## Current standard repeat result

The same current probe completed `--repeat 100` without `--no-network` on a fresh cache. All attempts passed. Every report records Duckomo `v0.4.0`, the locked decompressed artifact hash, the locked 64-row golden hash, unsigned/community loading disabled, and repository access forbidden after load. Attempt latency ranged from 6,104 ms to 11,937 ms. HTTPFS installed once and loaded on the other 99 connections.

- [Current 100-run JSON](EXIT-1-spark-aarch64-community-hardened-final-20261120.json)
- [Current 100-run log](EXIT-1-spark-aarch64-community-hardened-final-20261120-run.log)

## Scope

This validates the current signed route on Spark Linux arm64 only. It does not select a global channel; Linux x86_64 and macOS arm64 remain not-run. The older direct-file and built-in community observations are historical failures of those routes.

Additional current-route checks:

- [Wrong-key rejection](SPIKE-3-community-wrong-key-spark-aarch64-20261120.md): a different valid community key fails `FORCE INSTALL` under signature enforcement; no unsigned retry occurs.
- [Conflicting cache](SPIKE-3-community-conflicting-cache-spark-aarch64-20261120.md): an unrelated canonical Duckomo cache file remains untouched while the repository-scoped file matches the lock.
