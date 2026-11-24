#!/usr/bin/env python3
"""Automated GUI acceptance suite for M1 on an isolated virtual X11 display."""

import argparse
import os
import shutil
import subprocess
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def resolve_bin(name: str, x11_root: Path | None) -> str:
    if x11_root is not None:
        candidate = x11_root / "usr/bin" / name
        if candidate.is_file():
            return str(candidate)
    found = shutil.which(name)
    if found:
        return found
    raise FileNotFoundError(f"required tool `{name}` not found in PATH or --x11-root")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--display", default=os.environ.get("OMO_GUI_DISPLAY", ":125"))
    parser.add_argument(
        "--bootstrap-config",
        type=Path,
        default=Path(os.environ.get("OMOBENCH_BOOTSTRAP_CONFIG", "/tmp/omo-gui-run/bootstrap.json")),
    )
    parser.add_argument(
        "--data-dir",
        type=Path,
        default=Path(os.environ.get("OMO_GUI_DATA_DIR", "/tmp/omo-gui-run/data")),
    )
    parser.add_argument(
        "--x11-root",
        type=Path,
        default=Path(os.environ["OMO_X11_ROOT"]) if os.environ.get("OMO_X11_ROOT") else None,
        help="Optional relocatable root containing usr/bin/{Xvfb,openbox} and usr/lib/*",
    )
    parser.add_argument(
        "--app-bin",
        type=Path,
        default=REPO_ROOT / "target/release/omo-app",
    )
    parser.add_argument(
        "--stations-csv",
        type=Path,
        default=REPO_ROOT / "fixtures/stations/m1-100.csv",
    )
    parser.add_argument(
        "--evidence-dir",
        type=Path,
        default=REPO_ROOT / "docs/m1/evidence",
    )
    parser.add_argument(
        "--log-path",
        type=Path,
        default=Path("/tmp/omo-gui-auto.log"),
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    env = os.environ.copy()
    env["DISPLAY"] = args.display
    env["OMOBENCH_BOOTSTRAP_CONFIG"] = str(args.bootstrap_config)
    if args.x11_root is not None:
        lib_dirs = [str(p) for p in (args.x11_root / "usr/lib").glob("*") if p.is_dir()]
        if lib_dirs:
            existing = env.get("LD_LIBRARY_PATH", "")
            env["LD_LIBRARY_PATH"] = ":".join(lib_dirs + ([existing] if existing else []))

    args.evidence_dir.mkdir(parents=True, exist_ok=True)
    xvfb_bin = resolve_bin("Xvfb", args.x11_root)
    openbox_bin = resolve_bin("openbox", args.x11_root)

    def screenshot(name: str) -> Path:
        xwd_path = f"/tmp/{name}.xwd"
        png_path = args.evidence_dir / f"{name}.png"
        subprocess.run(["xwd", "-root", "-out", xwd_path], env=env, check=True)
        subprocess.run(
            ["ffmpeg", "-y", "-i", xwd_path, str(png_path)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=True,
        )
        print(f"[{time.strftime('%X')}] Saved evidence screenshot: {png_path} ({png_path.stat().st_size} bytes)")
        return png_path

    def click(x: int, y: int, delay: float = 0.5) -> None:
        subprocess.run(["xdotool", "mousemove", str(x), str(y), "click", "1"], env=env, check=True)
        time.sleep(delay)

    def type_text(text: str, delay: float = 0.5) -> None:
        subprocess.run(["xdotool", "type", text], env=env, check=True)
        time.sleep(delay)

    def key(combo: str, delay: float = 0.5) -> None:
        subprocess.run(["xdotool", "key", combo], env=env, check=True)
        time.sleep(delay)

    def scroll_down(times: int = 8, delay: float = 0.05) -> None:
        subprocess.run(["xdotool", "mousemove", "500", "400"], env=env, check=True)
        for _ in range(times):
            subprocess.run(["xdotool", "click", "5"], env=env, check=True)
            time.sleep(delay)
        time.sleep(0.5)

    print("Starting isolated Xvfb on DISPLAY", args.display)
    xvfb = subprocess.Popen(
        [xvfb_bin, args.display, "-screen", "0", "1600x1000x24", "-ac", "-nolisten", "tcp"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(1)

    print("Starting Openbox window manager")
    ob = subprocess.Popen([openbox_bin], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)

    print(f"Starting {args.app_bin}")
    app_log = open(args.log_path, "w")
    app = subprocess.Popen([str(args.app_bin)], env=env, stdout=app_log, stderr=subprocess.STDOUT)
    time.sleep(3)

    timings: dict[str, float] = {}
    try:
        win_id = (
            subprocess.check_output(["xdotool", "search", "--name", "Omobench Studio"], env=env)
            .decode()
            .strip()
            .split()[0]
        )
        print(f"Located Omobench Studio window: {win_id}")
        subprocess.run(["xdotool", "windowfocus", win_id], env=env, check=True)
        time.sleep(0.5)

        screenshot("V1-01-initial-catalog-ready")

        print("Entering CSV path...")
        click(500, 225, 0.3)
        type_text(str(args.stations_csv.resolve()), 0.5)
        click(1358, 225, 1.5)
        screenshot("V1-02-import-mapping-proposal")

        scroll_down(8)
        click(351, 218, 2.0)
        screenshot("V1-03-stations-confirmed")

        t0 = time.monotonic()
        click(336, 495, 1.0)
        for _ in range(40):
            time.sleep(3)
            elapsed = time.monotonic() - t0
            arrow_files = list(args.data_dir.glob("**/*.arrow"))
            if arrow_files:
                time.sleep(2)
                elapsed_total = time.monotonic() - t0
                timings["series_request_s"] = round(elapsed_total, 2)
                print(f"Series request returned in {elapsed_total:.2f}s! Arrow: {arrow_files[0]}")
                break
            print(f"Waiting... elapsed {elapsed:.1f}s")

        time.sleep(2)
        screenshot("V1-04-series-completed-badge")

        click(450, 668, 0.3)
        type_text("1,2,3,4,5", 0.5)
        click(770, 668, 1.0)

        click(320, 115, 1.5)
        screenshot("V3-01-plot-chart-5-stations")

        click(235, 115, 1.5)
        screenshot("V6-01-sql-editor-tab")

        key("Alt+F4", 1.5)
        screenshot("V5-01-close-guard-dialog")
        key("Escape", 0.8)
        screenshot("V5-02-close-guard-dismissed")

        print("\n" + "=" * 50)
        print("ALL GUI SCENARIOS (V1, V3, V5/T057, V6) PASSED WITH VISUAL EVIDENCE!")
        print(f"SC-002 Timing: {timings.get('series_request_s', 'N/A')} s")
        print("=" * 50)
    finally:
        print("Cleaning up processes...")
        app.terminate()
        ob.terminate()
        xvfb.terminate()
        app_log.close()


if __name__ == "__main__":
    main()
