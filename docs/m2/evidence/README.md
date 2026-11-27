# M2 Evidence Conventions

Every evidence record under `docs/m2/evidence/` records:
- Host and target triple (e.g., `lizhan-spark`, `aarch64-unknown-linux-gnu`);
- Date (UTC);
- Locked software versions (`rustc 1.100.0`, `DuckDB v2.0.0`, `duckomo v0.4.0`, `NSH =0.5.1` retained per Constitution Principle II after gate `S0-NSH-070`);
- Exact command executed;
- Observed result and output artifact paths.

Naming: `S0-NSH070-…`, `S1-…`, `V1-V6-…` (GUI), `V7-…` (fault injection), `SC-002-003-…`, `CI-…`, plus PNG screenshots.
Nothing is claimed as passed without a record here (Constitution Principle VI).
