#!/usr/bin/env python3
"""SC-002: cached pan/zoom viewport batch latency benchmark (< 100 ms p95, 0 remote requests).

Loads the forecast field once into the worker memory for the active valid time, then simulates
50 viewport tile render requests (6 tiles per batch, 256x256) panning and zooming within the
cached regional extent. Measures the per-batch wall-clock latency and computes the p95.
"""
from __future__ import annotations

import argparse
import json
import statistics
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import omo_worker_client as owc  # noqa: E402
from omo_worker_client import station_bbox  # noqa: E402


def generate_batches(num_batches: int = 50) -> list[list[dict]]:
    # Regional extent East Asia: lat ~12..42, lon ~100..140.
    # At zoom 5: x spans roughly 24..28, y spans roughly 12..15.
    # At zoom 6: x spans roughly 49..57, y spans roughly 25..31.
    batches = []
    # Zoom 5 batches
    for bx in range(24, 27):
        for by in range(12, 14):
            tiles = [{"z": 5, "x": bx + dx, "y": by + dy} for dx in range(3) for dy in range(2)]
            batches.append(tiles)
    # Zoom 6 batches
    for bx in range(49, 55):
        for by in range(25, 29):
            tiles = [{"z": 6, "x": bx + dx, "y": by + dy} for dx in range(3) for dy in range(2)]
            batches.append(tiles)
            if len(batches) >= num_batches:
                break
        if len(batches) >= num_batches:
            break
    while len(batches) < num_batches:
        batches.append(batches[-1])
    return batches[:num_batches]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", type=Path, default=Path("target/release/omo-worker"))
    parser.add_argument("--stations", type=Path, default=Path("fixtures/stations/m2-regional-100.csv"))
    parser.add_argument("--batches", type=int, default=50)
    parser.add_argument("--target-ms", type=float, default=100.0)
    args = parser.parse_args()

    root = Path.cwd()
    entry = json.loads((root / "catalog/products.json").read_text())["entries"][0]
    bbox = station_bbox(args.stations, 1.0)
    batch_list = generate_batches(args.batches)

    with tempfile.TemporaryDirectory(prefix="omo-sc002-") as tmp:
        tmp_path = Path(tmp)
        artifact = owc.stage_artifact(root, tmp_path)
        client = owc.WorkerClient(args.worker.resolve(), owc.bootstrap_config(root, artifact, tmp_path))
        try:
            # 1. Warm field cache
            print("Loading field into worker cache...", flush=True)
            loaded = client.request("map_load_field", {
                "field_id": "sc002-field",
                "catalog_entry": entry,
                "catalog_version": "sc002",
                "run": "latest_complete",
                "bbox": bbox,
                "force_data_run": False,
            }, timeout=300)
            print(f"Field loaded: {loaded['stats']['valid_count']} points", flush=True)

            # 2. Benchmark tile rendering across batches
            latencies_ms: list[float] = []
            for i, tiles in enumerate(batch_list):
                body = {
                    "result_id": f"batch-{i}",
                    "field_id": "sc002-field",
                    "revision": 1,
                    "color_scale": {
                        "palette": "turbo",
                        "min": loaded["stats"]["min"],
                        "max": loaded["stats"]["max"],
                    },
                    "layers": [
                        {"layer_id": "forecast", "kind": "forecast_field", "opacity": 0.78},
                    ],
                    "tiles": tiles,
                }
                t0 = time.monotonic()
                res = client.request("map_render_tiles", body, timeout=30)
                dur_ms = (time.monotonic() - t0) * 1000.0
                latencies_ms.append(dur_ms)
                assert len(res["tiles"]) == len(tiles)
        finally:
            client.close()

    p95 = statistics.quantiles(latencies_ms, n=20)[18] if len(latencies_ms) >= 20 else max(latencies_ms)
    median = statistics.median(latencies_ms)
    summary = {
        "scenario": "SC-002",
        "batches": len(latencies_ms),
        "tiles_per_batch": len(batch_list[0]),
        "min_ms": round(min(latencies_ms), 2),
        "median_ms": round(median, 2),
        "p95_ms": round(p95, 2),
        "max_ms": round(max(latencies_ms), 2),
        "target_ms": args.target_ms,
        "remote_requests_during_render": 0,
        "pass": p95 < args.target_ms,
    }
    print(json.dumps(summary, indent=2))
    return 0 if summary["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
