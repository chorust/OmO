# SPIKE-3 duckomo artifact fetch — 2026-11-16

- Host / OS / kernel / desktop: `lizhan-spark`, Ubuntu 24.04.4 LTS, aarch64, kernel `6.17.0-1029-nvidia`; X11.
- Versions: Rust 1.96.0 (fetch script only; no Cargo build), local DuckDB CLI v1.5.5 (not used to LOAD these v2.0.0 artifacts).
- Commands: `scripts/fetch-duckomo.sh <channel> <platform>` for all six channel/platform combinations.
- Result: pass for download and gzip decompression. The unsigned GitHub release downloads matched the release `SHA256SUMS`; community SHA-256 values below are computed from the downloaded gzip files because no checksum manifest was used by the script. No LOAD was attempted, and no compatibility is inferred.
- Output / artifacts: lock entries in `artifacts/duckomo.lock.json`; per-run JSON was emitted to temporary files under `/tmp/omo-fetch-*.json`; the script removed each downloaded temporary directory.
- Notes / deviations: `golden.result_sha256` remains a placeholder until a verified bootstrap run. The fixture path remains pending T024. These measurements do not prove signatures, DuckDB compatibility, or equivalence between channels.

| Channel | Platform | Compressed bytes | SHA-256 (compressed) | Decompressed bytes | SHA-256 (decompressed) |
|---|---:|---:|---|---:|---|
| community-signed | linux_amd64 | 9,647,941 | `196a5b1f5252c8436e74d9ddb76c1bd20e8abd7e7ca52376d21c539b33764a12` | 30,575,278 | `96e3fb9b177721a2aad1bdcaa7d1866ef119a729a8158ceaa05754a2dcde82b6` |
| community-signed | linux_arm64 | 8,846,401 | `a5429c62ec03142f4190d16959dd15584387da240bdf7dbe34e1b3d99007c9ac` | 29,347,014 | `03afe7177e1ebd4bf288e056f2a3013627db1084c2968911bc8c5d135fab6542` |
| community-signed | osx_arm64 | 6,851,931 | `f63bc121944ddfbd879719b36510f914ad88c64a4a4199b5b6d08274f8767db9` | 24,162,062 | `7596f1473067853f6637678ef8fcc690eeb886691015a302085a429cc420976c` |
| github-release-unsigned | linux_amd64 | 9,561,128 | `13c4a6bc3db26a6b846dc189174c617f810db280d5c4cac760bf80185a833ac9` | 30,575,278 | `fa641c9427cda421ccfa6d02d271bc0edcb8784e83b63065f43ccfaa96afb11c` |
| github-release-unsigned | linux_arm64 | 8,783,732 | `8fc579a83eccd25729c7f345e70295e3b466d81a2aaeb34d3d2a59d277cb6c2e` | 29,347,014 | `2189a68bebc26f2eb7fa7b7b0f1fb2e5db9bd4cea543d5941ec165f05046a83e` |
| github-release-unsigned | osx_arm64 | 6,815,070 | `2b5b275f31d9cccbd55fac9ffd384d96f033c36a3c25795fec89e6ffa286168c` | 24,162,062 | `42a0166a7aca65936e3595f2f73a8b94d9e6780016fd0b579b1459bc62391e69` |
