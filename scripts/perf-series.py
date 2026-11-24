#!/usr/bin/env python3
"""SC-002: wall time of a 100-station default-window `series` request (fresh worker per run)."""
from __future__ import annotations

import argparse
import csv
import json
import statistics
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import omo_worker_client as owc  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--worker", type=Path, default=Path("target/release/omo-worker"))
    parser.add_argument("--stations", type=Path, default=Path("fixtures/stations/m1-100.csv"))
    args = parser.parse_args()
    root = Path.cwd()
    entry = json.loads((root / "catalog/products.json").read_text())["entries"][0]
    timings, rows_seen = [], None
    with tempfile.TemporaryDirectory(prefix="omo-perf-") as tmp:
        tmp_path = Path(tmp)
        artifact = owc.stage_artifact(root, tmp_path)
        stations = tmp_path / "stations.csv"
        with args.stations.open(encoding="utf-8-sig", newline="") as src, stations.open("w", newline="") as dst:
            writer = csv.writer(dst, lineterminator="\n")
            writer.writerow(["station_id", "latitude", "longitude", "name", "elevation_m"])
            for row in csv.DictReader(src):
                writer.writerow([row["id"], row["lat"], row["lon"], row.get("name", ""), ""])
        for run in range(args.runs):
            work = tmp_path / f"run-{run}"
            work.mkdir()
            client = owc.WorkerClient(args.worker.resolve(), owc.bootstrap_config(root, artifact, work))
            try:
                started = time.monotonic()
                body = client.request("series", {
                    "stations_ref": str(stations), "stations_sha256": "perf", "catalog_entry": entry,
                    "catalog_version": "perf", "run": "latest_complete", "window": {"start_h": 0, "end_h": 168},
                    "max_distance_m": entry["max_distance_m"], "result_id": f"perf-{run}",
                    "limits": {"rows": 2_000_000, "bytes": 256 * 1024 * 1024}}, timeout=600)
                timings.append(round(time.monotonic() - started, 2))
                rows_seen = body["result_ref"]["rows"]
                cycle = body["run"]["cycle"]
            finally:
                client.close()
            print(json.dumps({"run": run, "seconds": timings[-1], "rows": rows_seen}), flush=True)
    print(json.dumps({"runs": timings, "median_s": statistics.median(timings), "rows": rows_seen,
                      "cycle": cycle, "target_s": 120}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
