# N1 current source build and test verification — spark-aarch64/n/a — 2026-11-20

- Host: lizhan-spark, Ubuntu 24.04, Linux aarch64, kernel `6.17.0-1029-nvidia`.
- Toolchain: Rust `1.100.0 (0ef1bc386 2026-11-09)`.
- Prerequisites: host GTK4/Pango/GStreamer development packages from [T019 installation record](T019-spark-aarch64-install-20261119T013400Z.log); build used system pkg-config and no `/tmp/gtkdev` override.

## Validation

- `cargo +1.100.0 test --workspace --locked --offline`: **52 passed, 0 failed**. This includes the repository signature metadata, cache containment, atomic staging, symlink replacement, owner-only permission and no-fallback tests. Full output: [workspace test log](N1-spark-aarch64-current-workspace-test-final-20261120.log), SHA-256 `ba0d294e436ffbcc06a0dda6224c527a925fa1aaf906ed249ddca249937acffc`.
- `cargo +1.100.0 build --workspace --locked --offline`: passed; [debug build log](N1-spark-aarch64-current-workspace-build-final-20261120.log).
- `cargo +1.100.0 build --workspace --release --locked --offline`: passed; [release build log](N1-spark-aarch64-current-workspace-release-final-20261120.log).
- `cargo +1.100.0 fmt --all -- --check` and `git diff --check`: passed.
- `scripts/check-boundaries.sh` and `scripts/check-boundaries.sh --self-test`: passed; all 10 injected forbidden dependency cases were detected.
- Artifact lock JSON and the six channel/platform pairs were validated. The fetch script downloaded the locked community and unsigned arm64 assets; the community signature verified against the lock-pinned key.

## Current source and binary identifiers

- Source/build/test manifest: [source snapshot record](SOURCE-SNAPSHOT-20261120.md), 125 files / 1,110,872 bytes; manifest SHA-256 `12e56f6b06e80b29f82f19b1cd31c35244f1ac3d0bef1d4af68a7cd3148483dd`.
- Debug `omo-probe`: SHA-256 `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d`.
- Release `omo-app`: SHA-256 `7bf424781804ec8894a488f6d41227f5b0af12ae56d2e9231bab830cb71eb0d7`.
- Release `omo-worker`: SHA-256 `bcd1e8deaf89f82e721695fdb105d93939e5345b805298798bfce2f1d1ea0b24`.
- Release `omo-probe`: SHA-256 `6ceddb27e19fde35e7da5ebf81efb65ffbca0f072dd0010773a3d0d423947d4b`.

## Runtime smoke

With the current debug probe, a fresh signed cache passed in a bubblewrap network namespace with `--no-network`. The 64-row golden hash matched, remote work was rejected as capability-unavailable, the cache root and the checked metadata/version/platform/installed-repository directories were mode `0700`, and artifact and lock files were mode `0600`. Intermediate staging container directories remain inaccessible through the owner-only cache root. See [network-isolation evidence](COMMUNITY-LOCAL-NETWORK-ISOLATION-spark-aarch64-20261120.md) and the [current conflicting-cache check](SPIKE-3-community-conflicting-cache-spark-aarch64-20261120.md).

Result: **pass for current Spark build/test/boundary scope**. This does not provide host acceptance evidence for Linux x86_64 or macOS arm64.
