#!/usr/bin/env python3
"""Generate fixtures/stations/m2-regional-100.csv (30 deg x 40 deg East Asia synoptic window).

Box: lat 12..42, lon 100..140. Deterministic: 24 named coastal/inland cities with CJK names,
6 stations on O1280 row-boundary latitudes (midway between adjacent Gaussian rows) and
70 jittered grid-fill stations.
"""
import struct
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "fixtures/stations/m2-regional-100.csv"

CITIES = [
    ("北京", 39.90, 116.41), ("天津", 39.08, 117.20), ("上海", 31.23, 121.47), ("青岛", 36.07, 120.38),
    ("大连", 38.91, 121.61), ("厦门", 24.48, 118.09), ("广州", 23.13, 113.26), ("香港", 22.32, 114.17),
    ("首尔", 37.57, 126.98), ("釜山", 35.18, 129.08), ("东京", 35.68, 139.69), ("大阪", 34.69, 135.50),
    ("福冈", 33.59, 130.40), ("那霸", 26.21, 127.68), ("台北", 25.03, 121.57), ("马尼拉", 14.60, 120.98),
    ("河内", 21.03, 105.85), ("武汉", 30.59, 114.31), ("西安", 34.34, 108.94), ("重庆", 29.56, 106.55),
    ("成都", 30.66, 104.07), ("平壤", 39.04, 125.76), ("海口", 20.04, 110.20), ("杭州", 30.27, 120.15),
]


def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]


def o1280_lat(y, n=1280):
    dy = f32(180.0 / f32(2 * n + 0.5))
    return f32((n - y - 1) * dy) + dy / 2


def main():
    rows = [(n, la, lo) for n, la, lo in CITIES]
    # row-boundary latitudes inside 12..42 (rows y and y+1 straddle the boundary)
    ys = [(1280 - 1) - int(l / 0.0703125) for l in (40.0, 33.0, 26.0, 20.0, 15.0, 13.0)]
    for k, y in enumerate(ys):
        lat = round((o1280_lat(y) + o1280_lat(y + 1)) / 2, 5)
        rows.append((f"行界站{k + 1}", lat, round(101.0 + 6.5 * k, 5)))
    # jittered grid fill (deterministic LCG), 7 x 10 cells over the box
    seed = 20261125
    def rnd():
        nonlocal seed
        seed = (1103515245 * seed + 12345) & 0x7FFFFFFF
        return seed / 0x7FFFFFFF
    i = 0
    for r in range(7):
        for c in range(10):
            i += 1
            lat = 12.0 + (r + 0.15 + 0.7 * rnd()) * (30.0 / 7)
            lon = 100.0 + (c + 0.15 + 0.7 * rnd()) * 4.0
            rows.append((f"区域站{i:02d}", round(lat, 5), round(lon, 5)))
    assert len(rows) == 100, len(rows)
    with OUT.open("w", encoding="utf-8", newline="\n") as f:
        f.write("id,name,lat,lon\n")
        for idx, (n, la, lo) in enumerate(rows, 1):
            f.write(f"{idx},{n},{la:.5f},{lo:.5f}\n")
    print("wrote", OUT)


main()
