#!/usr/bin/env python3
"""M2 network smoke test (`SC-009`): field load, a 4-tile render, one readout and a composite PNG export.

Runs against the latest complete run: `map_load_field` over a small western-Europe window, a
4-tile `map_render_tiles`, a `map_inspect` at one grid point and an `export` of the composite PNG
with its `omo-export-sidecar/3`. A remote outage is reported as `skipped(network)` (exit 0) and is
never a pass: `status` is `passed` only when every step ran and every invariant held.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import omo_worker_client as owc  # noqa: E402

NETWORK_HINTS = ("RemoteUnavailable", "remote_unavailable", "CapabilityUnavailable", "timed out",
                 "Could not resolve", "Connection", "HTTP", "urlopen", "Name or service", "SSL")
BBOX = {"lat_min": 40.0, "lat_max": 58.0, "lon_spans": [[-8.0, 12.0]]}
TILES = [{"z": 4, "x": x, "y": y} for x in (7, 8) for y in (5, 6)]
PNG_MAGIC = b"\x89PNG\r\n\x1a\n"


def is_network(error: BaseException) -> bool:
    text = f"{type(error).__name__}: {error}".lower()
    return any(hint.lower() in text for hint in NETWORK_HINTS)


def export_body(target: Path, entry: dict, loaded: dict) -> dict:
    run = loaded["run"]
    return {
        "target": str(target),
        "format": "png",
        "overwrite_confirmed": False,
        "map_export": {
            "revision": 1,
            "field_id": loaded["field_id"],
            "viewport": {"center_lat": 49.0, "center_lon": 2.0, "zoom": 4, "width": 640, "height": 400},
            "stations": [{"id": "paris", "lat": 48.86, "lon": 2.35}],
            "catalog_entry": entry["id"],
            "catalog_version": "ci",
            "forecast": {
                "reference_time_utc": run["reference_time_utc"], "cycle": run["cycle"],
                "valid_time_utc": loaded["valid_time_utc"], "source_layout": loaded["source_layout"],
                "source_object": loaded["source_object"], "object_version": loaded.get("object_version"),
            },
            "color_scale": {"palette": "turbo", "min": loaded["stats"]["min"],
                            "max": loaded["stats"]["max"], "unit": loaded["stats"]["unit"], "locked": False},
            "layers": [
                {"id": "coastlines", "kind": "builtin_coastlines", "opacity": 1.0},
                {"id": "forecast", "kind": "forecast_field", "opacity": 0.78},
                {"id": "stations", "kind": "station_overlay", "opacity": 1.0},
            ],
            "attribution": list(entry.get("attribution", [])) + ["Natural Earth (public domain)"],
            "title": "Omobench CI smoke",
            "subtitle": f"Run {run['reference_time_utc']} · valid {loaded['valid_time_utc']}",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", type=Path, default=Path("target/debug/omo-worker"))
    parser.add_argument("--timeout", type=float, default=900.0)
    args = parser.parse_args()
    root = Path.cwd()
    entry = json.loads((root / "catalog/products.json").read_text())["entries"][0]
    report: dict = {"scenario": "ci_verify_map", "platform": owc.detect_platform()}
    client = None
    with tempfile.TemporaryDirectory(prefix="omo-ci-map-") as tmp:
        tmp_path = Path(tmp)
        try:
            artifact = owc.stage_artifact(root, tmp_path)
            client = owc.WorkerClient(args.worker.resolve(), owc.bootstrap_config(root, artifact, tmp_path))
            check = client.request("catalog_check", {"entry": entry}, timeout=args.timeout)
            readiness = check["readiness"]
            if readiness["state"] != "ready":
                if readiness.get("reason") == "remote_unavailable":
                    report["status"] = "skipped(network)"
                    print(json.dumps(report))
                    return 0
                report.update({"status": "failed", "detail": readiness})
                print(json.dumps(report))
                return 1
            loaded = client.request("map_load_field", {
                "field_id": "ci-map", "catalog_entry": entry, "catalog_version": "ci",
                "run": "latest_complete", "bbox": BBOX, "force_data_run": False,
            }, timeout=args.timeout)
            stats = loaded["stats"]
            assert stats["valid_count"] > 0, stats
            assert stats["min"] <= stats["max"], stats
            report.update({"run": loaded["run"]["reference_time_utc"], "layout": loaded["source_layout"],
                           "valid_count": stats["valid_count"]})

            rendered = client.request("map_render_tiles", {
                "result_id": "ci-tiles", "field_id": "ci-map", "revision": 1,
                "color_scale": {"palette": "turbo", "min": stats["min"], "max": stats["max"]},
                "layers": [{"layer_id": "forecast", "kind": "forecast_field"}],
                "tiles": TILES,
            }, timeout=args.timeout)
            assert not rendered["layer_errors"], rendered["layer_errors"]
            assert len(rendered["tiles"]) == len(TILES), rendered["tiles"]
            filled = [t for t in rendered["tiles"] if not t["empty"]]
            assert filled, "no tile carries forecast pixels"
            for tile in filled:
                size = Path(tile["rgba_path"]).stat().st_size
                assert size == tile["width"] * tile["height"] * 4, (tile, size)
            report["tiles_filled"] = len(filled)

            probe = client.request("map_inspect", {
                "field_id": "ci-map", "latitude": 48.86, "longitude": 2.35, "max_distance_m": 25000,
            }, timeout=args.timeout)
            sample = probe["sample"]
            assert sample["state"] == "value", sample
            assert stats["min"] - 1e-3 <= sample["value"] <= stats["max"] + 1e-3, (sample, stats)
            report["inspect"] = {"value": sample["value"], "unit": sample["unit"]}

            target = tmp_path / "map.png"
            request_id = client.send("export", export_body(target, entry, loaded))
            committed = client.wait("export_committed", request_id, timeout=args.timeout)
            png = target.read_bytes()
            assert png[:8] == PNG_MAGIC, "export is not a PNG"
            sidecar = json.loads(Path(committed["sidecar"]).read_text())
            assert sidecar["schema"] == "omo-export-sidecar/3", sidecar["schema"]
            assert sidecar["data"]["sha256"] == hashlib.sha256(png).hexdigest(), "sidecar hash mismatch"
            report.update({"status": "passed", "png_bytes": len(png)})
            print(json.dumps(report))
            return 0
        except (OSError, EOFError, TimeoutError, RuntimeError) as error:
            if is_network(error):
                report.update({"status": "skipped(network)", "detail": str(error)[:300]})
                print(json.dumps(report))
                return 0
            report.update({"status": "failed", "detail": str(error)[:500]})
            print(json.dumps(report))
            return 1
        except (AssertionError, KeyError) as error:
            report.update({"status": "failed", "detail": f"invariant violated: {error!r}"})
            print(json.dumps(report))
            return 1
        finally:
            if client is not None:
                client.close()


if __name__ == "__main__":
    sys.exit(main())
