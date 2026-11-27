#!/usr/bin/env python3
"""V7 / SC-007 fault injection for the M2 map against the real worker (default 10 trials each).

Scenarios:
  cancel   - interrupt a running `map_load_field` once it reports `reading`; expect `failed`/Cancelled
             and a worker that still answers `catalog_check` (the app keeps the prior map `stale`).
  kill9    - `omo-harness map-crash --at load`: SIGKILL the real worker under the real supervisor while it
             reads the field; expect WorkerCrashed, generation g -> g+1, `results/g<g>/` reaped with 0
             leaked `.rgba`/`.png`/`.f32bin` files, and a working new generation.
  kill9-render - the same, killing during a 96-tile `map_render_tiles`.
  offline  - bootstrap with `no_network = true`; expect `map_load_field` failed with
             CapabilityUnavailable(remote) while a local `.pmtiles` layer still renders.
  export-kill - `omo-harness export-kill --source map --at step1|step2|step3`; expect the startup scan
             verdicts incomplete / incomplete / finished.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import omo_worker_client as owc  # noqa: E402
from omo_worker_client import station_bbox  # noqa: E402

ARCHIVE = "fixtures/pmtiles/sample-raster.pmtiles"


def load_body(entry: dict, bbox: dict, tag: str) -> dict:
    return {"field_id": tag, "catalog_entry": entry, "catalog_version": "fault", "run": "latest_complete",
            "bbox": bbox, "force_data_run": False}


def wait_stage(client, request_id: int, stage: str, timeout: float = 300.0) -> None:
    deadline = time.monotonic() + timeout
    while True:
        item = client.event(deadline)
        if item["request_id"] != request_id:
            continue
        if item["kind"] == "progress" and item["body"].get("stage") == stage:
            return
        if item["kind"] in ("result", "failed"):
            raise RuntimeError(f"request ended before `{stage}`: {item['kind']} {json.dumps(item['body'])[:300]}")


def terminal(client, request_id: int, timeout: float = 300.0) -> dict:
    deadline = time.monotonic() + timeout
    while True:
        item = client.event(deadline)
        if item["request_id"] == request_id and item["kind"] in ("result", "failed"):
            return item


def render_body(root: Path, tag: str) -> dict:
    return {"result_id": tag, "revision": 1, "color_scale": {"palette": "turbo", "min": 0.0, "max": 1.0},
            "layers": [{"layer_id": "archive", "kind": "local_pmtiles", "path": str(root / ARCHIVE)}],
            "tiles": [{"z": 2, "x": 1, "y": 1}, {"z": 2, "x": 2, "y": 1}]}


def run_harness(args: list[str]) -> tuple[int, list[dict], str]:
    proc = subprocess.run(args, capture_output=True, text=True)
    lines = []
    for line in proc.stdout.splitlines():
        try:
            lines.append(json.loads(line))
        except json.JSONDecodeError:
            pass
    return proc.returncode, lines, proc.stdout


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--trials", type=int, default=10)
    parser.add_argument("--worker", type=Path, default=Path("target/debug/omo-worker"))
    parser.add_argument("--harness", type=Path, default=Path("target/debug/omo-harness"))
    parser.add_argument("--stations", type=Path, default=Path("fixtures/stations/m2-regional-100.csv"))
    parser.add_argument("--scenarios", default="cancel,kill9,kill9-render,offline,export-kill")
    args = parser.parse_args()
    root = Path.cwd()
    entry = json.loads((root / "catalog/products.json").read_text())["entries"][0]
    bbox = station_bbox(args.stations, 1.0)
    worker = args.worker.resolve()
    results: dict[str, dict] = {}
    with tempfile.TemporaryDirectory(prefix="omo-fault-map-") as tmp:
        tmp_path = Path(tmp)
        artifact = owc.stage_artifact(root, tmp_path)

        def config(sub: str, no_network: bool = False) -> dict:
            work = tmp_path / sub
            work.mkdir(parents=True, exist_ok=True)
            return owc.bootstrap_config(root, artifact, work, no_network=no_network)

        for scenario in args.scenarios.split(","):
            outcomes: list[dict] = []
            if scenario == "cancel":
                client = owc.WorkerClient(worker, config("cancel"))
                try:
                    for trial in range(args.trials):
                        started = time.monotonic()
                        rid = client.send("map_load_field", load_body(entry, bbox, f"cancel-{trial}"))
                        wait_stage(client, rid, "reading")
                        client.send("interrupt", {"target_request_id": rid})
                        end = terminal(client, rid)
                        detail = json.dumps(end["body"])[:200]
                        cancelled = end["kind"] == "failed" and "ancel" in detail
                        check = client.request("catalog_check", {"entry": entry}, timeout=90)
                        ok = cancelled and check["readiness"]["state"] == "ready"
                        outcomes.append({"trial": trial, "ok": ok, "elapsed_s": round(time.monotonic() - started, 1),
                                         "detail": "Cancelled; worker still ready" if ok else detail})
                        print(json.dumps({"scenario": "cancel", **outcomes[-1]}), flush=True)
                finally:
                    client.close()
            elif scenario in ("kill9", "kill9-render"):
                cfg = tmp_path / f"{scenario}-config.json"
                cfg.write_text(json.dumps(config(scenario)))
                body = tmp_path / f"{scenario}-load.json"
                body.write_text(json.dumps(load_body(entry, bbox, "kill")))
                code, lines, _ = run_harness([
                    str(args.harness.resolve()), "map-crash", "--worker", str(worker), "--config", str(cfg),
                    "--load-body", str(body), "--archive", str(root / ARCHIVE),
                    "--at", "load" if scenario == "kill9" else "render", "--trials", str(args.trials)])
                for line in lines:
                    print(json.dumps(line), flush=True)
                outcomes = [line for line in lines if "trial" in line]
            elif scenario == "offline":
                for trial in range(args.trials):
                    client = owc.WorkerClient(worker, config(f"offline-{trial}", no_network=True))
                    try:
                        rid = client.send("map_load_field", load_body(entry, bbox, f"offline-{trial}"))
                        end = terminal(client, rid, timeout=120)
                        detail = json.dumps(end["body"])[:200]
                        capability = end["kind"] == "failed" and "emote" in detail
                        local = client.request("map_render_tiles", render_body(root, f"offline-{trial}"), timeout=60)
                        rendered = bool(local["tiles"]) and not local["layer_errors"]
                        ok = capability and rendered
                        outcomes.append({"trial": trial, "ok": ok,
                                         "detail": "CapabilityUnavailable(remote); local archive still renders"
                                         if ok else detail})
                        print(json.dumps({"scenario": "offline", **outcomes[-1]}), flush=True)
                    finally:
                        client.close()
            elif scenario == "export-kill":
                expected = {"step1": ("incomplete", False), "step2": ("incomplete", False),
                            "step3": ("finished", True)}
                for step, (recovery, saved) in expected.items():
                    for trial in range(args.trials):
                        code, lines, raw = run_harness([str(args.harness.resolve()), "export-kill", "--at",
                                                        step, "--source", "map"])
                        report = lines[-1] if lines else {}
                        if not lines:  # `export-kill` pretty-prints one JSON document
                            try:
                                report = json.loads(raw)
                            except json.JSONDecodeError:
                                report = {}
                        ok = (code == 0 and report.get("recovery") == recovery and report.get("saved") is saved
                              and report.get("source") == "map")
                        outcomes.append({"step": step, "trial": trial, "ok": ok,
                                         "detail": report if not ok else f"{recovery}/saved={saved}"})
                    print(json.dumps({"scenario": f"export-kill-{step}",
                                      "passed": sum(1 for o in outcomes if o["step"] == step and o["ok"]),
                                      "trials": args.trials}), flush=True)
            else:
                print(f"unknown scenario {scenario}", file=sys.stderr)
                return 2
            passed = sum(1 for o in outcomes if o.get("ok"))
            results[scenario] = {"trials": len(outcomes), "passed": passed, "outcomes": outcomes}
            print(json.dumps({"scenario": scenario, "trials": len(outcomes), "passed": passed}), flush=True)
    print(json.dumps({k: {"trials": v["trials"], "passed": v["passed"]} for k, v in results.items()}, indent=2))
    return 0 if results and all(r["trials"] >= args.trials and r["passed"] == r["trials"]
                                for r in results.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
