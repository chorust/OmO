#!/usr/bin/env python3
"""V7 fault injection against the real worker (SC-005): cancel, kill -9, and network-unavailable trials.

Scenarios (each N trials, default 10):
  cancel   - interrupt a running `series` once it reports `reading`; expect `failed`/Cancelled and a worker
             that still answers `catalog_check`.
  kill9    - SIGKILL the worker while it reads; expect EOF (the supervisor reports WorkerCrashed), then a
             fresh worker (generation + 1) completes a retry.
  offline  - bootstrap with `no_network = true`; expect `catalog_check` NotReady(RemoteUnavailable) and
             `series` failed with CapabilityUnavailable(remote).
Network-down-during-reading needs an interposed network cut and is not covered here.
"""
from __future__ import annotations

import argparse
import json
import os
import signal
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import omo_worker_client as owc  # noqa: E402


def series_body(entry, stations: Path, count_note: str, result_id: str) -> dict:
    return {"stations_ref": str(stations), "stations_sha256": "fault", "catalog_entry": entry,
            "catalog_version": "fault", "run": "latest_complete", "window": {"start_h": 0, "end_h": 168},
            "max_distance_m": entry["max_distance_m"], "result_id": result_id,
            "limits": {"rows": 2_000_000, "bytes": 256 * 1024 * 1024}}


def wait_stage(client, request_id: int, stage: str, timeout: float = 120.0) -> None:
    deadline = time.monotonic() + timeout
    while True:
        item = client.event(deadline)
        if item["request_id"] != request_id:
            continue
        if item["kind"] == "progress" and item["body"].get("stage") == stage:
            return
        if item["kind"] in ("result", "failed"):
            raise RuntimeError(f"request ended before `{stage}`: {item['kind']} {json.dumps(item['body'])[:300]}")


def terminal(client, request_id: int, timeout: float = 120.0) -> dict:
    deadline = time.monotonic() + timeout
    while True:
        item = client.event(deadline)
        if item["request_id"] == request_id and item["kind"] in ("result", "failed"):
            return item


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--trials", type=int, default=10)
    parser.add_argument("--worker", type=Path, default=Path("target/debug/omo-worker"))
    parser.add_argument("--stations", type=Path, default=Path("fixtures/stations/m1-100.csv"))
    parser.add_argument("--scenarios", default="cancel,kill9,offline")
    args = parser.parse_args()
    root = Path.cwd()
    entry = json.loads((root / "catalog/products.json").read_text())["entries"][0]
    worker = args.worker.resolve()
    results: dict[str, dict] = {}
    with tempfile.TemporaryDirectory(prefix="omo-fault-") as tmp:
        tmp_path = Path(tmp)
        artifact = owc.stage_artifact(root, tmp_path)
        stations = tmp_path / "stations.csv"
        import csv
        with args.stations.open(encoding="utf-8-sig", newline="") as src, stations.open("w", newline="") as dst:
            rows = list(csv.DictReader(src))
            writer = csv.writer(dst, lineterminator="\n")
            writer.writerow(["station_id", "latitude", "longitude", "name", "elevation_m"])
            for row in rows:
                writer.writerow([row["id"], row["lat"], row["lon"], row.get("name", ""), ""])
        small = tmp_path / "small.csv"
        small.write_text("".join(
            ["station_id,latitude,longitude,name,elevation_m\n"] +
            [f"{r['id']},{r['lat']},{r['lon']},{r.get('name','')},\n" for r in rows[:3]]))

        def new_client(sub: str, generation: int = 1, no_network: bool = False):
            work = tmp_path / sub
            work.mkdir(parents=True, exist_ok=True)
            return owc.WorkerClient(worker, owc.bootstrap_config(root, artifact, work, no_network=no_network),
                                    generation=generation)

        for scenario in args.scenarios.split(","):
            outcomes = []
            if scenario == "cancel":
                client = new_client("cancel")
                try:
                    for trial in range(args.trials):
                        started = time.monotonic()
                        rid = client.send("series", series_body(entry, stations, "", f"cancel-{trial}"))
                        wait_stage(client, rid, "reading")
                        client.send("interrupt", {"target_request_id": rid})
                        end = terminal(client, rid)
                        failed = end["kind"] == "failed"
                        detail = json.dumps(end["body"])[:200]
                        cancelled = failed and "ancel" in detail
                        check = client.request("catalog_check", {"entry": entry}, timeout=90)
                        ok = cancelled and check["readiness"]["state"] == "ready"
                        outcomes.append({"trial": trial, "ok": ok, "elapsed_s": round(time.monotonic() - started, 1),
                                         "detail": detail if not ok else "Cancelled; worker still ready"})
                        print(json.dumps({"scenario": "cancel", **outcomes[-1]}), flush=True)
                finally:
                    client.close()
            elif scenario == "kill9":
                for trial in range(args.trials):
                    started = time.monotonic()
                    client = new_client(f"kill-{trial}", generation=1)
                    try:
                        rid = client.send("series", series_body(entry, stations, "", f"kill-{trial}"))
                        try:
                            wait_stage(client, rid, "reading")
                        except RuntimeError as error:
                            outcomes.append({"trial": trial, "ok": False, "detail": f"setup failed: {error}"})
                            print(json.dumps({"scenario": "kill9", **outcomes[-1]}), flush=True)
                            continue
                        os.kill(client.proc.pid, signal.SIGKILL)
                        try:
                            terminal(client, rid, timeout=30)
                            died = False
                        except (EOFError, TimeoutError):
                            died = client.proc.wait(timeout=10) == -signal.SIGKILL
                    finally:
                        client.proc.kill()
                    retry = new_client(f"retry-{trial}", generation=2)
                    try:
                        body = retry.request("series", series_body(entry, small, "", f"retry-{trial}"), timeout=180)
                        ok = died and body["result_ref"]["rows"] > 0
                    finally:
                        retry.close()
                    outcomes.append({"trial": trial, "ok": ok, "elapsed_s": round(time.monotonic() - started, 1),
                                     "detail": "worker killed (EOF); generation 2 retry returned rows" if ok else "failed"})
                    print(json.dumps({"scenario": "kill9", **outcomes[-1]}), flush=True)
            elif scenario == "offline":
                for trial in range(args.trials):
                    client = new_client(f"offline-{trial}", no_network=True)
                    try:
                        check = client.request("catalog_check", {"entry": entry}, timeout=60)
                        readiness = check["readiness"]
                        not_ready = readiness["state"] == "not_ready" and readiness.get("reason") == "remote_unavailable"
                        rid = client.send("series", series_body(entry, small, "", f"offline-{trial}"))
                        end = terminal(client, rid, timeout=60)
                        detail = json.dumps(end["body"])[:200]
                        capability = end["kind"] == "failed" and "emote" in detail
                        outcomes.append({"trial": trial, "ok": not_ready and capability,
                                         "detail": detail if not (not_ready and capability) else "NotReady(RemoteUnavailable); series CapabilityUnavailable(remote)"})
                    finally:
                        client.close()
            passed = sum(1 for o in outcomes if o["ok"])
            results[scenario] = {"trials": len(outcomes), "passed": passed, "outcomes": outcomes}
            print(json.dumps({"scenario": scenario, "trials": len(outcomes), "passed": passed}), flush=True)
    print(json.dumps(results, indent=2))
    return 0 if all(r["passed"] == r["trials"] for r in results.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
