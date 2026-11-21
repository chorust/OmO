#!/usr/bin/env python3
"""Compare Plan A and B CSVs field-by-field and summarize their recorded scan metrics."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = [
    "station_id", "variable", "valid_time", "value", "grid_lat", "grid_lon",
    "distance_m", "source_object", "object_version",
]
REFERENCE_HEADER = [
    "station_id", "variable", "valid_time", "value", "grid_lat", "grid_lon",
    "distance_m", "expected_point_index",
]
TOLERANCES = {"value": 1e-6, "grid_lat": 1e-10, "grid_lon": 1e-10, "distance_m": 1e-4}
NX = 5
NY = 5
LAT0 = 20.0
LON0 = 100.0
STEP = 1.0


def read_csv(path: Path, expected_header: list[str]) -> dict[tuple[str, str, str], dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as stream:
        reader = csv.DictReader(stream)
        if reader.fieldnames != expected_header:
            raise ValueError(f"{path}: unexpected CSV schema {reader.fieldnames!r}")
        rows: dict[tuple[str, str, str], dict[str, str]] = {}
        for row in reader:
            key = (row[expected_header[0]], row[expected_header[1]], row[expected_header[2]])
            if key in rows:
                raise ValueError(f"{path}: duplicate key {key}")
            rows[key] = row
    return rows


def optional_float(value: str) -> float | None:
    return None if value == "" else float(value)


def optional_index(value: str) -> int | None:
    return None if value == "" else int(value)


def point_index(row: dict[str, str]) -> int | None:
    latitude = optional_float(row["grid_lat"])
    longitude = optional_float(row["grid_lon"])
    if latitude is None or longitude is None:
        return None
    y = round((latitude - LAT0) / STEP)
    x = round((longitude - LON0) / STEP)
    if not (0 <= x < NX and 0 <= y < NY):
        return None
    if abs(latitude - (LAT0 + y * STEP)) > 1e-10 or abs(longitude - (LON0 + x * STEP)) > 1e-10:
        return None
    return y * NX + x


def compare_value(a: str, b: str, tolerance: float) -> bool:
    left = optional_float(a)
    right = optional_float(b)
    if left is None or right is None:
        return left is right
    return abs(left - right) <= tolerance


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan-a", type=Path, default=ROOT / "docs/m0/evidence/EXIT-2-spark-aarch64-20261117-plan-a.csv")
    parser.add_argument("--plan-b", type=Path, default=ROOT / "docs/m0/evidence/EXIT-2-spark-aarch64-20261117-plan-b.csv")
    parser.add_argument("--report-a", type=Path, default=ROOT / "docs/m0/evidence/EXIT-2-spark-aarch64-20261117-plan-a.report.json")
    parser.add_argument("--report-b", type=Path, default=ROOT / "docs/m0/evidence/EXIT-2-spark-aarch64-20261117-plan-b.report.json")
    parser.add_argument("--reference", type=Path, default=ROOT / "fixtures/reference/station_series_reference.csv")
    parser.add_argument("--output", type=Path, default=ROOT / "docs/m0/evidence/EXIT-2-spark-aarch64-20261117-comparison.json")
    args = parser.parse_args()

    a_rows = read_csv(args.plan_a, SCHEMA)
    b_rows = read_csv(args.plan_b, SCHEMA)
    reference_rows = read_csv(args.reference, REFERENCE_HEADER)
    report_a = json.loads(args.report_a.read_text(encoding="utf-8"))
    report_b = json.loads(args.report_b.read_text(encoding="utf-8"))
    mismatches: list[str] = []
    if set(a_rows) != set(b_rows):
        mismatches.extend(f"plan key mismatch: {key}" for key in sorted(set(a_rows) ^ set(b_rows)))
    if set(a_rows) != set(reference_rows):
        mismatches.extend(f"reference key mismatch: {key}" for key in sorted(set(a_rows) ^ set(reference_rows)))

    compared = 0
    for key in sorted(set(a_rows) & set(b_rows) & set(reference_rows)):
        a = a_rows[key]
        b = b_rows[key]
        reference = reference_rows[key]
        compared += 1
        for field, tolerance in TOLERANCES.items():
            expected = reference[field]
            for plan_name, row in (("A", a), ("B", b)):
                if not compare_value(row[field], expected, tolerance):
                    mismatches.append(f"Plan {plan_name} {field} differs from reference at {key}: {row[field]!r} vs {expected!r}")
            if not compare_value(a[field], b[field], tolerance):
                mismatches.append(f"Plan A/B {field} differs at {key}: {a[field]!r} vs {b[field]!r}")
        for field in ("source_object", "object_version"):
            if a[field] != b[field]:
                mismatches.append(f"Plan A/B {field} differs at {key}: {a[field]!r} vs {b[field]!r}")
        expected_index = optional_index(reference["expected_point_index"])
        actual_index_a = point_index(a)
        actual_index_b = point_index(b)
        if actual_index_a != expected_index:
            mismatches.append(f"Plan A nearest point differs at {key}: expected {expected_index}, actual {actual_index_a}")
        if actual_index_b != expected_index:
            mismatches.append(f"Plan B nearest point differs at {key}: expected {expected_index}, actual {actual_index_b}")
        if not key[2].endswith("Z"):
            mismatches.append(f"non-UTC time at {key}: {key[2]}")

    crs_a = report_a["plan_details"]["grid_info"]["crs"]
    crs_b = report_b["plan_details"]["grid_info"]["crs"]
    if crs_a != crs_b:
        mismatches.append("Plan A/B CRS descriptors differ")
    if crs_a.get("axis_order") != ["longitude", "latitude"] or crs_a.get("earth", {}).get("radius_m") != 6371229:
        mismatches.append(f"unexpected grid CRS descriptor: {crs_a}")
    if report_a["reference_comparison"]["status"] != "passed":
        mismatches.append("Plan A report did not pass the independent reference comparison")
    if report_b["reference_comparison"]["status"] != "passed":
        mismatches.append("Plan B report did not pass the independent reference comparison")

    wrap_checks = []
    for time in sorted({key[2] for key in a_rows}):
        positive_key = ("wrap_0_360", "value", time)
        negative_key = ("wrap_negative", "value", time)
        positive = a_rows.get(positive_key)
        negative = a_rows.get(negative_key)
        if positive is None or negative is None:
            mismatches.append(f"longitude wrap stations missing at {time}")
            continue
        for field in ("value", "grid_lat", "grid_lon", "distance_m", "source_object", "object_version"):
            if field in TOLERANCES:
                same = compare_value(positive[field], negative[field], TOLERANCES[field])
            else:
                same = positive[field] == negative[field]
            if not same:
                mismatches.append(f"longitude wrap stations differ in {field} at {time}")
        wrap_checks.append(time)

    tie_reference = [row for key, row in reference_rows.items() if key[0] == "tie_break"]
    tie_ok = len(tie_reference) == 3 and all(optional_index(row["expected_point_index"]) == 5 for row in tie_reference)
    if not tie_ok:
        mismatches.append("independent fixture does not record the declared row-major tie break at point index 5")

    value_nulls = sum(row["value"] == "" for row in reference_rows.values())
    out_of_domain = sum(row["expected_point_index"] == "" for row in reference_rows.values())
    grid_info_a = report_a["plan_details"]["grid_info"]
    plan_a_scan = report_a["scan_measurement"]
    plan_b_scan = report_b["scan_measurement"]
    plan_a_raw = plan_a_scan["raw_metrics"]
    plan_b_raw = plan_b_scan["raw_metrics"]["per_chunk"][0]["raw_metrics"]
    result: dict[str, Any] = {
        "schema": "omo-series-plan-comparison/1",
        "status": "passed" if not mismatches and compared == len(reference_rows) else "failed",
        "host": {
            "platform": report_a["platform"],
            "channel": report_a["channel"],
            "duckdb_version": report_a["bootstrap"]["duckdb_version"],
            "duckomo_version": report_a["bootstrap"]["duckomo_version"],
            "locked_artifact_sha256": report_a["locked_artifact_sha256"],
            "locked_artifact_bytes": report_a["locked_artifact_bytes"],
            "loaded_extension_sha256": report_a["loaded_extension_sha256"],
        },
        "fixture": {
            "stations": len({key[0] for key in reference_rows}),
            "expected_rows": len(reference_rows),
            "compared_rows": compared,
            "unique_keys_plan_a": len(a_rows),
            "unique_keys_plan_b": len(b_rows),
            "value_null_rows": value_nulls,
            "out_of_domain_rows": out_of_domain,
        },
        "semantic_checks": {
            "stable_schema": a_rows.keys() == b_rows.keys() and report_a["schema_columns"] == report_b["schema_columns"] == SCHEMA,
            "station_variable_time_keys": set(a_rows) == set(b_rows) == set(reference_rows),
            "value_coordinates_distance_and_nearest_index": not mismatches,
            "utc_suffix": all(key[2].endswith("Z") for key in reference_rows),
            "crs_equal_and_lon_lat_order": crs_a == crs_b and crs_a.get("axis_order") == ["longitude", "latitude"],
            "positive_and_negative_longitude_wrap": len(wrap_checks) == 3,
            "tie_break_lowest_row_major_index_5": tie_ok,
            "null_and_out_of_domain_rows": value_nulls > 0 and out_of_domain == 6,
            "provenance_equal_between_plans": not any("source_object" in mismatch or "object_version" in mismatch for mismatch in mismatches),
        },
        "tolerances": TOLERANCES,
        "plan_costs": {
            "A": {
                "query_elapsed_ms": report_a["query_elapsed_ms"],
                "duckomo_value_data_bytes_reported": plan_a_scan.get("data_bytes"),
                "physical_read_bytes_reported": plan_a_scan.get("physical_read_bytes"),
                "metadata_bytes_reported": plan_a_scan.get("metadata_bytes"),
                "index_bytes_reported": plan_a_scan.get("index_bytes"),
                "scan_complete": plan_a_scan.get("scan_complete"),
                "candidate_rows": plan_a_raw.get("legacy_v3", {}).get("candidate_rows"),
                "full_grid_points": report_a["plan_details"].get("full_grid_points"),
                "selected_unique_grid_points": report_a["plan_details"].get("selected_unique_grid_points"),
                "slice_feasibility": report_a["plan_details"].get("slice_feasibility"),
                "raw_metrics_path": str(args.report_a),
            },
            "B": {
                "query_elapsed_ms": report_b["query_elapsed_ms"],
                "duckomo_value_data_bytes_reported": plan_b_scan.get("data_bytes"),
                "physical_read_bytes_reported": plan_b_scan.get("physical_read_bytes"),
                "metadata_bytes_reported": plan_b_scan.get("metadata_bytes"),
                "index_bytes_reported": plan_b_scan.get("index_bytes"),
                "scan_complete": plan_b_scan.get("scan_complete"),
                "candidate_rows": plan_b_raw.get("legacy_v3", {}).get("candidate_rows"),
                "region_candidates": report_b["plan_details"].get("region_count"),
                "raw_metrics_path": str(args.report_b),
            },
        },
        "measurement_limits": [
            "duckomo_last_scan_metrics values are reported separately as value data, metadata, index and physical read bytes; they are not interchangeable.",
            "Plan A reports scan_complete=false with zero value bytes on this local 488-byte fixture; that zero is not evidence of zero physical work or a complete data-byte measurement.",
            "The one-chunk synthetic fixture proves semantics and API behavior, not production-grid cost or scaling.",
            "Runtime overlap is absent from this one-chunk fixture; duplicate handling is separately covered by the engine schema tests, including conflicting duplicate rejection.",
        ],
        "mismatches": mismatches[:100],
        "artifacts": {
            "plan_a_csv": {"path": str(args.plan_a), "sha256": sha256(args.plan_a)},
            "plan_b_csv": {"path": str(args.plan_b), "sha256": sha256(args.plan_b)},
            "reference_csv": {"path": str(args.reference), "sha256": sha256(args.reference)},
            "plan_a_report": str(args.report_a),
            "plan_b_report": str(args.report_b),
            "grid_type": grid_info_a.get("grid_type"),
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": result["status"], "output": str(args.output), "mismatches": result["mismatches"]}, indent=2))
    if result["status"] != "passed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
