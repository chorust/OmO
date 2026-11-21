# Source snapshot identity — 2026-11-20

This manifest identifies the current implementation and its build/test inputs while the feature remains uncommitted.

- Manifest: [SOURCE-SNAPSHOT-20261120.sha256](SOURCE-SNAPSHOT-20261120.sha256)
- Manifest SHA-256: `12e56f6b06e80b29f82f19b1cd31c35244f1ac3d0bef1d4af68a7cd3148483dd`
- Entries: 125 files, 1,110,872 bytes.
- Scope: workspace manifests and lockfile, Rust crates and tests, CI/release workflows, build/boundary scripts, artifact/catalog locks, fixtures, and retained Duckling provenance sources. The manifest sorts repository-relative paths and stores each file's SHA-256 and byte length. It excludes documentation, specs, generated `target/` outputs, and agent configuration.

The current N1 debug/release build and workspace test evidence is recorded in [N1 current-source verification](N1-spark-aarch64-current-20261120.md), with exact logs linked there. No source, build, lock, test, fixture, or workflow file changed after those builds; subsequent work updated acceptance/evidence documentation, which is outside this source manifest. The tested binaries are identified by their SHA-256 values in the N1 record and the corresponding runtime evidence.

The feature remains uncommitted. This manifest records the worktree contents without creating a commit or changing repository history. Evidence files, including the N5 screenshots and D-Bus traces, retain their own hashes in their respective records.
