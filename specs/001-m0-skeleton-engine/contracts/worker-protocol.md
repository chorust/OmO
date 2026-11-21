# Contract: omo-worker stdio protocol v1

**Transport**: worker stdin/stdout. Each frame is a `u32` little-endian byte length followed by
a UTF-8 JSON object. The max frame is 16 MiB; a larger frame is a protocol error, and the
worker is killed. **stdout carries only frames.** Logs go to stderr, which the supervisor
captures in a ring buffer and attaches to failure diagnostics.

## Envelope
```json
{"v":1,"worker_generation":7,"job_id":12,"request_id":40,"kind":"<kind>","body":{}}
```
The supervisor drops any frame whose `worker_generation` ≠ the current one, and deletes its
owned `result_ref.path` and optional `plot_ref.path` when discarding a stale result.

## Requests (supervisor → worker)
| kind | body | notes |
|---|---|---|
| `hello` | `{bootstrap_config}` | first frame; worker runs `open_bootstrapped` |
| `query` | `{sql, limits:{rows, bytes}, purpose:"preview"\|"job"}` | function probe before each task |
| `series_plan` | `{plan:"A"\|"B", stations_ref, variable, window_utc, catalog_entry}` | spike |
| `export` | `{result_id \| sql, target, format:"csv"\|"parquet", job_snapshot}` | see export-protocol |
| `interrupt` | `{target_request_id}` | cancel level 1 |
| `ensure_spatial` | `{}` | R4 measurement |
| `shutdown` | `{}` | graceful exit |

## Events (worker → supervisor)
| kind | body |
|---|---|
| `ready` | `{bootstrap_report}` |
| `bootstrap_failed` | `{error: BootstrapError, report}` → worker exits non-zero |
| `heartbeat` | `{t_ms, busy_request_id?}`, every 1 s |
| `progress` | `{request_id, stage: JobState, chunks_done?, chunks_total?}` |
| `result` | `{request_id, result_ref: ResultRef, plot_ref?: ResultRef|null, plot_source_rows?: u64, scan_metrics?}` | Preview results keep the full table file in `result_ref`; the worker writes an evenly sampled Arrow plot artifact in `plot_ref` (≤20 numeric columns × ≤10,000 rows). `plot_source_rows` records the rows sampled. `plot_ref` is null when the result has no numeric columns.
| `failed` | `{request_id, kind: FailKind, message}` |
| `export_committed` | `{request_id, target, sidecar, data_sha256}` |

## Cancellation (supervisor-side state machine)
1. Send `interrupt` and wait for `failed{Cancelled(Interrupt)}` or `result`.
2. If neither arrives, wait up to **10 s** (drain).
3. Then SIGKILL the worker, mark `Failed(Cancelled(Kill))`, spawn generation+1, send `hello`,
   and requeue interactive previews.

App exit skips straight to step 3.

## Result files
`<session_tmp>/g<generation>/<result_id>.arrow` (Arrow IPC **file** format), written batch by
batch. Interactive limits are enforced, and `truncated=true` is set when they are hit. A preview
may additionally publish `<session_tmp>/g<generation>/<result_id>-plot.arrow`; it contains the
worker-side bounded sample and does not replace the full table result.
