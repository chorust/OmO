#!/usr/bin/env python3
"""Validate and summarize omo-main-thread-probe JSONL captures."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(2)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path, help="JSONL log written by omo-app")
    parser.add_argument(
        "--min-coverage-seconds",
        type=float,
        default=0,
        help="require at least this much main-thread callback coverage",
    )
    parser.add_argument(
        "--require-stall-ms",
        type=float,
        help="require an observed callback gap at least this long",
    )
    args = parser.parse_args()

    try:
        records = [json.loads(line) for line in args.log.read_text().splitlines() if line.strip()]
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read valid JSONL from {args.log}: {error}")

    if not records or records[0].get("type") != "header":
        fail("missing header record")
    header = records[0]
    schema = header.get("schema")
    if schema not in {"omo-main-thread-probe/1", "omo-main-thread-probe/2"}:
        fail(f"unsupported schema: {schema!r}")
    has_wall_clock = schema == "omo-main-thread-probe/2"
    if header.get("expected_interval_us") != 25_000:
        fail("expected callback interval is not 25,000 microseconds")
    if header.get("glib_source_priority") != "default (0)":
        fail("missing or unexpected GLib timeout priority")

    snapshots = [record for record in records[1:] if record.get("type") == "snapshot"]
    if not snapshots:
        fail("no snapshots recorded")
    thread_ids = {record.get("main_thread_id") for record in records if record.get("type") == "header"}
    if len(thread_ids) != 1:
        fail("capture contains conflicting main-thread identities")
    if any(not record.get("callback_on_main_thread") for record in snapshots):
        fail("the timeout callback did not consistently run on its installing thread")

    previous_samples = -1
    previous_elapsed = -1
    previous_max_gap = 0
    previous_max_wall_gap = 0
    previous_snapshot_wall_ms = int(header.get("wall_start_unix_ms", 0))
    wall_clock_regressions = 0
    for index, record in enumerate(snapshots, start=1):
        samples = int(record["samples"])
        elapsed = int(record["callback_elapsed_us"])
        max_gap = int(record["max_callback_gap_us"])
        max_wall_gap = int(record.get("max_callback_wall_gap_us", 0))
        if samples < previous_samples or elapsed < previous_elapsed:
            fail(f"snapshot {index} moved a monotonic counter backwards")
        if max_gap < previous_max_gap or max_wall_gap < previous_max_wall_gap:
            fail(f"snapshot {index} moved a maximum callback gap backwards")
        if has_wall_clock:
            snapshot_wall_ms = int(record["snapshot_wall_unix_ms"])
            if snapshot_wall_ms < previous_snapshot_wall_ms:
                wall_clock_regressions += 1
            previous_snapshot_wall_ms = snapshot_wall_ms
        previous_samples = samples
        previous_elapsed = elapsed
        previous_max_gap = max_gap
        previous_max_wall_gap = max_wall_gap

    last = snapshots[-1]
    coverage_us = int(last["callback_elapsed_us"])
    wall_coverage_us = int(last.get("callback_wall_elapsed_us", 0)) if has_wall_clock else None
    max_gap_us = int(last["max_callback_gap_us"])
    max_wall_gap_us = int(last.get("max_callback_wall_gap_us", 0)) if has_wall_clock else None
    max_ongoing_gap_us = max(int(record["ongoing_gap_us"]) for record in snapshots)
    max_ongoing_wall_gap_us = (
        max(int(record["ongoing_wall_gap_us"]) for record in snapshots) if has_wall_clock else None
    )
    min_coverage_us = int(args.min_coverage_seconds * 1_000_000)
    if last.get("termination") != "stopped":
        fail(f"capture did not terminate cleanly: {last.get('termination')!r}")
    if coverage_us < min_coverage_us:
        fail(
            f"monotonic callback coverage was {coverage_us / 1_000_000:.3f}s; "
            f"required {args.min_coverage_seconds:.3f}s"
        )
    if has_wall_clock and wall_coverage_us is not None and wall_coverage_us < min_coverage_us:
        fail(
            f"wall-clock callback coverage was {wall_coverage_us / 1_000_000:.3f}s; "
            f"required {args.min_coverage_seconds:.3f}s"
        )
    if args.require_stall_ms is not None:
        required_stall_us = args.require_stall_ms * 1_000
        if max_gap_us < required_stall_us:
            fail(
                f"maximum monotonic callback gap was {max_gap_us / 1_000:.3f}ms; "
                f"required at least {args.require_stall_ms:.3f}ms"
            )
        if has_wall_clock and max_wall_gap_us is not None and max_wall_gap_us < required_stall_us:
            fail(
                f"maximum wall-clock callback gap was {max_wall_gap_us / 1_000:.3f}ms; "
                f"required at least {args.require_stall_ms:.3f}ms"
            )

    summary: dict[str, Any] = {
        "schema": header["schema"],
        "expected_interval_ms": header["expected_interval_us"] / 1_000,
        "main_thread_id": header.get("main_thread_id"),
        "glib_source_priority": header["glib_source_priority"],
        "samples": int(last["samples"]),
        "coverage_seconds_monotonic": coverage_us / 1_000_000,
        "max_callback_gap_ms_monotonic": max_gap_us / 1_000,
        "max_callback_overrun_ms_monotonic": int(last["max_callback_overrun_us"]) / 1_000,
        "max_ongoing_gap_ms_monotonic": max_ongoing_gap_us / 1_000,
        "callback_on_main_thread": last["callback_on_main_thread"],
        "termination": last["termination"],
        "snapshots": len(snapshots),
    }
    if has_wall_clock and wall_coverage_us is not None and max_wall_gap_us is not None:
        summary.update({
            "coverage_seconds_wall_clock": wall_coverage_us / 1_000_000,
            "monotonic_wall_coverage_delta_seconds": (wall_coverage_us - coverage_us) / 1_000_000,
            "wall_to_monotonic_rate_ratio": wall_coverage_us / coverage_us if coverage_us else None,
            "max_callback_gap_ms_wall_clock": max_wall_gap_us / 1_000,
            "max_callback_overrun_ms_wall_clock": int(last["max_callback_wall_overrun_us"]) / 1_000,
            "max_ongoing_gap_ms_wall_clock": (max_ongoing_wall_gap_us or 0) / 1_000,
            "snapshot_wall_clock_regressions": wall_clock_regressions,
        })
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
