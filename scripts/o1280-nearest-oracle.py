#!/usr/bin/env python3
"""Independent brute-force O1280 nearest-point oracle (research R-09)."""

import argparse
import csv
import math
import struct
from pathlib import Path

N = 1280
ROWS = 2 * N
EARTH_RADIUS_M = 6_371_008.8


def f32(x: float) -> float:
    return struct.unpack("f", struct.pack("f", x))[0]


def nx(y: int) -> int:
    return 20 + 4 * min(y, 2 * N - y - 1)


def build_row_table():
    two_n = f32(2.0 * N + 0.5)
    dy = f32(180.0 / two_n)
    offsets = [0]
    lats = []
    for y in range(ROWS):
        offsets.append(offsets[-1] + nx(y))
        scaled = f32(f32(N - y - 1) * dy)
        lats.append(f32(scaled + f32(dy / 2.0)))
    assert offsets[-1] == 6_599_680, f"expected 6599680 points, got {offsets[-1]}"
    return offsets, lats


def normalize_lon(lon: float) -> float:
    wrapped = lon % 360.0
    if wrapped >= 180.0:
        wrapped -= 360.0
    if wrapped < -180.0:
        wrapped += 360.0
    return 0.0 if wrapped == 0.0 else wrapped


def haversine_m(lat1: float, lon1: float, lat2: float, lon2: float) -> float:
    if lat1 == lat2 and normalize_lon(lon1) == normalize_lon(lon2):
        return 0.0
    dlon = math.radians(((lon2 - lon1 + 180.0) % 360.0) - 180.0)
    p1 = math.radians(lat1)
    p2 = math.radians(lat2)
    dp = p2 - p1
    a = min(1.0, max(0.0, math.sin(dp / 2.0) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dlon / 2.0) ** 2))
    return EARTH_RADIUS_M * 2.0 * math.atan2(math.sqrt(a), math.sqrt(1.0 - a))


def lon_f32(x: int, count: int) -> float:
    dlon = f32(360.0 / float(count))
    raw = f32(float(x) * dlon)
    if raw >= 180.0:
        raw = f32(raw - 360.0)
    elif raw < -180.0:
        raw = f32(raw + 360.0)
    return 0.0 if raw == 0.0 else raw


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--stations", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    offsets, lats = build_row_table()
    with args.stations.open("r", encoding="utf-8-sig", newline="") as handle:
        reader = csv.DictReader(handle)
        headers = {h.strip().lower(): h for h in (reader.fieldnames or [])}
        id_col = next(headers[k] for k in ("station_id", "id", "code", "name_id") if k in headers)
        lat_col = next(headers[k] for k in ("lat", "latitude", "y") if k in headers)
        lon_col = next(headers[k] for k in ("lon", "lng", "longitude", "x") if k in headers)
        stations = [(r[id_col].strip(), float(r[lat_col]), float(r[lon_col])) for r in reader]

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, lineterminator="\n")
        writer.writerow(["station_id", "flat_index", "grid_lat", "grid_lon", "distance_m"])
        for sid, lat, lon in stations:
            if not (-90.0 <= lat <= 90.0) or not math.isfinite(lat) or not math.isfinite(lon):
                continue
            center = min(range(ROWS), key=lambda y: abs(lats[y] - lat))
            y0 = max(0, center - 3)
            y1 = min(ROWS - 1, center + 3)
            best = None
            for y in range(y0, y1 + 1):
                count = nx(y)
                glat = lats[y]
                base = offsets[y]
                for x in range(count):
                    glon = lon_f32(x, count)
                    dist = haversine_m(lat, lon, glat, glon)
                    flat = base + x
                    cand = (dist, flat, glat, glon)
                    if best is None or cand[0] < best[0] or (cand[0] == best[0] and cand[1] < best[1]):
                        best = cand
            assert best is not None
            dist, flat, glat, glon = best
            writer.writerow([sid, flat, repr(glat), repr(glon), repr(dist)])


if __name__ == "__main__":
    main()
