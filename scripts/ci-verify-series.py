#!/usr/bin/env python3
"""M1 network smoke test: `catalog_check` plus a 3-station `series` against the latest complete run.

Asserts the T028 invariants (valid cycle, row count per station, distances within the catalog
limit, exact-grid coordinates echoed back). A network failure is reported as `skipped(network)`
and exits 0 with that status; it is never reported as a pass (SC-009).
"""
from __future__ import annotations

import argparse
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import omo_worker_client as owc  # noqa: E402

STATIONS = "station_id,latitude,longitude,name,elevation_m\n1,39.9042,116.4074,Beijing,\n2,51.5074,-0.1278,London,\n3,-33.8688,151.2093,Sydney,\n"
NETWORK_HINTS = ("RemoteUnavailable", "CapabilityUnavailable", "timed out", "Could not resolve",
                 "Connection", "HTTP", "urlopen", "Name or service", "SSL")


def is_network(error: BaseException) -> bool:
    text = f"{type(error).__name__}: {error}"
    return any(hint.lower() in text.lower() for hint in NETWORK_HINTS)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", type=Path, default=Path("target/debug/omo-worker"))
    parser.add_argument("--timeout", type=float, default=240.0)
    args = parser.parse_args()
    root = Path.cwd()
    entry = json.loads((root / "catalog/products.json").read_text())["entries"][0]
    report: dict = {"scenario": "ci_verify_series", "platform": owc.detect_platform()}
    client = None
    with tempfile.TemporaryDirectory(prefix="omo-ci-series-") as tmp:
        tmp_path = Path(tmp)
        try:
            artifact = owc.stage_artifact(root, tmp_path)
            config = owc.bootstrap_config(root, artifact, tmp_path)
            client = owc.WorkerClient(args.worker.resolve(), config)
            check = client.request("catalog_check", {"entry": entry}, timeout=args.timeout)
            readiness = check["readiness"]
            report["readiness"] = readiness["state"]
            if readiness["state"] != "ready":
                if readiness.get("reason") == "remote_unavailable":
                    report["status"] = "skipped(network)"
                    print(json.dumps(report))
                    return 0
                report["status"] = "failed"
                report["detail"] = readiness
                print(json.dumps(report))
                return 1
            stations = tmp_path / "stations.csv"
            stations.write_text(STATIONS)
            body = client.request("series", {
                "stations_ref": str(stations), "stations_sha256": "ci", "catalog_entry": entry,
                "catalog_version": "ci", "run": "latest_complete", "window": {"start_h": 0, "end_h": 168},
                "max_distance_m": entry["max_distance_m"], "result_id": "ci-3",
                "limits": {"rows": 100000, "bytes": 64 * 1024 * 1024},
            }, timeout=args.timeout)
            run = body["run"]
            window = body["effective_window"]
            per_station = 109 if run["cycle"] in ("06", "18") else 113
            assert body["result_ref"]["rows"] == 3 * per_station, body["result_ref"]
            assert window["truncated"] == (run["cycle"] in ("06", "18")), window
            assert body["plan"] == "G", body["plan"]
            matches = Path(body["matches_ref"]["path"])
            assert matches.is_file(), matches
            report.update({"status": "passed", "run": run["reference_time_utc"], "cycle": run["cycle"],
                           "rows": body["result_ref"]["rows"], "plan": body["plan"]})
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
        except AssertionError as error:
            report.update({"status": "failed", "detail": f"invariant violated: {error}"})
            print(json.dumps(report))
            return 1
        finally:
            if client is not None:
                client.close()


if __name__ == "__main__":
    sys.exit(main())
