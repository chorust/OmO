# Duckomo distribution channel decision

| Platform | Community-signed | GitHub-release-unsigned | Current decision |
|---|---|---|---|
| spark Linux arm64 | Current probe `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d`: fresh network-isolated smoke and 100/100 standard repeats passed; golden SHA-256 `bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f`. See [SPIKE-3 current route](../evidence/SPIKE-3-community-local-repository-spark-aarch64-20261120.md). Direct-file and built-in community failures from 2026-11-16 are historical and do not describe this route. | The same current probe passed 100/100 standard network-enabled repeats; an additional network-isolated no-network smoke passed. Both channels match the golden result while retaining separate artifact hashes; see [EXIT-5](../evidence/EXIT-5-spark-aarch64-current-20261120.md). | Select `community-signed` for Spark Linux arm64 under D5. No silent fallback. |
| Linux x86_64 | CI run 45104328529: locked SHA-256 verified, bootstrap + golden 3/3 passed. See [CI evidence](../evidence/CI-CROSS-PLATFORM-20261123.md). | CI run 45104328529: locked SHA-256 verified, bootstrap + golden 3/3 passed, live worker passed. | `community-signed` preferred; `github-release-unsigned` allowed by explicit selection. |
| macOS arm64 | CI run 45104328529 (`macos-15`): locked SHA-256 verified, bootstrap + golden 3/3 passed. | CI run 45104328529 (`macos-15`): locked SHA-256 verified, bootstrap + golden 3/3 passed, live worker passed. | `community-signed` preferred; `github-release-unsigned` allowed by explicit selection. |

The community and release files have distinct content hashes and are selected explicitly. Signature, repository, or hash failure on the community route remains a failure; bootstrap never substitutes the unsigned artifact.

## Global channel policy (OQ-4) — user decision 2026-11-23

- `community-signed` is the preferred channel on every locked platform.
- `github-release-unsigned` stays **allowed**. It is not disabled, but it must be selected explicitly (user setting, CLI flag, or worker config), and the BootstrapReport and export sidecars always record which channel was used.
- No silent signed→unsigned fallback. A signed-route failure is reported as a failure, and the user can then choose the unsigned channel.

### Lock-bump ownership and procedure (user decision 2026-11-23)

- **Owner:** the user decides whether to upgrade. When a new duckomo or DuckDB release appears, an agent or maintainer may report it and prepare a proposal. The lock is not changed without the user's explicit confirmation.
- **Procedure after confirmation:**
  1. Refresh both channels' entries for all three locked platforms in `artifacts/duckomo.lock.json` (via `scripts/fetch-duckomo.sh`).
  2. Re-establish the golden hash.
  3. Require `scripts/ci-verify-duckomo.py` to pass in CI before any release.
