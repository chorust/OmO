#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode="${1:-}"

python3 - "$repo_root" "$mode" <<'PY'
from __future__ import annotations

import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib

root = pathlib.Path(sys.argv[1]).resolve()
mode = sys.argv[2]
CORE = {"omo-engine", "omo-protocol", "omo-worker", "omo-project"}
APP = "omo-app"
UI_RE = re.compile(r"^(?:nsh[-_].*|gpui.*|tauri.*)$", re.I)
APP_FORBIDDEN_RE = re.compile(
    r"^(?:duckdb|libduckdb-sys|gpui.*|tauri.*|.*quickjs.*|nsh-js|nsh-platform-gpui)$",
    re.I,
)
APP_DIRECT_FORBIDDEN_RE = re.compile(
    r"^(?:reqwest|ureq|hyper|isahc)$",
    re.I,
)
ENGINE_FORBIDDEN_RE = re.compile(r"^(?:zstd-sys|tokio|reqwest)$", re.I)
HOST_NSH_RE = re.compile(r"\bnsh_[A-Za-z0-9_]*\b")


def dependency_names(name: str, spec: object) -> list[str]:
    names = [name]
    if isinstance(spec, dict):
        package = spec.get("package")
        if isinstance(package, str) and package not in names:
            names.append(package)
    return names


def strip_rust_comments_and_strings(source: str) -> str:
    out: list[str] = []
    i = 0
    n = len(source)
    block_depth = 0
    while i < n:
        ch = source[i]
        nxt = source[i + 1] if i + 1 < n else ""
        if block_depth > 0:
            if ch == "/" and nxt == "*":
                block_depth += 1
                i += 2
            elif ch == "*" and nxt == "/":
                block_depth -= 1
                i += 2
            else:
                if ch == "\n":
                    out.append("\n")
                i += 1
            continue
        if ch == "/" and nxt == "/":
            while i < n and source[i] != "\n":
                i += 1
            continue
        if ch == "/" and nxt == "*":
            block_depth = 1
            i += 2
            continue
        if ch == '"':
            out.append(" ")
            i += 1
            while i < n:
                if source[i] == "\\":
                    i += 2
                    continue
                if source[i] == '"':
                    i += 1
                    break
                if source[i] == "\n":
                    out.append("\n")
                i += 1
            continue
        if ch == "r" and (nxt == '"' or nxt == "#"):
            hashes = 0
            j = i + 1
            while j < n and source[j] == "#":
                hashes += 1
                j += 1
            if j < n and source[j] == '"':
                out.append(" ")
                i = j + 1
                closing = '"' + ("#" * hashes)
                end = source.find(closing, i)
                if end == -1:
                    break
                out.append("\n" * source[i:end].count("\n"))
                i = end + len(closing)
                continue
        out.append(ch)
        i += 1
    return "".join(out)


def host_nsh_violations(base: pathlib.Path) -> list[str]:
    violations: list[str] = []
    host_dir = base / "crates" / "omo-app" / "src" / "host"
    if not host_dir.is_dir():
        return violations
    for path in sorted(host_dir.rglob("*.rs")):
        cleaned = strip_rust_comments_and_strings(path.read_text())
        for lineno, line in enumerate(cleaned.splitlines(), start=1):
            matches = sorted(set(HOST_NSH_RE.findall(line)))
            if matches:
                violations.append(
                    f"omo-app::host: forbidden NSH reference {', '.join(matches)} in {path}:{lineno}"
                )
    return violations


def manifest_paths(base: pathlib.Path) -> list[pathlib.Path]:
    return [base / "Cargo.toml", *sorted((base / "crates").glob("*/Cargo.toml"))]


def package_manifest(base: pathlib.Path, package: str) -> tuple[pathlib.Path, dict]:
    for path in manifest_paths(base):
        data = tomllib.loads(path.read_text())
        if data.get("package", {}).get("name") == package:
            return path, data
    raise RuntimeError(f"manifest for {package} not found")


def dependency_tables(data: dict):
    for key in ("dependencies", "build-dependencies"):
        if key in data:
            yield data[key]
    for target in data.get("target", {}).values():
        for key in ("dependencies", "build-dependencies"):
            if key in target:
                yield target[key]


def static_violations(base: pathlib.Path) -> list[str]:
    violations: list[str] = []
    for package in sorted(CORE):
        path, data = package_manifest(base, package)
        for table in dependency_tables(data):
            for name, spec in table.items():
                for dep in dependency_names(name, spec):
                    if UI_RE.match(dep):
                        violations.append(f"{package}: forbidden UI dependency `{dep}` in {path}")
    path, app_data = package_manifest(base, APP)
    for table in dependency_tables(app_data):
        for name, spec in table.items():
            for dep in dependency_names(name, spec):
                if APP_FORBIDDEN_RE.match(dep) or APP_DIRECT_FORBIDDEN_RE.match(dep):
                    violations.append(f"{APP}: forbidden dependency `{dep}` in {path}")
    workspace_data = tomllib.loads((base / "Cargo.toml").read_text()).get("workspace", {})
    workspace = workspace_data.get("dependencies", {})
    for name, spec in workspace.items():
        if name.startswith("nsh-"):
            version = spec.get("version") if isinstance(spec, dict) else spec
            if version != "=0.5.1":
                violations.append(f"workspace: {name} must be pinned to =0.5.1, found {version!r}")

    for manifest in manifest_paths(base):
        data = tomllib.loads(manifest.read_text())
        for table in dependency_tables(data):
            for name, spec in table.items():
                if not name.startswith("nsh-"):
                    continue
                if isinstance(spec, dict) and spec.get("workspace") is True:
                    spec = workspace.get(name)
                version = spec.get("version") if isinstance(spec, dict) else spec
                if version != "=0.5.1":
                    violations.append(f"{manifest}: {name} must be pinned to =0.5.1, found {version!r}")

    engine_path, engine_data = package_manifest(base, "omo-engine")
    engine_deps = engine_data.get("dependencies", {})
    engine_duckdb = engine_deps.get("duckdb")
    if engine_duckdb is not None and not (isinstance(engine_duckdb, dict) and engine_duckdb.get("workspace") is True):
        version = engine_duckdb.get("version") if isinstance(engine_duckdb, dict) else engine_duckdb
        if version != "=1.20000.1":
            violations.append(f"omo-engine: duckdb must be pinned to =1.20000.1, found {version!r}")
    for table in dependency_tables(engine_data):
        for name, spec in table.items():
            for dep in dependency_names(name, spec):
                if ENGINE_FORBIDDEN_RE.match(dep):
                    violations.append(f"omo-engine: forbidden dependency `{dep}` in {engine_path}")
    root_workspace = tomllib.loads((base / "Cargo.toml").read_text()).get("workspace", {}).get("dependencies", {})
    workspace_duckdb = root_workspace.get("duckdb")
    workspace_version = workspace_duckdb.get("version") if isinstance(workspace_duckdb, dict) else workspace_duckdb
    if workspace_version != "=1.20000.1":
        violations.append(f"workspace: duckdb must be pinned to =1.20000.1, found {workspace_version!r}")
    violations.extend(host_nsh_violations(base))
    return violations


def copy_manifests(source: pathlib.Path, target: pathlib.Path) -> None:
    shutil.copy2(source / "Cargo.toml", target / "Cargo.toml")
    (target / "crates").mkdir()
    for path in (source / "crates").glob("*/Cargo.toml"):
        package_dir = target / "crates" / path.parent.name
        package_dir.mkdir(parents=True)
        shutil.copy2(path, package_dir / "Cargo.toml")


def inject_dependency(base: pathlib.Path, package: str, dependency: str, *, alias: str | None = None) -> None:
    path, _ = package_manifest(base, package)
    text = path.read_text()
    marker = "[dependencies]\n"
    if marker not in text:
        raise RuntimeError(f"{path} has no [dependencies] table")
    if alias is None:
        line = f'{dependency} = "0.1"\n'
    else:
        line = f'{alias} = {{ package = "{dependency}", version = "0.1" }}\n'
    path.write_text(text.replace(marker, marker + line, 1))


def self_test() -> int:
    cases = [
        ("omo-engine", "nsh-core", None),
        ("omo-protocol", "gpui", None),
        ("omo-worker", "tauri", None),
        ("omo-project", "nsh-platform-gpui", None),
        ("omo-engine", "zstd-sys", None),
        ("omo-engine", "tokio", None),
        ("omo-engine", "reqwest", None),
        (APP, "duckdb", None),
        (APP, "libduckdb-sys", None),
        (APP, "gpui", None),
        (APP, "tauri", None),
        (APP, "quickjs-core", None),
        (APP, "rquickjs", None),
        (APP, "nsh-js", None),
        (APP, "nsh-js", "js_alias"),
        (APP, "nsh-platform-gpui", None),
        (APP, "reqwest", None),
        (APP, "ureq", None),
        (APP, "hyper", None),
        (APP, "isahc", None),
    ]
    failures = []
    with tempfile.TemporaryDirectory(prefix="omo-boundaries-") as temp:
        scratch = pathlib.Path(temp)
        for index, (package, dependency, alias) in enumerate(cases):
            case_root = scratch / str(index)
            case_root.mkdir()
            copy_manifests(root, case_root)
            inject_dependency(case_root, package, dependency, alias=alias)
            violations = static_violations(case_root)
            if not violations:
                failures.append(f"injection not detected: {package} -> {dependency} (alias={alias})")

        host_case_root = scratch / "host-nsh-ref"
        host_case_root.mkdir()
        copy_manifests(root, host_case_root)
        host_dir = host_case_root / "crates" / "omo-app" / "src" / "host"
        host_dir.mkdir(parents=True)
        (host_dir / "leak.rs").write_text(
            "pub fn leak() { let _ = nsh_platform_linux::gtk4::glib::ControlFlow::Continue; }\n"
        )
        if not static_violations(host_case_root):
            failures.append("injection not detected: omo-app::host -> nsh_platform_linux")
    if failures:
        print("boundary self-test failed:", *failures, sep="\n  ", file=sys.stderr)
        return 1
    print(f"boundary self-test passed ({len(cases) + 1} forbidden boundary injections detected)")
    return 0


if mode == "--self-test":
    raise SystemExit(self_test())
if mode:
    print(f"unknown option: {mode}", file=sys.stderr)
    raise SystemExit(2)

violations = static_violations(root)
if violations:
    print("manifest boundary checks failed:", *violations, sep="\n  ", file=sys.stderr)
    raise SystemExit(1)

for package in sorted(CORE):
    result = subprocess.run(
        ["cargo", "tree", "-p", package, "-e", "normal,build", "--locked"],
        cwd=root, text=True, capture_output=True,
    )
    if result.returncode:
        print(result.stderr, file=sys.stderr)
        raise SystemExit(result.returncode)
    names = re.findall(r"(?m)^\s*[├└│─ ]*([A-Za-z0-9_-]+) v", result.stdout)
    offenders = [name for name in names if UI_RE.match(name)]
    if offenders:
        print(f"{package}: forbidden transitive UI dependencies: {', '.join(sorted(set(offenders)))}", file=sys.stderr)
        raise SystemExit(1)

result_direct = subprocess.run(
    ["cargo", "tree", "-p", APP, "-e", "normal", "--depth", "1", "--all-features", "--locked"],
    cwd=root, text=True, capture_output=True,
)
if result_direct.returncode:
    print(result_direct.stderr, file=sys.stderr)
    raise SystemExit(result_direct.returncode)
direct_names = re.findall(r"(?m)^\s*[├└│─ ]*([A-Za-z0-9_-]+) v", result_direct.stdout)
direct_offenders = [name for name in direct_names if APP_DIRECT_FORBIDDEN_RE.match(name)]
if direct_offenders:
    joined = ", ".join(sorted(set(direct_offenders)))
    print(f"{APP}: forbidden direct HTTP client dependencies: {joined}", file=sys.stderr)
    raise SystemExit(1)

result = subprocess.run(
    ["cargo", "tree", "-p", APP, "-e", "normal", "--locked"],
    cwd=root, text=True, capture_output=True,
)
if result.returncode:
    print(result.stderr, file=sys.stderr)
    raise SystemExit(result.returncode)
names = re.findall(r"(?m)^\s*[├└│─ ]*([A-Za-z0-9_-]+) v", result.stdout)
offenders = [name for name in names if APP_FORBIDDEN_RE.match(name)]
if offenders:
    print(f"{APP}: forbidden transitive dependencies: {', '.join(sorted(set(offenders)))}", file=sys.stderr)
    raise SystemExit(1)
print("dependency boundaries are valid")
PY
