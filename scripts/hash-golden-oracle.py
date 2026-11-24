#!/usr/bin/env python3
"""Hash the first 64 ordered float32 values from the official OM oracle CSV."""

import csv
import hashlib
import json
import math
import struct
import sys
from pathlib import Path

LIMIT = 64


def decode_value(raw: str) -> float | None:
    value = raw.strip()
    if value == "" or value.lower() == "null":
        return None
    return float(value)


def sort_key(value: float | None) -> tuple[int, float]:
    if value is None:
        return (0, 0.0)
    if math.isnan(value):
        # DuckDB sorts NaN after finite numbers in ascending order.
        return (1, math.inf)
    return (1, value)


def main() -> int:
    if len(sys.argv) != 2:
        print(f"Usage: {Path(sys.argv[0]).name} <official-oracle.csv>", file=sys.stderr)
        return 2

    path = Path(sys.argv[1])
    with path.open(newline="", encoding="utf-8") as source:
        rows = list(csv.DictReader(source))
    if not rows or "value" not in rows[0]:
        raise SystemExit("oracle CSV must contain a value column and at least one row")

    values = [decode_value(row["value"]) for row in rows]
    values.sort(key=sort_key)
    selected = values[:LIMIT]
    if len(selected) != LIMIT:
        raise SystemExit(f"oracle CSV has only {len(selected)} rows; expected at least {LIMIT}")

    digest = hashlib.sha256()
    for value in selected:
        if value is None:
            digest.update(b"\x00")
        else:
            digest.update(b"\x01")
            digest.update(struct.pack(">f", value))

    print(
        json.dumps(
            {
                "query_id": "golden-v1",
                "source_csv": str(path),
                "source_rows": len(rows),
                "hashed_rows": len(selected),
                "encoding": "null=0x00; float32=0x01 || IEEE-754 big-endian",
                "result_sha256": digest.hexdigest(),
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
