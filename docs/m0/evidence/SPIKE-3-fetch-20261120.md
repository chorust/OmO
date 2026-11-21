# SPIKE-3 artifact fetch and signature verification — 2026-11-20

On Spark Linux arm64, ran the current fetch script for both locked channels:

```bash
scripts/fetch-duckomo.sh community-signed linux_arm64
scripts/fetch-duckomo.sh github-release-unsigned linux_arm64
```

The script downloaded into fresh temporary directories, decompressed and measured each artifact, and printed lock-entry JSON without modifying the lock. The emitted compressed/decompressed hashes and sizes match the `linux_arm64` entries in `artifacts/duckomo.lock.json`:

- `community-signed`: compressed SHA-256 `a5429c62ec03142f4190d16959dd15584387da240bdf7dbe34e1b3d99007c9ac`; decompressed SHA-256 `03afe7177e1ebd4bf288e056f2a3013627db1084c2968911bc8c5d135fab6542`. OpenSSL signature verification succeeded using pinned key SHA-256 `b9aac70a9b1de896079041d76691284aceec44c4eef75a50c90ab2e99f9d54d1`.
- `github-release-unsigned`: compressed SHA-256 `8fc579a83eccd25729c7f345e70295e3b466d81a2aaeb34d3d2a59d277cb6c2e`; decompressed SHA-256 `2189a68bebc26f2eb7fa7b7b0f1fb2e5db9bd4cea543d5941ec165f05046a83e`. The downloaded bytes matched the asset's `SHA256SUMS` entry.

Both generated entries reused the existing locked golden hash; the fetch step did not change `artifacts/duckomo.lock.json`. Runtime compatibility is recorded separately in the current [EXIT-1](EXIT-1-spark-aarch64-n-a-20261120.md) and [EXIT-5 comparison](EXIT-5-spark-aarch64-current-20261120.md).
