# SPIKE-3 wrong signer rejected — spark-aarch64 — 2026-11-20

- Host: lizhan-spark, Ubuntu 24.04, Linux aarch64.
- Probe SHA-256: `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d`.
- Test: ran the current `omo-probe` community-signed route in a fresh extension cache and `bwrap --unshare-net` network namespace. The test lock preserves the real artifact's compressed and decompressed hashes, size, version, platform, and golden hash, but pins a different public key from the DuckDB community repository metadata. That alternate key's fingerprint is internally valid (`3cd98db7806f72c2e6c3382ad7896c2d925f4b54eb0083d5504efbe90a38c166`), so the lock parser accepted it and DuckDB had to enforce the artifact signature.
- Result: **fail closed as expected**. DuckDB rejected `FORCE INSTALL` because the extension signature did not verify under the pinned key. The report records exactly one attempt for `community-signed`, `allow_unsigned_extensions=false`, `allow_community_extensions=false`, and no installed repository-scoped Duckomo binary. No unsigned retry occurred.
- Test lock: [wrong-key fixture](EXIT-1-spark-aarch64-community-wrong-key-lock-20261120.json).
- Runtime report: [current JSON](EXIT-1-spark-aarch64-community-wrong-key-final-20261120.json), [current probe log](EXIT-1-spark-aarch64-community-wrong-key-final-20261120.log).

This exercises the application route and DuckDB signature enforcement separately from the matching hash checks. The network namespace also ensures this failure path cannot fetch a different artifact.
