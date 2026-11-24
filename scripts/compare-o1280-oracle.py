#!/usr/bin/env python3
"""SC-003: compare an app CSV export of a pinned run with the committed O1280 oracle output.

Grid coordinates (hence the flat index) must match 100%; values must agree at 0.05 degC
resolution, including the positions of nulls.
"""
from __future__ import annotations

import argparse
import csv
import json
import math
import sys
from pathlib import Path

COORD_TOLERANCE = 1e-9
VALUE_RESOLUTION = 0.05


def norm_time(value: str) -> str:
    """`2026-11-23T06:00Z` and `2026-11-23T06:00:00Z` compare equal."""
    value = value.strip()
    if len(value) == 17 and value.endswith("Z"):
        return value[:-1] + ":00Z"
    return value


def read_csv(path: Path) -> list[dict]:
    with path.open(encoding="utf-8", newline="") as handle:
        lines = [line for line in handle if not line.startswith("#")]
    return list(csv.DictReader(lines))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--nearest", required=True, type=Path)
    parser.add_argument("--values", required=True, type=Path)
    parser.add_argument("--app-csv", required=True, type=Path)
    args = parser.parse_args()

    nearest = {r["station_id"]: r for r in read_csv(args.nearest)}
    oracle = {(r["station_id"], norm_time(r["valid_time"])): r["value"].strip() for r in read_csv(args.values)}
    app_rows = read_csv(args.app_csv)

    problems: list[str] = []
    app_values: dict[tuple[str, str], str] = {}
    app_coords: dict[str, tuple[float, float]] = {}
    for row in app_rows:
        sid = row["station_id"]
        app_values[(sid, norm_time(row["valid_time"]))] = (row.get("value") or "").strip()
        app_coords[sid] = (float(row["grid_lat"]), float(row["grid_lon"]))

    missing = sorted(set(nearest) - set(app_coords))
    extra = sorted(set(app_coords) - set(nearest))
    if missing:
        problems.append(f"stations absent from the app export: {missing}")
    if extra:
        problems.append(f"stations the oracle does not match: {extra}")

    coordinate_mismatches = []
    for sid, (lat, lon) in app_coords.items():
        ref = nearest.get(sid)
        if ref is None:
            continue
        if abs(lat - float(ref["grid_lat"])) > COORD_TOLERANCE or abs(lon - float(ref["grid_lon"])) > COORD_TOLERANCE:
            coordinate_mismatches.append((sid, (lat, lon), (ref["grid_lat"], ref["grid_lon"])))
    if coordinate_mismatches:
        problems.append(f"{len(coordinate_mismatches)} coordinate mismatches, e.g. {coordinate_mismatches[:3]}")

    compared = null_mismatch = value_mismatch = 0
    worst = 0.0
    keys = {key for key in oracle if key[0] in app_coords}
    for key in sorted(keys | {k for k in app_values if k[0] in nearest}):
        if key not in oracle or key not in app_values:
            problems.append(f"time axis differs for {key}")
            continue
        compared += 1
        expected, actual = oracle[key], app_values[key]
        if (expected == "") != (actual == ""):
            null_mismatch += 1
            continue
        if expected == "":
            continue
        delta = abs(float(expected) - float(actual))
        worst = max(worst, delta)
        if delta > VALUE_RESOLUTION or math.isnan(delta):
            value_mismatch += 1
    if null_mismatch:
        problems.append(f"{null_mismatch} null-position mismatches")
    if value_mismatch:
        problems.append(f"{value_mismatch} values differ by more than {VALUE_RESOLUTION}")

    report = {
        "status": "failed" if problems else "passed",
        "stations_oracle": len(nearest), "stations_app": len(app_coords),
        "coordinate_exact": len(app_coords) - len(coordinate_mismatches),
        "values_compared": compared, "max_abs_delta": round(worst, 6), "problems": problems[:10],
    }
    print(json.dumps(report, indent=2))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
