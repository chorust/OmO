# Contract: `artifacts/duckomo.lock.json`

```json
{
  "schema": "omo-artifact-lock/1",
  "entries": [
    {
      "channel": "community-signed",
      "extension": "duckomo",
      "version": "v0.4.0",
      "duckdb_version": "v2.0.0",
      "platform": "linux_arm64",
      "file": "duckomo.duckdb_extension.gz",
      "sha256": "<filled by fetch script>",
      "size": 0,
      "sha256_decompressed": "<filled by fetch script>",
      "size_decompressed": 0,
      "signature_key_sha256": "<SHA-256 of the DER public key>",
      "signature_public_key_pem": "<pinned DuckDB community public key PEM>",
      "source": {
        "url": "https://community-extensions.duckdb.org/v2.0.0/linux_arm64/duckomo.duckdb_extension.gz",
        "pr": "duckdb/community-extensions#3577",
        "ref": "1bc76c3a36d26607b463909153cff9c255e19919",
        "downloaded_at": "<RFC3339>"
      },
      "golden": {"fixture": "fixtures/om/<name>.om", "query_id": "golden-v1", "result_sha256": "<filled after first verified run>"}
    },
    {
      "channel": "github-release-unsigned",
      "extension": "duckomo",
      "version": "v0.4.0",
      "duckdb_version": "v2.0.0",
      "platform": "linux_arm64",
      "file": "duckomo-v0.4.0-duckdb-v2.0.0-linux_arm64.duckdb_extension.gz",
      "sha256": "<cross-checked against release SHA256SUMS>",
      "size": 0,
      "sha256_decompressed": "<filled by fetch script>",
      "size_decompressed": 0,
      "source": {"url": "https://github.com/chorust/duckomo/releases/download/v0.4.0/…", "release_tag": "v0.4.0", "downloaded_at": "<RFC3339>"},
      "golden": {"fixture": "fixtures/om/<name>.om", "query_id": "golden-v1", "result_sha256": "<same value expected>"}
    }
  ]
}
```

Rules:
- Platforms: `linux_amd64`, `linux_arm64`, `osx_arm64` × both channels → 6 entries.
  `osx_amd64` is excluded (OQ-5); Constitution 1.2.0 build-only `macos-15-intel` CI and
  release archive builds do not add `osx_amd64` lock entries or imply runtime acceptance.
- `sha256`/`size` refer to the downloaded file. The decompressed file's hash and size are
  also recorded because DuckDB installs and loads the decompressed extension.
- Every `community-signed` entry pins the signer fingerprint (`signature_key_sha256`) and
  PEM (`signature_public_key_pem`). The fingerprint is SHA-256 over the DER public key and
  must match the PEM. Bootstrap stages the already verified compressed asset in a private
  local repository configured with that key, verifies the installed bytes against the lock,
  then loads from the repository. Bootstrap holds an OS-level per-repository lock from staging
  through `LOAD`. On Unix, the configured extension-cache root and the directories that directly
  contain app-staged artifacts, locks, and repository-scoped Duckomo files are mode `0700`; the
  app-staged artifact and lock files are mode `0600`. DuckDB-created intermediate cache
  directories and metadata files may retain their umask mode, but are inaccessible through the
  owner-only root. This prevents other OS users from replacing the verified file during that
  interval. The configured cache is user-owned; protection against a malicious process running
  as the same user is outside the threat model. Built-in community loading and unsigned loading
  stay disabled; custom repository access is forbidden after Duckomo loads. Unsigned entries
  must not carry community signing material.
- `golden.result_sha256` is the deterministic IEEE-754 big-endian SHA-256 over the first 64
  sorted float32 values of `fixtures/om/chmi-aladin-cz-hsurf.om` (computed by
  `scripts/hash-golden-oracle.py` and verified on `linux_arm64`). Under Constitution 1.3.0
  (2026-11-21), all 6 entries carry this locked golden hash so released binaries can bootstrap
  on `linux_amd64` and `osx_arm64`, and `.github/workflows/ci.yml` executes
  `scripts/ci-verify-duckomo.py` on native `ubuntu-24.04`, `ubuntu-24.04-arm`, and `macos-14`
  runners to enforce that each platform matches the locked hash before passing CI. Any
  placeholder remains a bootstrap failure.
- Lookup key: (channel, version, duckdb_version, platform). A missing entry is a
  `BootstrapError::ArtifactMismatch`.
- `scripts/fetch-duckomo.sh <channel> <platform>` downloads, verifies (against
  `SHA256SUMS` for releases), and prints the entry with the currently pinned community signer
  when applicable. It never edits the lock silently; the diff is reviewed.
