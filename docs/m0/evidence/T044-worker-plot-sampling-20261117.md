# T044 worker-side plot sampling — 2026-11-17

## Result

The worker now creates a generation-scoped Arrow IPC plot artifact after writing a preview's full
result. The full result remains the source for table paging. The plot artifact contains at most 20
numeric columns and 10,000 evenly distributed rows. Query result events carry both references and
the source row count. The UI loads the plot artifact for worker results; app-side sampling remains
only for `OMOBENCH_FIXTURE_RESULT` acceptance fixtures.

## Verification

- Rust `1.100.0`: `cargo test --locked -p omo-engine --test results -p omo-worker --lib` passed. The
  plot test wrote 20,000 × 22 numeric values, then confirmed a 10,000 × 20 sample, evenly spaced
  values including both endpoints, and an unchanged full result.
- Rust `1.100.0`: `PKG_CONFIG_PATH=/tmp/gtkdev/pc cargo test --locked -p omo-app --test supervisor`
  passed (5 tests). The stale-generation case confirmed that both the full result and plot
  artifact are deleted. The GTK/GStreamer development metadata came from the documented
  `/tmp/gtkdev` extraction, not system package installation.
- Rust `1.100.0`: `cargo build --locked -p omo-worker` passed. Cargo reported unused dependency
  warnings for `anyhow` and `tokio` in `omo-worker/Cargo.toml`.
- Final Rust `1.100.0` workspace verification passed:
  `PKG_CONFIG_PATH=/tmp/gtkdev/pc cargo build --locked --workspace`,
  `PKG_CONFIG_PATH=/tmp/gtkdev/pc cargo test --locked --workspace`, and
  `cargo fmt --all -- --check`. The build/test used the documented `/tmp/gtkdev` metadata
  workaround, not system package installation.
- `scripts/check-boundaries.sh`, `scripts/check-boundaries.sh --self-test` (10 injected cases),
  and `python3 scripts/verify-duckling-snapshot.py` passed.
- Live worker protocol query using `/tmp/omo-ui-run/bootstrap.json` and
  `SELECT i::BIGINT AS idx, (i * 0.5)::DOUBLE AS value FROM range(20000) AS t(i)` emitted one
  `result` event with `result_ref.rows=20,000`, `plot_ref.rows=10,000`, and
  `plot_source_rows=20,000`. Both generation-1 files existed in the same results directory:
  full result 327,834 bytes; plot artifact 163,250 bytes.

The live query verified the worker protocol and output files. It did not measure chart render time or
replace the separate N3 interactive chart acceptance run.
