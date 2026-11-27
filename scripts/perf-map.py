#!/usr/bin/env python3
"""Gate S1-M2 / SC-001: cold `map_load_field` wall time over the station extent.

Each trial starts a fresh worker process with a fresh temporary data directory, so no field,
connection, or DuckDB cache survives between trials. Reports the `map_load_field` request time
(the SC-001 figure) and, separately, worker bootstrap time.
"""
from __future__ import annotations

import argparse
import json
import statistics
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import omo_worker_client as owc  # noqa: E402
from omo_worker_client import station_bbox  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--worker", type=Path, default=Path("target/release/omo-worker"))
    parser.add_argument("--stations", type=Path, default=Path("fixtures/stations/m2-regional-100.csv"))
    parser.add_argument("--pad-deg", type=float, default=1.0)
    parser.add_argument("--run", default="latest_complete", help="'latest_complete' or an RFC 3339 run time")
    parser.add_argument("--valid-time", default=None, help="RFC 3339 valid time (default: first of the run)")
    parser.add_argument("--force-data-run", action="store_true")
    parser.add_argument("--target-s", type=float, default=30.0)
    args = parser.parse_args()

    root = Path.cwd()
    entry = json.loads((root / "catalog/products.json").read_text())["entries"][0]
    bbox = station_bbox(args.stations, args.pad_deg)
    timings, boots, last = [], [], {}
    with tempfile.TemporaryDirectory(prefix="omo-perf-map-") as tmp:
        tmp_path = Path(tmp)
        artifact = owc.stage_artifact(root, tmp_path)
        for trial in range(args.runs):
            work = tmp_path / f"run-{trial}"
            work.mkdir()
            boot_started = time.monotonic()
            client = owc.WorkerClient(args.worker.resolve(), owc.bootstrap_config(root, artifact, work))
            boots.append(round(time.monotonic() - boot_started, 2))
            try:
                body = {"field_id": f"perf-{trial}", "catalog_entry": entry, "catalog_version": "perf",
                        "run": args.run, "bbox": bbox, "force_data_run": args.force_data_run}
                if args.valid_time:
                    body["valid_time_utc"] = args.valid_time
                started = time.monotonic()
                last = client.request("map_load_field", body, timeout=900)
                timings.append(round(time.monotonic() - started, 2))
            finally:
                client.close()
            print(json.dumps({"trial": trial, "seconds": timings[-1], "bootstrap_s": boots[-1],
                              "layout": last["source_layout"], "valid_count": last["stats"]["valid_count"]}),
                  flush=True)
    summary = {
        "runs": timings, "median_s": statistics.median(timings), "bootstrap_s": boots,
        "target_s": args.target_s, "pass": statistics.median(timings) < args.target_s,
        "reference_time_utc": last["run"]["reference_time_utc"], "valid_time_utc": last["valid_time_utc"],
        "source_layout": last["source_layout"], "source_object": last["source_object"],
        "object_version": last.get("object_version"), "stats": last["stats"],
        "coverage_bbox": last["coverage_bbox"], "scan_metrics": last["scan_metrics"],
    }
    print(json.dumps(summary))
    return 0 if summary["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
