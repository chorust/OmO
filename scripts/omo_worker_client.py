#!/usr/bin/env python3
"""Minimal omo-worker protocol v1 client shared by the M1 verification scripts."""
from __future__ import annotations

import csv
import json
import os
import platform
import selectors
import struct
import subprocess
import tempfile
import time
from pathlib import Path


def detect_platform() -> str:
    system, machine = platform.system(), platform.machine().lower()
    if system == "Linux" and machine in ("aarch64", "arm64"):
        return "linux_arm64"
    if system == "Linux" and machine in ("x86_64", "amd64"):
        return "linux_amd64"
    if system == "Darwin" and machine in ("aarch64", "arm64"):
        return "osx_arm64"
    raise SystemExit(f"unsupported host: {system}/{machine}")


def bootstrap_config(root: Path, artifact_path: Path, workdir: Path, *, no_network: bool = False,
                     channel: str = "github-release-unsigned") -> dict:
    lock = json.loads((root / "artifacts/duckomo.lock.json").read_text())
    host = detect_platform()
    entry = next(e for e in lock["entries"] if e["channel"] == channel and e["platform"] == host)
    data_dir = workdir / "data"
    ext_cache = workdir / "ext-cache"
    data_dir.mkdir(parents=True, exist_ok=True)
    ext_cache.mkdir(mode=0o700, parents=True, exist_ok=True)
    return {
        "data_dir": str(data_dir),
        "ext_cache_dir": str(ext_cache),
        "artifact_path": str(artifact_path),
        "fixture_path": str((root / entry["golden"]["fixture"]).resolve()),
        "artifact": entry,
        "platform": host,
        "threads": 4,
        "no_network": no_network,
    }


class WorkerClient:
    def __init__(self, worker: Path, config: dict, generation: int = 1):
        self.generation = generation
        self.config = config
        self.proc = subprocess.Popen([str(worker)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, bufsize=0)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.proc.stdout, selectors.EVENT_READ)
        self._next = 1
        self.send("hello", {"bootstrap_config": config})
        self.wait("ready", 1, timeout=120)

    def send(self, kind: str, body: dict, request_id: int | None = None) -> int:
        if request_id is None:
            request_id = self._next
            self._next += 1
        else:
            self._next = max(self._next, request_id + 1)
        data = json.dumps({"v": 1, "worker_generation": self.generation, "job_id": 1,
                           "request_id": request_id, "kind": kind, "body": body}).encode()
        self.proc.stdin.write(struct.pack("<I", len(data)) + data)
        self.proc.stdin.flush()
        return request_id

    def _read_exact(self, size: int) -> bytes:
        out = b""
        while len(out) < size:
            chunk = os.read(self.proc.stdout.fileno(), size - len(out))
            if not chunk:
                raise EOFError("worker stdout closed")
            out += chunk
        return out

    def event(self, deadline: float) -> dict:
        if not self.selector.select(max(0.0, deadline - time.monotonic())):
            raise TimeoutError("worker event timeout")
        length = struct.unpack("<I", self._read_exact(4))[0]
        return json.loads(self._read_exact(length))

    def wait(self, kind: str, request_id: int, timeout: float = 60.0) -> dict:
        deadline = time.monotonic() + timeout
        while True:
            item = self.event(deadline)
            if item["request_id"] != request_id:
                continue
            if item["kind"] == kind:
                return item["body"]
            if item["kind"] in ("failed", "bootstrap_failed"):
                raise RuntimeError(json.dumps(item["body"]))

    def request(self, kind: str, body: dict, timeout: float = 60.0) -> dict:
        return self.wait("result", self.send(kind, body), timeout)

    def close(self) -> None:
        try:
            self.send("shutdown", {})
            self.proc.wait(timeout=10)
        except Exception:  # noqa: BLE001
            self.proc.kill()


def stage_artifact(root: Path, workdir: Path, channel: str = "github-release-unsigned") -> Path:
    """Locate the locked artifact for this host (cached copy or download) and verify it."""
    import hashlib
    import urllib.request

    lock = json.loads((root / "artifacts/duckomo.lock.json").read_text())
    host = detect_platform()
    entry = next(e for e in lock["entries"] if e["channel"] == channel and e["platform"] == host)
    for candidate in (Path(tempfile.gettempdir()) / "omobench-duckomo-artifacts" / entry["file"],):
        if candidate.is_file():
            data = candidate.read_bytes()
            if len(data) == entry["size"] and hashlib.sha256(data).hexdigest() == entry["sha256"]:
                return candidate
    dest = workdir / entry["file"]
    req = urllib.request.Request(entry["source"]["url"], headers={"User-Agent": "omobench-m1-verify/1"})
    with urllib.request.urlopen(req, timeout=120) as resp:
        data = resp.read()
    if len(data) != entry["size"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
        raise SystemExit("locked artifact size/hash mismatch")
    dest.write_bytes(data)
    return dest


def station_bbox(path: Path, pad_deg: float) -> dict:
    """Compute a padded single-span MapBbox around a station CSV (lat/lon columns)."""
    with path.open(encoding="utf-8-sig", newline="") as handle:
        rows = list(csv.DictReader(handle))
    lats = [float(row["lat"]) for row in rows]
    lons = [float(row["lon"]) for row in rows]
    return {
        "lat_min": min(lats) - pad_deg,
        "lat_max": max(lats) + pad_deg,
        "lon_spans": [[min(lons) - pad_deg, max(lons) + pad_deg]],
    }
