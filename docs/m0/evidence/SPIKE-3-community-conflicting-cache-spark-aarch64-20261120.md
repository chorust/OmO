# SPIKE-3 repository-scoped load with a conflicting cache — spark-aarch64 — 2026-11-20

A product-level probe with current `omo-probe` SHA-256 `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d` tested whether `LOAD duckomo FROM <pinned-repository>` can select an unrelated canonical cache entry. It ran with `--no-network` in a fresh `bwrap --unshare-net` namespace and created this conflicting file before bootstrap:

```text
<extension-cache>/v2.0.0/linux_arm64/duckomo.duckdb_extension
```

The file contained `unrelated pre-existing extension cache entry`, with SHA-256 `92ed5432e941ffbe0cae0096b00500de56d6edaf9008f39fc8ae8e0c915107e9`. Bootstrap passed, the signed repository-scoped file at `v2.0.0/linux_arm64/repositories/omo_duckomo_v0_4_0_v2_0_0_linux_arm64_a5429c62/duckomo.duckdb_extension` hashed to the locked `03afe7177e1ebd4bf288e056f2a3013627db1084c2968911bc8c5d135fab6542`, and the unrelated canonical file remained byte-for-byte unchanged. The golden hash matched across 64 rows; repository access was forbidden after load.

- Report: [current JSON](EXIT-1-spark-aarch64-community-conflict-hardening-final-20261120.json), [current probe and cache verification log](EXIT-1-spark-aarch64-community-conflict-hardening-final-20261120.log).
- The extension-cache root was mode `0700` after bootstrap; the canonical file remained byte-for-byte unchanged, and the repository-scoped installed binary matched the lock. Unprotected intermediate creation modes remain inaccessible through the owner-only root.
- Scope: this verifies repository scoping and an unrelated pre-existing cache file on DuckDB v2.0.0/Linux arm64. Cache directories are user-owned and the filesystem lock serializes cooperating bootstraps; arbitrary same-user replacement is outside this test's threat model.
