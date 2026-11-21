#!/usr/bin/env python3
"""Verify the retained, approved Duckling source snapshot against its manifest."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "snapshot",
        nargs="?",
        type=Path,
        default=Path(__file__).resolve().parents[1]
        / "vendor-provenance/snapshots/duckling-8ec3bec-20261116",
    )
    args = parser.parse_args()
    root = args.snapshot.resolve()
    manifest_path = root / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"cannot read snapshot manifest {manifest_path}: {error}", file=sys.stderr)
        return 2

    if manifest.get("schema") != "duckling-source-snapshot/1":
        print("unsupported snapshot manifest schema", file=sys.stderr)
        return 2

    entries = manifest.get("files")
    if not isinstance(entries, list) or not entries:
        print("snapshot manifest has no file entries", file=sys.stderr)
        return 2

    errors: list[str] = []
    seen: set[str] = set()
    for entry in entries:
        relative = entry.get("snapshot_path")
        if not isinstance(relative, str) or relative in seen:
            errors.append(f"invalid or duplicate snapshot_path: {relative!r}")
            continue
        seen.add(relative)
        path = (root / relative).resolve()
        if not path.is_relative_to(root):
            errors.append(f"path escapes snapshot directory: {relative}")
            continue
        try:
            data = path.read_bytes()
        except OSError as error:
            errors.append(f"{relative}: {error}")
            continue
        actual_hash = hashlib.sha256(data).hexdigest()
        if len(data) != entry.get("bytes"):
            errors.append(f"{relative}: size {len(data)} != {entry.get('bytes')}")
        if actual_hash != entry.get("sha256"):
            errors.append(f"{relative}: sha256 {actual_hash} != {entry.get('sha256')}")

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        print(f"FAILED: {len(errors)} issue(s) across {len(entries)} entries", file=sys.stderr)
        return 1

    print(f"verified {len(entries)} snapshot files; all sizes and SHA-256 values match")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
