# Engine source decision — feature 001

**Decision:** Use only the user-approved, content-addressed Duckling snapshot retained in this repository as the source reference for feature 001. No Duckling git or path dependency is used. Do not take another working-tree snapshot without separate explicit approval.

## Provenance and verification

- Source HEAD: `8ec3bec749fef31caa92bdddc240155df725526f` (context only; the approved source bytes came from the working tree).
- Capture timestamp: `2026-11-16T08:49:00Z`.
- Retained snapshot: [`vendor-provenance/snapshots/duckling-8ec3bec-20261116/`](../../../vendor-provenance/snapshots/duckling-8ec3bec-20261116/); 25 selected source files, each with captured size, source working-tree state, source path, snapshot path, and SHA-256 in its manifest.
- Verification command: `python3 scripts/verify-duckling-snapshot.py`.
- Verification result on 2026-11-17: all 25 file sizes and SHA-256 values matched the manifest.
- Complete source-to-destination and adaptation notes: [`vendor-provenance/DUCKLING.md`](../../../vendor-provenance/DUCKLING.md).

## Adaptation outcome

The approved snapshot was used as a bounded reference for the M0 engine seams. The implementation is adapted to the Omobench contracts: explicit artifact paths and channels, pre-task probes, a bootstrapped-connection type boundary, batch-wise Arrow IPC results, worker-owned cancellation, and bounded result limits. Snapshot modules without an M0 destination remain reference-only; no dependency or runtime read points back to Duckling.

The mapping and import notes in `vendor-provenance/DUCKLING.md` identify the source modules considered and their concrete destinations or reference-only status. Duckling and duckomo were not modified.
