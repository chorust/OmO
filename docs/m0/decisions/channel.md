# Duckomo distribution channel decision

| Platform | Community-signed | GitHub-release-unsigned | Current decision |
|---|---|---|---|
| spark Linux arm64 | Current probe `0348de869c4c3bd52b130f9ac29b94383f66ea87d9a1393bb55f1e189069936d`: fresh network-isolated smoke and 100/100 standard repeats passed; golden SHA-256 `bc7cf6db243073d9baee64cdda6900cae9547a33bde00f61b8da10a7ce30254f`. See [SPIKE-3 current route](../evidence/SPIKE-3-community-local-repository-spark-aarch64-20261120.md). Direct-file and built-in community failures from 2026-11-16 are historical and do not describe this route. | The same current probe passed 100/100 standard network-enabled repeats; an additional network-isolated no-network smoke passed. Both channels match the golden result while retaining separate artifact hashes; see [EXIT-5](../evidence/EXIT-5-spark-aarch64-current-20261120.md). | Select `community-signed` for Spark Linux arm64 under D5. No silent fallback. |
| Linux x86_64 | Not run: no host/session evidence. | Not run: no host/session evidence. | Undecided. |
| macOS arm64 | Not run: no host/session evidence. | Not run: no host/session evidence. | Undecided. |

The community and release files have distinct content hashes and are selected explicitly. Signature, repository, or hash failure on the community route remains a failure; bootstrap never substitutes the unsigned artifact. The Spark decision is per platform. Linux x86_64 and macOS arm64 are undecided, and the global channel preference remains OQ-4 for the user after evidence exists for every target platform.
