#!/usr/bin/env python3
"""Pack Natural Earth 1:50m coastlines + land admin-0 boundary lines into the `OCB1` asset.

Deterministic, standard-library only. Inputs are the public-domain Natural Earth shapefiles
(`ne_50m_coastline`, `ne_50m_admin_0_boundary_lines_land`); see `fixtures/PROVENANCE.md`.

OCB1 layout (little-endian), see specs/003-m2-regional-tile-map/contracts/layer-stack-and-pmtiles.md §3:
  header 16 B : magic b"OCB1", version u16=1, flags u16=0, line_count u32, vertex_count u32
  index       : line_count x 32 B
                min_lon_e4 i32, min_lat_e4 i32, max_lon_e4 i32, max_lat_e4 i32,
                first_lon_e4 i32, first_lat_e4 i32, delta_offset u32 (entry index into `deltas`),
                vertex_len u16, feature_kind u8 (0 coastline, 1 admin0 border), pad u8
  deltas      : (vertex_count - line_count) x 4 B  (dlon_e4 i16, dlat_e4 i16)
"""

from __future__ import annotations

import argparse
import hashlib
import struct
import sys
import zipfile
from pathlib import Path

MAX_DELTA = 32767
MAX_VERTS = 65535
REPO_ROOT = Path(__file__).resolve().parent.parent

SOURCES = (
    ("ne_50m_coastline", 0),
    ("ne_50m_admin_0_boundary_lines_land", 1),
)


def read_polylines(shp: bytes):
    """Yield one list of (lon, lat) tuples per polyline part (shape types 3/13/23)."""
    pos = 100
    n = len(shp)
    while pos + 8 <= n:
        _rec_no, content_words = struct.unpack(">ii", shp[pos : pos + 8])
        body = shp[pos + 8 : pos + 8 + content_words * 2]
        pos += 8 + content_words * 2
        shape_type = struct.unpack("<i", body[:4])[0]
        if shape_type == 0:
            continue
        if shape_type not in (3, 13, 23):
            raise SystemExit(f"unsupported shape type {shape_type}")
        num_parts, num_points = struct.unpack("<ii", body[36:44])
        parts = struct.unpack(f"<{num_parts}i", body[44 : 44 + 4 * num_parts])
        pts_off = 44 + 4 * num_parts
        pts = struct.unpack(f"<{num_points * 2}d", body[pts_off : pts_off + 16 * num_points])
        coords = [(pts[2 * i], pts[2 * i + 1]) for i in range(num_points)]
        bounds = list(parts) + [num_points]
        for a, b in zip(bounds[:-1], bounds[1:]):
            if b - a >= 2:
                yield coords[a:b]


def quantize(v: float) -> int:
    # round half away from zero, deterministic
    return int(v * 10000.0 + 0.5) if v >= 0 else -int(-v * 10000.0 + 0.5)


def split_line(points):
    """Split a quantized polyline on i16 delta overflow or u16 vertex overflow."""
    out = []
    cur = [points[0]]
    for p in points[1:]:
        dx, dy = p[0] - cur[-1][0], p[1] - cur[-1][1]
        if abs(dx) > MAX_DELTA or abs(dy) > MAX_DELTA:
            if len(cur) >= 2:
                out.append(cur)
            cur = [p]
            continue
        if len(cur) >= MAX_VERTS:
            out.append(cur)
            cur = [cur[-1]]
        cur.append(p)
    if len(cur) >= 2:
        out.append(cur)
    return out


def pack(source_dir: Path) -> bytes:
    lines = []  # (kind, [(lon_e4, lat_e4), ...])
    for stem, kind in SOURCES:
        zpath = source_dir / f"{stem}.zip"
        if zpath.is_file():
            with zipfile.ZipFile(zpath) as z:
                shp = z.read(f"{stem}.shp")
        else:
            shp = (source_dir / stem / f"{stem}.shp").read_bytes()
        for part in read_polylines(shp):
            q = []
            for lon, lat in part:
                p = (quantize(lon), quantize(lat))
                if not q or q[-1] != p:
                    q.append(p)
            if len(q) >= 2:
                for seg in split_line(q):
                    lines.append((kind, seg))

    index = bytearray()
    deltas = bytearray()
    vertex_count = 0
    delta_entries = 0
    for kind, pts in lines:
        lons = [p[0] for p in pts]
        lats = [p[1] for p in pts]
        index += struct.pack(
            "<6iIHBB",
            min(lons), min(lats), max(lons), max(lats),
            pts[0][0], pts[0][1], delta_entries, len(pts), kind, 0,
        )
        for a, b in zip(pts[:-1], pts[1:]):
            deltas += struct.pack("<hh", b[0] - a[0], b[1] - a[1])
            delta_entries += 1
        vertex_count += len(pts)
    header = b"OCB1" + struct.pack("<HHII", 1, 0, len(lines), vertex_count)
    assert len(index) == 32 * len(lines)
    assert len(deltas) == 4 * (vertex_count - len(lines))
    return header + bytes(index) + bytes(deltas)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--source-dir", type=Path, required=True,
                    help="directory holding the Natural Earth .zip files (or extracted folders)")
    ap.add_argument("--out", type=Path, default=REPO_ROOT / "fixtures/geodata/ne_50m_coast_borders.ocb1")
    args = ap.parse_args()
    data = pack(args.source_dir)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_bytes(data)
    lc, vc = struct.unpack("<II", data[8:16])
    print(f"wrote {args.out} ({len(data)} bytes, lines={lc}, vertices={vc}, "
          f"sha256={hashlib.sha256(data).hexdigest()})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
