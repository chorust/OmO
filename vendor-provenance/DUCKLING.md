# Duckling source provenance

## Approved source route

- Approval: the user approved the content-addressed working-tree snapshot route for feature
  `001` on 2026-11-16. This exception applies only to the selected files below; another
  working-tree snapshot needs separate explicit approval.
- Source repository: `/home/blizhan/repo/github/duckling`
- Source HEAD (context, not a claim that the working-tree bytes belong to this commit):
  `8ec3bec749fef31caa92bdddc240155df725526f`
- Captured: 2026-11-16T08:49:00Z
- Retained snapshot: [`snapshots/duckling-8ec3bec-20261116/`](snapshots/duckling-8ec3bec-20261116/)
- Machine-readable manifest: [`manifest.json`](snapshots/duckling-8ec3bec-20261116/manifest.json)
- Capture method: exact selected working-tree bytes were copied into this repository, read back,
  and SHA-256 checked; source bytes and working-tree status were checked again before the
  snapshot directory was finalized. Imports must be derived from the retained snapshot, not a
  later read of Duckling's working tree.

## Source-to-destination map

| Duckling source | Working-tree state | Omobench destination or use | Adaptation status |
|---|---|---|---|
| `connector/src/dialect/duckdb/bootstrap.rs` | untracked | `crates/omo-engine/src/{config,bootstrap,probe}.rs` | Adapted to explicit artifact verification, ordered bootstrap, probes, and configuration safeguards. |
| `connector/src/dialect/duckdb/session.rs` | untracked | `crates/omo-engine/src/conn.rs` | Adapted to process-owned connection lifecycle; a failed pre-task probe retires the connection. |
| `connector/src/dialect/duckdb/duckdb_sync.rs` | modified | `crates/omo-engine/src/{conn,query,results}.rs` | Adapted to bootstrapped connection ownership and batch-wise Arrow IPC output with row/byte limits. |
| `connector/src/dialect/duckdb/mod.rs` | modified | Engine module structure reference | Reference only; Omobench module boundaries follow the feature contracts. |
| `connector/src/dialect/duckdb/xlsx.rs` | untracked | No destination | Not imported; spreadsheet handling is outside M0. |
| `connector/src/dialect/mod.rs` | modified | DuckDB module boundary reference | Reference only. |
| `connector/src/cancel.rs` | clean | Worker cancellation reference | Reference only; cancellation ownership is implemented in the worker/supervisor. |
| `connector/src/error.rs` | clean | `crates/omo-engine/src/bootstrap.rs` error handling | Relevant failures were mapped to Omobench errors; the source taxonomy was not copied wholesale. |
| `connector/src/utils.rs` | clean | Arrow query/result handling reference | Reviewed; the bounded Arrow IPC writer is implemented in `crates/omo-engine/src/results.rs`. |
| `connector/src/studio/results.rs` | untracked | `crates/omo-engine/src/results.rs` | Reviewed; Omobench implements its `ResultRef` lifecycle and bounded writer without wholesale copying. |
| `connector/src/studio/result_query.rs` | untracked | `crates/omo-engine/src/query.rs` | Reviewed as reference for query ownership under the bootstrapped-connection contract. |
| `connector/src/studio/result_relation.rs` | untracked | No destination | Reference only; not imported into M0. |
| `connector/src/studio/reader.rs` | untracked | No destination | Reference only; M0 query streaming uses the locked DuckDB Arrow API. |
| `connector/src/studio/stations.rs` | untracked | No destination | Reference only; Plan A/B were implemented against the locked Duckomo API and independent fixture oracle. |
| `connector/src/studio/export.rs` | untracked | `crates/omo-engine/src/export.rs` | Adapted to same-filesystem staging, hash sidecar, fsync, and the three-step commit contract. |
| `connector/src/studio/catalog.rs` | untracked | No destination | Not imported; M0 does not implement catalog loading. |
| `connector/src/studio/catalog.json` | untracked | No destination | Not imported; `catalog/products.json` remains the M0 schema-versioned placeholder. |
| `connector/src/studio/model.rs` | untracked | Protocol and engine type reference | Reference only; M0 protocol and result types are defined in Omobench crates. |
| `connector/src/studio/profile.rs` | untracked | No destination | Reference only; no M0 settings were imported. |
| `connector/src/studio/error.rs` | untracked | Engine error taxonomy reference | Reference only; no Studio error variants were imported. |
| `connector/src/studio/mod.rs` | untracked | Module structure reference | Reference only. |
| `connector/Cargo.toml` | modified | Dependency/version evidence only | No dependency on Duckling's connector is permitted. |
| `connector/src/lib.rs` | modified | Module structure reference | No dependency on Duckling's connector is permitted. |
| `scripts/studio/duckomo-artifacts.json` | untracked | `artifacts/duckomo.lock.json` | The Omobench lock was generated independently from fetched artifacts; no Duckling path or runtime dependency is used. |
| `LICENSE` | clean | `vendor-provenance/snapshots/duckling-8ec3bec-20261116/LICENSE` | Retained for provenance and license review. |

This mapping is final for feature 001. It distinguishes adapted engine seams from reference-only files and files not imported. No direct file-copy or line-for-line reuse is claimed. Omobench contains no Duckling git or path dependency. Snapshot integrity is checked with `python3 scripts/verify-duckling-snapshot.py`.

## Per-file source hashes

These SHA-256 values bind the retained snapshot bytes. Working-tree status is recorded to show
which selected files were not present in the source commit. Full sizes and capture metadata are
in the machine-readable manifest.

| Source path | Working-tree state | SHA-256 |
|---|---|---|
| `LICENSE` | clean | `a57ebfdf4a7893bc2ba09a6f572c55a9f4052bebe8e6f33940df766ef61a1bcd` |
| `connector/Cargo.toml` | modified | `b06b3b851b917d84cf54a939a7e786af4aff02ac8943302fd529a2d24fc50914` |
| `connector/src/lib.rs` | modified | `6246ffc9341fbb551e296e08ac28f11189d4e989ad7e99b14c882b79a20f6352` |
| `connector/src/cancel.rs` | clean | `b3cb8e5988a4c39e5d28e34031cd58099535e66821bcbf61855dfc5abb8b3dcd` |
| `connector/src/error.rs` | clean | `4f4cd3ee71370aa4e192601095d10598e4e393e1f91c6d166e70b42c16e305f4` |
| `connector/src/utils.rs` | clean | `88cbdd6a7ad0c0eb385cba317d8ec7370e0a7595941d80dd2e6d682b5eebaaf4` |
| `connector/src/dialect/mod.rs` | modified | `d1c8b78a666bcf16e9702717bf140b11e1351d76ce21de2dd0deacda74f1e7e6` |
| `connector/src/dialect/duckdb/bootstrap.rs` | untracked | `090de27c26ca88ee2b1b4ec85c10e131cc723f08d77b4edbbb8a87b5785bf464` |
| `connector/src/dialect/duckdb/duckdb_sync.rs` | modified | `a3513ba0c2fe2eb83f0ae2ce488896287c9276aaaee47db598b880945d2637eb` |
| `connector/src/dialect/duckdb/mod.rs` | modified | `a72d306b8d408c94a9da477e23e535a488a1d88208664379c30ec40400354441` |
| `connector/src/dialect/duckdb/session.rs` | untracked | `68294ffda2834a1ba7cc88c823dea6629421b8452b465e7a16040a663c3e4902` |
| `connector/src/dialect/duckdb/xlsx.rs` | untracked | `9f6d1628e610be747dfc48b44f2e7ac17d7c37aa353e0024f84705be26b722ea` |
| `connector/src/studio/catalog.json` | untracked | `a7aaece33b2302dd730a66a9d8ef33fa4d51db674c2c08373ddc3b9bf22e2ec5` |
| `connector/src/studio/catalog.rs` | untracked | `48e77848336e912a8f7ae102b6a8949ec9563ea5e70dd17138bdaef8ea2a5352` |
| `connector/src/studio/error.rs` | untracked | `eb698c17dfad2306357ec94eb8a6e027c07f455a8b748720dbc7d07f230cd4ad` |
| `connector/src/studio/export.rs` | untracked | `18d2eaae2d880ce87faf80405e321a38eae761600d75f0ed5094f1eb7b318e1a` |
| `connector/src/studio/mod.rs` | untracked | `6bc9ee1a15758e57653ddf1a38653091d82a0a12c93170755a082d117ee4072e` |
| `connector/src/studio/model.rs` | untracked | `ed3a445628ab0fde4f07699fb357533b9be479cb1911830256eaeac890d06064` |
| `connector/src/studio/profile.rs` | untracked | `8d722719abb88ede59757f9140350b3387b6118f79908bbcef5ec435bc8a5656` |
| `connector/src/studio/reader.rs` | untracked | `f0dd8ed698b5de2d7e3bb4908e079ef7b4fbd1b65037a82af40438a4ed8f8b8d` |
| `connector/src/studio/result_query.rs` | untracked | `177b84a06e6bd74f8abfb87e2360ae367fa43df2f5ac3a28e2c9f36f3b33e501` |
| `connector/src/studio/result_relation.rs` | untracked | `0c2c8977af68f90b505f7e432d27a47b3a27a48adf3787e33f880a0bbf72dc92` |
| `connector/src/studio/results.rs` | untracked | `d18d247f2014143e4a04ebe14d62a9a792ae324ace9412c56d901a96ff4de99c` |
| `connector/src/studio/stations.rs` | untracked | `5fc035f02e60259529a4a6cd1f54980de0b686c69584c07f8be998eb80689b04` |
| `scripts/studio/duckomo-artifacts.json` | untracked | `d578a8e600d79c5f9e27c5cd355beac5484c3b1dd860c27134cc605c5666842d` |

## Import history

| Date | Duckling source | Omobench destination | Change |
|---|---|---|---|
| 2026-11-16–17 | Content-addressed snapshot above | `crates/omo-engine/src/{config,bootstrap,probe,conn,query,results,export}.rs` and `crates/omo-worker/src/` | Adapted M0 bootstrap, connection, query, cancellation, result, and export seams to Omobench contracts; other captured sources are marked reference-only or not imported. Snapshot hashes verified by `scripts/verify-duckling-snapshot.py`. |
