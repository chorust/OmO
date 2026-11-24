#!/usr/bin/env python3
"""Official `omfiles` O1280 value oracle for SC-003 (research R-09)."""

import argparse
import csv
import json
import urllib.request
from pathlib import Path

import omfiles


class RangeHttpFs:
    """Just the `size` / `cat_file` surface omfiles' sync fsspec reader needs, over HTTP ranges."""

    def __init__(self):
        self._sizes: dict[str, int] = {}

    def size(self, path: str) -> int:
        if path not in self._sizes:
            req = urllib.request.Request(path, method="HEAD")
            with urllib.request.urlopen(req, timeout=60) as resp:
                self._sizes[path] = int(resp.headers["Content-Length"])
        return self._sizes[path]

    def cat_file(self, path: str, start: int | None = None, end: int | None = None, **_kwargs) -> bytes:
        headers = {}
        if start is not None or end is not None:
            lo = 0 if start is None else start
            headers["Range"] = f"bytes={lo}-{'' if end is None else end - 1}"
        last = None
        for attempt in range(4):
            try:
                with urllib.request.urlopen(urllib.request.Request(path, headers=headers), timeout=60) as resp:
                    return resp.read()
            except Exception as error:  # noqa: BLE001
                last = error
        raise last


def fetch_meta(run: str) -> tuple[dict, str]:
    yyyy, mm, dd, hh = run[0:4], run[5:7], run[8:10], run[11:13]
    url = f"https://openmeteo.s3.amazonaws.com/data_run/ecmwf_ifs/{yyyy}/{mm}/{dd}/{hh}00Z/meta.json"
    with urllib.request.urlopen(url, timeout=60) as resp:
        return json.load(resp), resp.headers.get("ETag", "")


def head_object_etag(run: str) -> tuple[str, str]:
    yyyy, mm, dd, hh = run[0:4], run[5:7], run[8:10], run[11:13]
    url = f"https://openmeteo.s3.amazonaws.com/data_run/ecmwf_ifs/{yyyy}/{mm}/{dd}/{hh}00Z/temperature_2m.om"
    req = urllib.request.Request(url, method="HEAD")
    with urllib.request.urlopen(req, timeout=60) as resp:
        return url, resp.headers.get("ETag", "")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--run", required=True, help="YYYY-MM-DDTHHZ")
    parser.add_argument("--nearest", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    meta, _ = fetch_meta(args.run)
    valid_times = meta["valid_times"]
    url, etag = head_object_etag(args.run)

    with args.nearest.open("r", encoding="utf-8", newline="") as handle:
        stations = [(r["station_id"], int(r["flat_index"])) for r in csv.DictReader(handle)]

    reader = omfiles.OmFileReader.from_fsspec(RangeHttpFs(), url)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8", newline="") as handle:
        handle.write(f"# run={args.run} object={url} etag={etag}\n")
        writer = csv.writer(handle, lineterminator="\n")
        writer.writerow(["station_id", "valid_time", "value"])
        for sid, flat in stations:
            arr = reader[:, flat : flat + 1, :]
            vals = arr.reshape(-1)
            for t_idx, vt in enumerate(valid_times):
                val = vals[t_idx]
                s_val = "" if (val is None or (isinstance(val, float) and val != val)) else f"{float(val):.6f}"
                writer.writerow([sid, vt, s_val])


if __name__ == "__main__":
    main()
