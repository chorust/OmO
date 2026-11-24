#!/usr/bin/env python3
"""Run the app's series path headlessly for a pinned run and export the result as CSV (V2 / T062)."""
from __future__ import annotations

import argparse
import csv
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import omo_worker_client as owc  # noqa: E402


def canonical_stations(source: Path, target: Path) -> int:
    with source.open(encoding="utf-8-sig", newline="") as handle:
        rows = list(csv.DictReader(handle))
    with target.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, lineterminator="\n")
        writer.writerow(["station_id", "latitude", "longitude", "name", "elevation_m"])
        for row in rows:
            writer.writerow([row["id"], row["lat"], row["lon"], row.get("name", ""), ""])
    return len(rows)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--stations", type=Path, default=Path("fixtures/stations/m1-oracle.csv"))
    parser.add_argument("--run", required=True, help="reference time, e.g. 2026-11-23T06:00:00Z")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--worker", type=Path, default=Path("target/debug/omo-worker"))
    parser.add_argument("--timeout", type=float, default=900.0)
    args = parser.parse_args()
    root = Path.cwd()
    entry = json.loads((root / "catalog/products.json").read_text())["entries"][0]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="omo-pinned-series-") as tmp:
        tmp_path = Path(tmp)
        artifact = owc.stage_artifact(root, tmp_path)
        client = owc.WorkerClient(args.worker.resolve(), owc.bootstrap_config(root, artifact, tmp_path))
        try:
            stations = tmp_path / "stations.csv"
            count = canonical_stations(args.stations, stations)
            body = client.request("series", {
                "stations_ref": str(stations), "stations_sha256": "pinned", "catalog_entry": entry,
                "catalog_version": "pinned", "run": {"reference_time_utc": args.run},
                "window": {"start_h": 0, "end_h": 168}, "max_distance_m": entry["max_distance_m"],
                "result_id": "pinned", "limits": {"rows": 2_000_000, "bytes": 256 * 1024 * 1024},
            }, timeout=args.timeout)
            parquet = body["parquet_path"].replace("'", "''")
            sql = ("SELECT station_id, valid_time, value, grid_lat, grid_lon, distance_m "
                   f"FROM read_parquet('{parquet}') ORDER BY station_id, valid_time")
            export = client.send("export", {
                "sql": sql, "target": str(args.output.resolve()), "format": "csv",
                "job_snapshot": {"job_id": 1}, "overwrite_confirmed": True})
            done = client.wait("export_committed", export, timeout=args.timeout)
            print(json.dumps({"stations": count, "run": body["run"]["reference_time_utc"],
                              "rows": body["result_ref"]["rows"], "plan": body["plan"],
                              "effective_window": body["effective_window"], "csv": str(args.output),
                              "data_sha256": done["data_sha256"]}))
        finally:
            client.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
