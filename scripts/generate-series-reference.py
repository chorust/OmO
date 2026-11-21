#!/usr/bin/env python3
"""Generate the independent M0 station-series oracle from committed source values."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE_MANIFEST = ROOT / "fixtures/reference/station_series_manifest.json"
SOURCE_VALUES = ROOT / "fixtures/reference/duckomo-regions-v0.4.0/regions.reference.csv"
STATIONS = ROOT / "fixtures/stations/m0.csv"
OUTPUT = ROOT / "fixtures/reference/station_series_reference.csv"
SOURCE_SHA256 = "98b8349ae035ef9fc14e4e7f37caf5daa2d44f7de2cad5fe3201534cf4be1b32"
EARTH_RADIUS_M = 6_371_229.0
NX = 5
NY = 5
LAT0 = 20.0
LON0 = 100.0
LAT_STEP = 1.0
LON_STEP = 1.0
SLOTS = 3
STEP_SECONDS = 3_600


def normalize_longitude(longitude: float) -> float:
    normalized = math.fmod(longitude, 360.0)
    if normalized < -180.0:
        normalized += 360.0
    elif normalized >= 180.0:
        normalized -= 360.0
    return normalized


def distance_m(lon_a: float, lat_a: float, lon_b: float, lat_b: float) -> float:
    delta_lon = math.remainder(lon_b - lon_a, 360.0)
    phi_a = math.radians(lat_a)
    phi_b = math.radians(lat_b)
    delta_phi = phi_b - phi_a
    delta_lambda = math.radians(delta_lon)
    sin_lat = math.sin(delta_phi / 2.0)
    sin_lon = math.sin(delta_lambda / 2.0)
    haversine = sin_lat * sin_lat + math.cos(phi_a) * math.cos(phi_b) * sin_lon * sin_lon
    haversine = min(1.0, max(0.0, haversine))
    return EARTH_RADIUS_M * 2.0 * math.atan2(math.sqrt(haversine), math.sqrt(1.0 - haversine))


def nearest_cell(longitude: float, latitude: float) -> tuple[int, float, float, float] | None:
    if not math.isfinite(longitude) or not math.isfinite(latitude):
        raise ValueError("station coordinates must be finite")
    if latitude < LAT0 or latitude > LAT0 + (NY - 1) * LAT_STEP:
        return None
    normalized = normalize_longitude(longitude)
    if normalized < LON0 or normalized > LON0 + (NX - 1) * LON_STEP:
        return None

    candidates: list[tuple[float, int, float, float]] = []
    for y in range(NY):
        grid_lat = LAT0 + y * LAT_STEP
        for x in range(NX):
            grid_lon = LON0 + x * LON_STEP
            point_index = y * NX + x
            candidates.append((distance_m(normalized, latitude, grid_lon, grid_lat), point_index, grid_lat, grid_lon))
    distance, point_index, grid_lat, grid_lon = min(candidates, key=lambda item: (item[0], item[1]))
    return point_index, grid_lat, grid_lon, distance


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    args = parser.parse_args()

    manifest = json.loads(SOURCE_MANIFEST.read_text(encoding="utf-8"))
    array = manifest["array"]
    grid = manifest["grid"]
    parameters = grid["parameters"]
    if array["shape"] != [2, SLOTS, NY, NX] or array["dimensions"] != ["member", "time", "lat", "lon"]:
        raise SystemExit("series fixture manifest no longer matches the declared array layout")
    if (grid["layout"]["nx"], grid["layout"]["ny"]) != (NX, NY):
        raise SystemExit("series fixture grid shape differs from its array shape")
    if grid["type"] != "regular_latlon" or (parameters["lon0"], parameters["lat0"], parameters["dlon"], parameters["dlat"]) != (LON0, LAT0, LON_STEP, LAT_STEP):
        raise SystemExit("series fixture regular grid coordinates changed")
    if grid["earth"]["radius_m"] != EARTH_RADIUS_M:
        raise SystemExit("series fixture earth radius changed")
    source_bytes = SOURCE_VALUES.read_bytes()
    source_hash = hashlib.sha256(source_bytes).hexdigest()
    if source_hash != SOURCE_SHA256 or source_hash != manifest["source_fixture"]["official_value_reference_sha256"]:
        raise SystemExit(f"source value oracle hash mismatch: {source_hash}")
    values_by_index: dict[int, float | None] = {}
    for row in csv.DictReader(source_bytes.decode("utf-8").splitlines()):
        index = int(row["index"])
        value = float(row["value"])
        values_by_index[index] = value if math.isfinite(value) else None
    if set(values_by_index) != set(range(2 * SLOTS * NY * NX)):
        raise SystemExit("source value oracle does not cover the declared [member,time,lat,lon] shape")

    stations: list[dict[str, str]] = []
    with STATIONS.open(newline="", encoding="utf-8") as stream:
        for station in csv.DictReader(stream):
            if not station["station_id"] or station["station_id"] in {item["station_id"] for item in stations}:
                raise SystemExit("station ids must be nonempty and unique")
            stations.append(station)
    if not stations:
        raise SystemExit("station input is empty")

    epoch = datetime(1970, 1, 1, tzinfo=timezone.utc)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow([
            "station_id", "variable", "valid_time", "value", "grid_lat", "grid_lon",
            "distance_m", "expected_point_index",
        ])
        for station in stations:
            station_id = station["station_id"]
            latitude = float(station["latitude"])
            longitude = float(station["longitude"])
            selected = nearest_cell(longitude, latitude)
            for time_index in range(SLOTS):
                valid_time = (epoch + timedelta(seconds=time_index * STEP_SECONDS)).isoformat().replace("+00:00", "Z")
                if selected is None:
                    writer.writerow([station_id, "value", valid_time, "", "", "", "", ""])
                    continue
                point_index, grid_lat, grid_lon, distance = selected
                logical_index = time_index * NX * NY + point_index
                value = values_by_index[logical_index]
                writer.writerow([
                    station_id,
                    "value",
                    valid_time,
                    "" if value is None else format(value, ".9g"),
                    format(grid_lat, ".17g"),
                    format(grid_lon, ".17g"),
                    format(distance, ".17g"),
                    point_index,
                ])


if __name__ == "__main__":
    main()
