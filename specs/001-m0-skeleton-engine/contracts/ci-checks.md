# Contract: boundary checks (`scripts/check-boundaries.sh`, run in CI and locally)

For each target in the CI matrix:

| Check | Command (indicative) | Must be |
|---|---|---|
| Core crates UI-free | `for c in omo-engine omo-protocol omo-worker omo-project; do cargo tree -p $c -e normal,build --locked; done \| grep -E '^\W*(nsh-\|gpui\|tauri)'` | empty |
| App release has no DB | `cargo tree -p omo-app -e normal --locked` (default features; the DuckDB-linking `in-process-engine` feature is test-only and never default) `\| grep -E 'duckdb\|libduckdb-sys'` | empty |
| App has no GPUI/Tauri/QuickJS (L0) | `cargo tree -p omo-app -e normal --locked \| grep -E 'gpui\|nsh-platform-gpui\|tauri\|quickjs\|nsh-js'` | empty |
| NSH pins exact | every `nsh-*` requirement in `Cargo.toml` is `=0.5.1` | true |
| duckdb pin exact | `duckdb = "=1.20000.1"` in `omo-engine` | true |
| Locked build | `cargo build --workspace --locked` and `cargo test --workspace --locked` | pass |

**Self-test**: `scripts/check-boundaries.sh --self-test` creates a throwaway copy of the
manifest in a temp dir, injects each forbidden dependency in turn, and asserts that the
matching check fails (SC-008). It never edits the real manifests.
