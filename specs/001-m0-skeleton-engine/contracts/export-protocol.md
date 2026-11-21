# Contract: atomic export (single commit point = sidecar rename)

Target `T = <dir>/<name>.<ext>`, sidecar `S = <dir>/<name>.<ext>.omo.json`.
Staging directory: `<dir>/.<name>.omo-tmp-<id>/`, which must be on the same filesystem as `T`.

1. **Stage**: write `data` and `sidecar` into the staging directory and fsync both.
   Precheck: `rename()` returning `EXDEV` → abort with `Export(CrossFilesystem)`. Never
   copy+delete.
2. **Step 1**: if `T` or `S` exists, the user has already confirmed the overwrite. Rename `S`
   to `<dir>/.<name>.omo-old` so the old result is invalidated first.
3. **Step 2**: rename the staged data to `T`.
4. **Step 3**: rename the staged sidecar to `S`, then fsync `<dir>`. → **saved**.
5. **Cleanup**: remove the staging directory and `.omo-old`.

**Saved predicate** (shared by UI and consumers): `S` exists, its `data.sha256` equals the
SHA-256 of `T`, and `S.schema` is known.

**Startup scan** of recent export dirs (from app state):

| Found | Saved predicate | Verdict / action |
|---|---|---|
| `.omo-tmp-*` and/or `.omo-old` | true | finished: delete leftovers |
| `.omo-tmp-*` and/or `.omo-old` | false | mark `T` "incomplete export — re-run"; keep files for inspection; never show as saved |
| no leftovers | — | nothing |

**Kill-injection points for acceptance**: after step 1, after step 2, after step 3 (before
cleanup). Expected verdicts: incomplete, incomplete, finished.

Optional directory mode (`<name>.omo/` as a single rename) is reserved for M2+/reports and is
not built in M0.
