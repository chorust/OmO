# Quickstart: validating M1 Stations × Forecast Series

These are validation scenarios, not implementation steps. Contracts:
- [worker protocol](contracts/worker-protocol-m1.md)
- [catalog](contracts/catalog.md)
- [project file](contracts/project-file.md)
- [station CSV](contracts/station-csv-import.md)
- [export sidecar](contracts/export-sidecar-v2.md)

Evidence goes to `docs/m1/evidence/` (Constitution VI).

## Prerequisites

- Reference host `lizhan-spark` (Linux aarch64, X11 session), with the M0 prerequisites
  installed (`docs/m0/linux-prereqs.md`), Rust 1.100.0, and network access to
  `s3://openmeteo`.
- Locked duckomo artifacts, fetched as in M0 (`scripts/fetch-duckomo.sh`).
- Oracle tooling (dev only): `python3 -m venv .venv-oracle && .venv-oracle/bin/pip install -r scripts/requirements-oracle.txt`.

```bash
cargo +1.100.0 build --workspace --locked
cargo +1.100.0 test --workspace --locked
scripts/check-boundaries.sh && scripts/check-boundaries.sh --self-test
```

## S1 — Plan G feasibility gate (must pass before UI work)

Run on the locked stack through the worker (`omo-series-spike --plan G`, or the `series`
request in the harness). Record the results in `docs/m1/evidence/S1-spark-aarch64-<date>.md`:

1. Dimensions of the latest complete `data_run/ecmwf_ifs/.../temperature_2m.om`.
2. 1 station and 100 stations: each station returns exactly its selected point, and the scan
   metrics show selective reads (body bytes ≪ object size).
3. Row-table cross-check: ≥ 10,000 sampled flat indices match duckomo `lat`/`lon` exactly.
4. 100 stations × full window from a cold cache: wall time (median of 5).

Expected: all pass. Otherwise, stop and ask the user (spec Assumptions; research R-01).

## V1 — Default series end to end (US1, US2; SC-001, SC-004)

1. Start `omo-app` with an empty project. The catalog shows
   `ecmwf_ifs.temperature_2m` selected and **Ready**, with the latest complete run time.
2. Import `fixtures/stations/m1-100.csv` (100 stations, `id,name,lat,lon`). Check the mapping
   proposal and the preview, then confirm.
3. Run the series. Expected:
   - every station appears;
   - 113 valid times per station (00Z/12Z run), or 109 with the label
     "0–144 h of requested 0–168 h" (06Z/18Z run);
   - the grid lat/lon and distance are ≤ 7.2 km;
   - the result badge reads "received, not saved".
4. Time steps 2–3 and record the duration (SC-001, SC-002).

## V2 — Reference accuracy (SC-003)

```bash
.venv-oracle/bin/python scripts/o1280-nearest-oracle.py fixtures/stations/m1-oracle.csv > /tmp/nearest.csv
.venv-oracle/bin/python scripts/o1280-value-oracle.py --run <reference_time> /tmp/nearest.csv > /tmp/values.csv
```

Run the app series for the same run (pinned with an explicit `run`), export it to CSV, and
compare. Expected: 100% equality of flat index and coordinates, and of values at
0.05 °C resolution, including nulls.

## V3 — Exploration (US3)

- Select one station, then five: the chart shows labelled curves and the legend toggles.
- Set the window to 0–72 h: only those valid times remain, and the preview is re-keyed.

## V4 — Export (US4; SC-007)

- Export to CSV and to Parquet. Check that the sidecar `/2` fields match the result, the data
  hash verifies, and the UI shows "saved → path" only after commit.
- Run `omo-harness export-kill --at step1|step2|step3` on a series export. Expected:
  incomplete, incomplete, finished.

## V5 — Project round trip and drafts (US5; SC-006)

- Save, close, and reopen: the stations hash, mapping, selection, window, SQL, and view are
  identical.
- Edit, wait more than 30 s, and kill the app. On restart a recovery prompt appears, and the
  project file is byte-identical to before.
- Open the same project on CI (linux_amd64/osx_arm64 test) and check the round-trip hash.

## V6 — SQL (US6)

`SELECT station_id, date_trunc('day', valid_time) d, max(value) FROM station_series JOIN stations USING (station_id) GROUP BY ALL`
returns per-station daily maxima that match the raw series for a sample station.

## V7 — Fault injection (SC-005, ≥ 10 trials each)

| Fault | Expected |
|---|---|
| Cancel during reading | `Cancelled`; previous result kept; UI idle |
| `kill -9` worker during reading | `WorkerCrashed`; generation + 1; no stale result shown; retry works |
| Network down before request | catalog `NotReady(RemoteUnavailable)`; series `CapabilityUnavailable(remote)` |
| Network down during reading | failed request; last result kept with a stale marker |

## V8 — Chinese IME (SC-008)

On Spark X11 with IBus/libpinyin or Fcitx5: compose, pick a candidate, and commit Chinese text
in the SQL editor and the station filter. Record the result in
`docs/m1/evidence/IME-spark-aarch64-x11-<date>.md`.

## V9 — Cross-platform (SC-009)

CI (`ci.yml`) must be green on all four jobs. The new network smoke test
`scripts/ci-verify-series.py` (series for 3 stations against the latest complete run) runs
on `linux_amd64`, `linux_arm64`, and `osx_arm64`. Network failures there are reported as
`skipped(network)`, not as passes.
