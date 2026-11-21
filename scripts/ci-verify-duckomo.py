#!/usr/bin/env python3
"""Download locked Duckomo artifacts for the current host and run bootstrap + live worker checks."""
from __future__ import annotations

import argparse
import hashlib
import json
import platform
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path


def detect_platform() -> str:
    system = platform.system()
    machine = platform.machine().lower()
    if system == "Linux" and machine in ("aarch64", "arm64"):
        return "linux_arm64"
    if system == "Linux" and machine in ("x86_64", "amd64"):
        return "linux_amd64"
    if system == "Darwin" and machine in ("aarch64", "arm64"):
        return "osx_arm64"
    raise SystemExit(f"unsupported Duckomo lock platform: {system}/{machine}")


def download_verified(entry: dict, dest: Path, cache_dir: Path | None) -> Path:
    dest.parent.mkdir(parents=True, exist_ok=True)
    if cache_dir is not None:
        cached = cache_dir / entry["file"]
        if cached.is_file():
            data = cached.read_bytes()
            if len(data) == entry["size"] and hashlib.sha256(data).hexdigest() == entry["sha256"]:
                dest.write_bytes(data)
                return dest

    url = entry["source"]["url"]
    last_error: Exception | None = None
    for attempt in range(1, 4):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "omobench-ci-verify/1"})
            with urllib.request.urlopen(req, timeout=120) as resp:
                data = resp.read()
            if len(data) != entry["size"]:
                raise ValueError(
                    f"size mismatch for {entry['channel']}/{entry['platform']}: "
                    f"expected {entry['size']}, got {len(data)}"
                )
            actual_sha = hashlib.sha256(data).hexdigest()
            if actual_sha != entry["sha256"]:
                raise ValueError(
                    f"sha256 mismatch for {entry['channel']}/{entry['platform']}: "
                    f"expected {entry['sha256']}, got {actual_sha}"
                )
            dest.write_bytes(data)
            return dest
        except Exception as error:  # noqa: BLE001
            last_error = error
            if attempt < 3:
                time.sleep(attempt * 2)
    raise SystemExit(f"failed to fetch locked artifact {url}: {last_error}")


def run_checked(cmd: list[str], cwd: Path) -> str:
    result = subprocess.run(cmd, cwd=cwd, text=True, capture_output=True)
    if result.returncode != 0:
        print(f"command failed ({result.returncode}): {' '.join(cmd)}", file=sys.stderr)
        if result.stdout:
            print(result.stdout, file=sys.stderr)
        if result.stderr:
            print(result.stderr, file=sys.stderr)
        raise SystemExit(result.returncode)
    return result.stdout


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--probe", type=Path, default=Path("target/debug/omo-probe"))
    parser.add_argument("--worker", type=Path, default=Path("target/debug/omo-worker"))
    parser.add_argument("--lock", type=Path, default=Path("artifacts/duckomo.lock.json"))
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=None,
        help="optional directory containing pre-downloaded artifacts to reuse when hashes match",
    )
    args = parser.parse_args()

    repo_root = Path.cwd()
    host = detect_platform()
    lock = json.loads((repo_root / args.lock).read_text())

    summary: dict[str, object] = {"status": "passed", "platform": host, "channels": {}}
    with tempfile.TemporaryDirectory(prefix="omo-ci-duckomo-") as tmp:
        tmp_path = Path(tmp)
        artifacts: dict[str, Path] = {}
        for channel in ("community-signed", "github-release-unsigned"):
            entry = next(
                (
                    item
                    for item in lock["entries"]
                    if item["channel"] == channel and item["platform"] == host
                ),
                None,
            )
            if entry is None:
                raise SystemExit(f"no lock entry for {channel}/{host}")
            artifact_path = download_verified(entry, tmp_path / channel / entry["file"], args.cache_dir)
            artifacts[channel] = artifact_path

            probe_out = run_checked(
                [
                    str((repo_root / args.probe).resolve()),
                    "--channel",
                    channel,
                    "--repeat",
                    str(args.repeat),
                    "--artifact",
                    str(artifact_path),
                    "--lock",
                    str((repo_root / args.lock).resolve()),
                    "--data-dir",
                    str(tmp_path / f"probe-data-{channel}"),
                    "--ext-cache-dir",
                    str(tmp_path / f"probe-ext-{channel}"),
                ],
                cwd=repo_root,
            )
            probe_report = json.loads(probe_out)
            if probe_report.get("status") != "passed":
                raise SystemExit(f"omo-probe failed for {channel}/{host}: {probe_out}")
            golden = probe_report["attempts"][0]["bootstrap"]["golden"]
            summary["channels"][channel] = {
                "golden_mode": probe_report["golden_mode"],
                "attempts": probe_report["attempts_completed"],
                "golden_sha256": golden["actual"],
            }

        live_out = run_checked(
            [
                sys.executable,
                str((repo_root / "scripts/check-worker-live.py").resolve()),
                "--artifact",
                str(artifacts["github-release-unsigned"]),
                "--worker",
                str((repo_root / args.worker).resolve()),
            ],
            cwd=repo_root,
        )
        live_report = json.loads(live_out)
        if live_report.get("status") != "passed":
            raise SystemExit(f"check-worker-live failed on {host}: {live_out}")
        summary["live_worker"] = {
            "result_rows": live_report["result_rows"],
            "cancellation": live_report["cancellation"],
        }

    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
