#!/usr/bin/env python3
"""SC-003: `map_inspect` readouts against the committed O1280 oracle (57 reference stations, pinned run).

For every station of `fixtures/stations/m1-oracle.csv` (exact nodes, off-grid points, tie-break, longitude
wraps, antimeridian, both poles, a global spread) the worker loads a small field around the point for the
pinned run and valid time, `map_inspect` reads the nearest grid point, and the result is compared with
`fixtures/reference/o1280-20261123T06Z/{nearest,values}.csv`:

  * the matched grid coordinates must equal the oracle's (`|d| <= 1e-9`);
  * the native numeric value, rounded to float32, must equal the oracle value (`max |delta| == 0.0`),
    and a masked/null oracle cell must read back as `no_data`.
"""
from __future__ import annotations

import argparse
import csv
import json
import struct
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import omo_worker_client as owc  # noqa: E402

PAD = 0.3


def f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", value))[0]


def norm_lon(lon: float) -> float:
    return ((lon + 180.0) % 360.0) - 180.0


def bbox_for(lat: float, lon: float) -> dict:
    lon = norm_lon(lon)
    lat_min, lat_max = max(-90.0, lat - PAD), min(90.0, lat + PAD)
    if abs(lat) > 85.0:
        spans = [[-180.0, 180.0]]
    else:
        lo, hi = lon - PAD, lon + PAD
        if lo < -180.0:
            spans = [[-180.0, hi], [lo + 360.0, 180.0]]
        elif hi > 180.0:
            spans = [[lo, 180.0], [-180.0, hi - 360.0]]
        else:
            spans = [[lo, hi]]
    return {"lat_min": lat_min, "lat_max": lat_max, "lon_spans": spans}


def read_oracle(root: Path, run_dir: str) -> tuple[dict, dict]:
    base = root / "fixtures/reference" / run_dir
    with (base / "nearest.csv").open(newline="") as handle:
        nearest = {row["station_id"]: row for row in csv.DictReader(handle)}
    values: dict[tuple[str, str], str] = {}
    with (base / "values.csv").open() as handle:
        lines = [line for line in handle if not line.startswith("#")]
    for row in csv.DictReader(lines):
        values[(row["station_id"], row["valid_time"])] = row["value"].strip()
    return nearest, values


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", type=Path, default=Path("target/release/omo-worker"))
    parser.add_argument("--run", default="2026-11-23T06:00:00Z")
    parser.add_argument("--valid-times", default="2026-11-23T06:00Z,2026-11-23T18:00Z")
    parser.add_argument("--stations", type=Path, default=Path("fixtures/stations/m1-oracle.csv"))
    parser.add_argument("--force-data-run", action="store_true")
    parser.add_argument("--timeout", type=float, default=900.0)
    args = parser.parse_args()
    root = Path.cwd()
    entry = json.loads((root / "catalog/products.json").read_text())["entries"][0]
    nearest, oracle = read_oracle(root, "o1280-20261123T06Z")
    with args.stations.open(encoding="utf-8-sig", newline="") as handle:
        stations = list(csv.DictReader(handle))
    report: dict = {"scenario": "map_inspect_parity", "run": args.run, "stations": len(stations)}
    problems: list[str] = []
    compared = coord_exact = values_equal = nulls_ok = 0
    worst = 0.0
    with tempfile.TemporaryDirectory(prefix="omo-sc003-") as tmp:
        tmp_path = Path(tmp)
        artifact = owc.stage_artifact(root, tmp_path)
        client = owc.WorkerClient(args.worker.resolve(), owc.bootstrap_config(root, artifact, tmp_path))
        started = time.monotonic()
        try:
            for valid in args.valid_times.split(","):
                valid_rfc = valid if valid.count(":") == 2 else valid[:-1] + ":00Z"
                for index, station in enumerate(stations):
                    sid, lat, lon = station["id"], float(station["lat"]), float(station["lon"])
                    ref = nearest.get(sid)
                    if ref is None:
                        problems.append(f"{sid}: not in the oracle")
                        continue
                    field_id = f"sc003-{valid}-{index}"
                    loaded = client.request("map_load_field", {
                        "field_id": field_id, "catalog_entry": entry, "catalog_version": "sc003",
                        "run": args.run, "valid_time_utc": valid_rfc, "bbox": bbox_for(lat, lon),
                        "force_data_run": args.force_data_run,
                    }, timeout=args.timeout)
                    probe = client.request("map_inspect", {
                        "field_id": field_id, "latitude": lat, "longitude": norm_lon(lon),
                        "max_distance_m": entry["max_distance_m"],
                    }, timeout=args.timeout)
                    sample = probe["sample"]
                    state = sample["state"]
                    expected_text = oracle.get((sid, valid))
                    compared += 1
                    if state not in ("value", "no_data"):
                        problems.append(f"{sid}@{valid}: unexpected sample state {state}")
                        continue
                    grid_ok = (abs(sample["grid_lat"] - float(ref["grid_lat"])) <= 1e-9
                               and abs(norm_lon(sample["grid_lon"]) - norm_lon(float(ref["grid_lon"]))) <= 1e-9)
                    if grid_ok:
                        coord_exact += 1
                    else:
                        problems.append(f"{sid}: grid ({sample['grid_lat']}, {sample['grid_lon']}) != "
                                        f"oracle ({ref['grid_lat']}, {ref['grid_lon']})")
                    if expected_text is None:
                        problems.append(f"{sid}@{valid}: oracle has no value for this time")
                    elif expected_text == "":
                        if state == "no_data":
                            nulls_ok += 1
                        else:
                            problems.append(f"{sid}@{valid}: oracle is masked, map read {sample.get('value')}")
                    elif state != "value":
                        problems.append(f"{sid}@{valid}: oracle {expected_text}, map read no_data")
                    else:
                        delta = abs(f32(sample["value"]) - f32(float(expected_text)))
                        worst = max(worst, delta)
                        if delta == 0.0:
                            values_equal += 1
                        else:
                            problems.append(f"{sid}@{valid}: {sample['value']} != {expected_text}")
                        if sample["provenance"] != "native_numeric":
                            problems.append(f"{sid}@{valid}: provenance {sample['provenance']}")
                    print(json.dumps({"station": sid, "valid": valid, "layout": loaded["source_layout"],
                                      "state": state, "value": sample.get("value")}), flush=True)
        finally:
            client.close()
        report["elapsed_s"] = round(time.monotonic() - started, 1)
    report.update({
        "compared": compared, "coordinates_exact": coord_exact, "values_equal": values_equal,
        "masked_cells_ok": nulls_ok, "max_abs_delta": worst, "problems": problems[:20],
        "status": "passed" if not problems and compared > 0 else "failed",
    })
    print(json.dumps(report, indent=2))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
