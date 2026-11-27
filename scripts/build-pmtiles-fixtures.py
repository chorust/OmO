#!/usr/bin/env python3
"""Generate deterministic offline PMTiles v3 test archives for feature 003 (M2).

Outputs (fixtures/pmtiles/):
  sample-raster.pmtiles       PNG raster, z0..3, tile compression none, global bounds
  sample-raster-jpeg.pmtiles  JPEG raster, z0..2 (Pillow encoder)
  sample-raster-webp.pmtiles  WebP raster, z0..2 (Pillow encoder, lossless)
  sample-mvt.pmtiles          geometry-only MVT, z0..3, tile compression gzip
  sample-mvt-zstd.pmtiles     same MVT, tile compression zstd (stored/raw-block frames)
  corrupt.pmtiles             valid header+directory, corrupted tile payloads

Pure standard library except JPEG/WebP encoding, which uses Pillow (dev-only; the Rust
runtime never depends on it). Output bytes are deterministic for a given Pillow build.
"""

from __future__ import annotations

import gzip
import hashlib
import json
import struct
import sys
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "fixtures/pmtiles"

COMP_NONE, COMP_GZIP, COMP_BROTLI, COMP_ZSTD = 1, 2, 3, 4
T_MVT, T_PNG, T_JPEG, T_WEBP = 1, 2, 3, 4


def varint(n: int) -> bytes:
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def rotate(n, x, y, rx, ry):
    if ry == 0:
        if rx == 1:
            x = n - 1 - x
            y = n - 1 - y
        x, y = y, x
    return x, y


def zxy_to_tileid(z: int, x: int, y: int) -> int:
    n = 1 << z
    acc = sum((1 << t) * (1 << t) for t in range(z))
    d = 0
    s = n // 2
    tx, ty = x, y
    while s > 0:
        rx = 1 if (tx & s) > 0 else 0
        ry = 1 if (ty & s) > 0 else 0
        d += s * s * ((3 * rx) ^ ry)
        tx, ty = rotate(s, tx, ty, rx, ry)
        s //= 2
    return acc + d


def gz(data: bytes) -> bytes:
    # mtime=0 for determinism
    return gzip.compress(data, compresslevel=9, mtime=0)


def zstd_raw_frame(data: bytes) -> bytes:
    """A valid zstd frame made of raw (stored) blocks; decodable by any conforming decoder."""
    out = bytearray(struct.pack("<I", 0xFD2FB528))
    # Frame header descriptor: single-segment, 4-byte content size (FCS flag = 2 -> 4 bytes).
    out.append(0b10100000)
    out += struct.pack("<I", len(data))
    max_block = 128 * 1024
    pos = 0
    if not data:
        out += (1).to_bytes(3, "little")
    while pos < len(data):
        chunk = data[pos : pos + max_block]
        pos += len(chunk)
        last = 1 if pos >= len(data) else 0
        header = (len(chunk) << 3) | (0 << 1) | last  # block type 0 = Raw
        out += header.to_bytes(3, "little")
        out += chunk
    return bytes(out)


def png_rgba(width: int, height: int, rgba: bytes) -> bytes:
    def chunk(tag: bytes, body: bytes) -> bytes:
        return struct.pack(">I", len(body)) + tag + body + struct.pack(">I", zlib.crc32(tag + body))

    raw = bytearray()
    stride = width * 4
    for row in range(height):
        raw.append(0)
        raw += rgba[row * stride : (row + 1) * stride]
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def tile_pixels(z: int, x: int, y: int) -> bytes:
    """Deterministic 256x256 RGBA pattern: per-tile base colour, border, diagonal alpha ramp."""
    h = hashlib.sha256(f"{z}/{x}/{y}".encode()).digest()
    base = (60 + h[0] % 160, 60 + h[1] % 160, 60 + h[2] % 160)
    px = bytearray()
    for v in range(256):
        for u in range(256):
            border = u < 2 or v < 2 or u >= 254 or v >= 254
            if border:
                px += bytes((255, 255, 255, 255))
            else:
                a = 96 + (u + v) // 3
                px += bytes((base[0], base[1], base[2], min(a, 255)))
    return bytes(px)


def mvt_tile(z: int) -> bytes:
    """Geometry-only Mapbox Vector Tile v2: earth + water polygons, boundary line."""

    def pb_varint(n: int) -> bytes:
        return varint(n)

    def key(field: int, wire: int) -> bytes:
        return pb_varint((field << 3) | wire)

    def ld(field: int, payload: bytes) -> bytes:
        return key(field, 2) + pb_varint(len(payload)) + payload

    def zigzag(n: int) -> int:
        return (n << 1) ^ (n >> 31)

    def cmd(cid: int, count: int) -> int:
        return (count << 3) | cid

    def polygon(points):
        g = [cmd(1, 1), zigzag(points[0][0]), zigzag(points[0][1])]
        g.append(cmd(2, len(points) - 1))
        px, py = points[0]
        for (x, y) in points[1:]:
            g += [zigzag(x - px), zigzag(y - py)]
            px, py = x, y
        g.append(cmd(7, 1))
        return g

    def line(points):
        g = [cmd(1, 1), zigzag(points[0][0]), zigzag(points[0][1])]
        g.append(cmd(2, len(points) - 1))
        px, py = points[0]
        for (x, y) in points[1:]:
            g += [zigzag(x - px), zigzag(y - py)]
            px, py = x, y
        return g

    def feature(geom_type: int, geometry, tags=()):
        body = b""
        if tags:
            body += ld(2, b"".join(pb_varint(t) for t in tags))
        body += key(3, 0) + pb_varint(geom_type)
        body += ld(4, b"".join(pb_varint(g) for g in geometry))
        return body

    def layer(name: str, features, keys=(), values=()):
        body = key(15, 0) + pb_varint(2) + ld(1, name.encode())
        for f in features:
            body += ld(2, f)
        for k in keys:
            body += ld(3, k.encode())
        for v in values:
            body += ld(4, key(5, 0) + pb_varint(v))  # Value.uint_value = field 5
        body += key(5, 0) + pb_varint(4096)
        return body

    extent = 4096
    earth = polygon([(0, 0), (extent, 0), (extent, extent), (0, extent)])
    water = polygon([(0, 0), (extent // 2 + 256 * (z % 2), 0), (extent // 2, extent), (0, extent)])
    boundary = line([(0, 0), (extent // 2, extent // 3), (extent, extent // 2)])
    tile = b""
    tile += ld(3, layer("earth", [feature(3, earth)]))
    tile += ld(3, layer("water", [feature(3, water)]))
    tile += ld(3, layer("boundary", [feature(2, boundary, tags=(0, 0))], keys=("admin_level",), values=(2,)))
    return tile


def directory(entries):
    """entries: list of (tile_id, offset, length) sorted by tile_id, run_length 1."""
    out = bytearray(varint(len(entries)))
    last = 0
    for tid, _, _ in entries:
        out += varint(tid - last)
        last = tid
    for _ in entries:
        out += varint(1)
    for _, _, length in entries:
        out += varint(length)
    prev_end = None
    for _, offset, length in entries:
        if prev_end is not None and offset == prev_end:
            out += varint(0)
        else:
            out += varint(offset + 1)
        prev_end = offset + length
    return bytes(out)


def build(tiles, *, tile_type, tile_comp, min_zoom, max_zoom, metadata, bounds=(-180.0, -85.0511287, 180.0, 85.0511287)):
    tiles = sorted(tiles, key=lambda t: t[0])
    data = bytearray()
    entries = []
    for tid, payload in tiles:
        entries.append((tid, len(data), len(payload)))
        data += payload
    root = gz(directory(entries))
    meta = gz(json.dumps(metadata, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode())
    root_off = 127
    meta_off = root_off + len(root)
    leaf_off = meta_off + len(meta)
    data_off = leaf_off
    e7 = lambda v: int(round(v * 1e7))
    header = bytearray(b"PMTiles") + bytes([3])
    header += struct.pack("<QQQQQQ", root_off, len(root), meta_off, len(meta), leaf_off, 0)
    header += struct.pack("<QQQQQ", data_off, len(data), len(tiles), len(tiles), len(tiles))
    header += bytes([1, COMP_GZIP, tile_comp, tile_type, min_zoom, max_zoom])
    header += struct.pack("<iiii", e7(bounds[0]), e7(bounds[1]), e7(bounds[2]), e7(bounds[3]))
    header += bytes([min_zoom])
    header += struct.pack("<ii", 0, 0)
    assert len(header) == 127, len(header)
    return bytes(header) + root + meta + bytes(data)


def zxy_all(max_zoom):
    for z in range(max_zoom + 1):
        for x in range(1 << z):
            for y in range(1 << z):
                yield z, x, y


def write(name: str, blob: bytes) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / name).write_bytes(blob)
    print(f"{name}\t{len(blob)}\t{hashlib.sha256(blob).hexdigest()}")


def main() -> int:
    from io import BytesIO

    from PIL import Image

    raster_meta = {
        "name": "omo-sample-raster",
        "attribution": "OMO test fixture (synthetic, public domain)",
        "type": "overlay",
        "format": "png",
    }
    png_tiles = [(zxy_to_tileid(z, x, y), png_rgba(256, 256, tile_pixels(z, x, y))) for z, x, y in zxy_all(3)]
    write("sample-raster.pmtiles", build(png_tiles, tile_type=T_PNG, tile_comp=COMP_NONE, min_zoom=0, max_zoom=3, metadata=raster_meta))

    def encode(fmt, z, x, y, **kw):
        img = Image.frombytes("RGBA", (256, 256), tile_pixels(z, x, y))
        if fmt == "JPEG":
            img = img.convert("RGB")
        buf = BytesIO()
        img.save(buf, fmt, **kw)
        return buf.getvalue()

    jpeg_tiles = [(zxy_to_tileid(z, x, y), encode("JPEG", z, x, y, quality=92)) for z, x, y in zxy_all(2)]
    write("sample-raster-jpeg.pmtiles", build(jpeg_tiles, tile_type=T_JPEG, tile_comp=COMP_NONE, min_zoom=0, max_zoom=2,
                                              metadata={**raster_meta, "name": "omo-sample-raster-jpeg", "format": "jpg"}))
    webp_tiles = [(zxy_to_tileid(z, x, y), encode("WEBP", z, x, y, lossless=True)) for z, x, y in zxy_all(2)]
    write("sample-raster-webp.pmtiles", build(webp_tiles, tile_type=T_WEBP, tile_comp=COMP_NONE, min_zoom=0, max_zoom=2,
                                              metadata={**raster_meta, "name": "omo-sample-raster-webp", "format": "webp"}))

    mvt_meta = {
        "name": "omo-sample-mvt",
        "attribution": "OMO test fixture (synthetic, public domain)",
        "format": "pbf",
        "vector_layers": [{"id": "earth"}, {"id": "water"}, {"id": "boundary"}],
    }
    raw = {zxy[0:3]: mvt_tile(zxy[0]) for zxy in zxy_all(3)}
    write("sample-mvt.pmtiles", build([(zxy_to_tileid(*k), gz(v)) for k, v in raw.items()], tile_type=T_MVT, tile_comp=COMP_GZIP,
                                      min_zoom=0, max_zoom=3, metadata=mvt_meta))
    write("sample-mvt-zstd.pmtiles", build([(zxy_to_tileid(*k), zstd_raw_frame(v)) for k, v in raw.items()], tile_type=T_MVT,
                                           tile_comp=COMP_ZSTD, min_zoom=0, max_zoom=3,
                                           metadata={**mvt_meta, "name": "omo-sample-mvt-zstd"}))

    corrupt_tiles = [(tid, b"\x00corrupt-payload\xff" * 3) for tid, _ in png_tiles]
    write("corrupt.pmtiles", build(corrupt_tiles, tile_type=T_PNG, tile_comp=COMP_NONE, min_zoom=0, max_zoom=3,
                                   metadata={**raster_meta, "name": "omo-corrupt-raster"}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
