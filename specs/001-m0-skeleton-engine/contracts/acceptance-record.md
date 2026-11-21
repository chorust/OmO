# Contract: M0 acceptance & evidence records

`docs/m0/acceptance.md` holds the single matrix table. Each cell links to an evidence file
`docs/m0/evidence/<item>-<platform>-<session>-<YYYYMMDD>.md`.

Columns: `item | platform | session | result | key measurements | evidence`.

- `item` ∈ N1..N9; SPIKE-1 (NSH acceptance), SPIKE-2 (nearest A/B), SPIKE-3 (channel),
  SPIKE-4 (R4), SPIKE-5 (dependency coexistence / no DB in app); EXIT-1..EXIT-6 (design §7.1
  exit conditions, in order).
- `platform` ∈ `macos14-arm64`, `linux-x86_64`, `spark-aarch64`.
- `session` ∈ `wayland`, `x11`, `n/a`.
- `result` ∈ `pass`, `fail`, `not-run (<reason>)`. A `pass` needs an evidence file; a blank
  cell is a defect.

Evidence file skeleton:
```markdown
# <item> on <platform>/<session> — <date>
- Host / OS / kernel / desktop:
- Versions: rustc, nsh-*, duckdb-rs, DuckDB, duckomo (channel), GTK/Pango
- Command(s):
- Result: pass | fail | not-run
- Measurements:
- Output / artifacts: <paths>
- Notes / deviations:
```

Decision records (`docs/m0/decisions/`): `shell.md` (NSH accepted / compatible-release
evaluated / §7.3 fallback triggered), `channel.md` (per platform), `series-plan.md` (A|B),
`r4.md`, `engine-source.md` (FR-005 outcome). Each one cites its evidence files.
