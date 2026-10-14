#!/usr/bin/env python3
"""L-004 / M-001 native POSIX bootstrap checks, including macOS Intel (ADR-0034).

Serves a native archive (renamed to a staging tag) from a local HTTP server laid out like
GitHub release downloads, generates `install.sh` with `--base-url`, and runs it through `sh` as a
non-root user with `HOME` / `XDG_DATA_HOME` redirected into the work directory.

    python3 tests/l004_posix_bootstrap.py --archive <axiom-X.Y.Z-linux-x64.tar.gz> --work <dir>
"""
from __future__ import annotations

import argparse
import functools
import hashlib
import http.server
import json
import os
import platform
import shutil
import subprocess
import sys
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GENERATOR = ROOT / "packaging" / "oneline" / "generate_bootstrappers.py"
LANE = ("macos-x64" if platform.system() == "Darwin" else
        "wsl2-linux-x64" if "microsoft" in platform.release().lower() else "linux-x64")
ASSET_PLATFORM = "macos-x64" if LANE == "macos-x64" else "linux-x64"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def publish(base: Path, tag: str, served: bytes, digest: str, base_url: str) -> Path:
    version = tag[1:]
    directory = base / tag
    directory.mkdir(parents=True, exist_ok=True)
    asset = f"axiom-{version}-{ASSET_PLATFORM}.tar.gz"
    (directory / asset).write_bytes(served)
    sums = "".join(f"{digest if lane == ASSET_PLATFORM else '0' * 64}  axiom-{version}-{lane}.{ext}\n"
                   for lane, ext in (("windows-x64", "zip"), ("linux-x64", "tar.gz"), ("macos-x64", "tar.gz")))
    (directory / "SHA256SUMS").write_text(sums)
    subprocess.run([sys.executable, str(GENERATOR), "--tag", tag, "--sums", str(directory / "SHA256SUMS"),
                    "--out", str(directory), "--base-url", base_url], check=True, capture_output=True)
    return directory / "install.sh"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--archive", type=Path, required=True)
    ap.add_argument("--work", type=Path, required=True)
    args = ap.parse_args()
    if platform.machine().lower() not in ("x86_64", "amd64") or os.getuid() == 0:
        ap.error("requires a native x64 non-root user")
    if not args.archive.name.endswith(f"-{ASSET_PLATFORM}.tar.gz"):
        ap.error("archive platform must match the executing host")
    work = args.work.resolve()
    if work.exists():
        ap.error("--work must be a new directory; existing work is preserved")
    srv = work / "srv"
    srv.mkdir(parents=True)
    good = args.archive.read_bytes()
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(srv))
    handler.log_message = lambda *a, **k: None
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{server.server_address[1]}"
    T = {n: f"v0.1.3-l004w.{n}" for n in range(1, 5)}
    publish(srv, T[1], good, sha(good), base)
    publish(srv, T[2], good, sha(good), base)
    publish(srv, T[3], good + b"tamper", sha(good), base)
    publish(srv, T[4], good[: len(good) // 2], sha(good), base)
    dead = publish(work / "dead", "v0.1.3-l004w.9", good, sha(good), "http://127.0.0.1:9")
    cases = []

    def run(name, command, extra=None):
        home = work / name
        tmp = work / f"tmp-{name}"
        home.mkdir(parents=True)
        tmp.mkdir()
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith("AXIOM")}
        env.update(HOME=str(home), XDG_DATA_HOME=str(home / "data"), TMPDIR=str(tmp), SHELL="/bin/sh")
        env.update(extra or {})
        proc = subprocess.run(["sh", "-c", command], env=env, capture_output=True, text=True,
                              stdin=subprocess.DEVNULL, start_new_session=True, timeout=1200)
        root = home / "data" / "axiom"
        return {"command": command, "exit_code": proc.returncode, "stdout_tail": proc.stdout[-1200:],
                "download_markers": [line for line in proc.stdout.splitlines() if "downloading " in line],
                "stderr_tail": proc.stderr[-1200:], "installed": (root / "installed.json").is_file(),
                "root_exists": root.exists(), "path_record": (root / "path-change.json").is_file(),
                "profile": (home / ".profile").read_text() if (home / ".profile").exists() else None,
                "temp_leftovers": sorted(p.name for p in tmp.iterdir())}

    def case(name, passed, **detail):
        cases.append({"case": name, "passed": bool(passed), **detail})

    url = lambda tag: f"{base}/{tag}/install.sh"  # noqa: E731
    r = run("oneliner", f"curl -fsSL {url(T[1])} | sh -s -- --yes")
    case("oneliner_installs_embedded_version", r["exit_code"] == 0 and r["installed"] and r["path_record"]
         and r["profile"] and "axiom" in r["profile"] and not r["temp_leftovers"], run=r)
    if r["installed"]:
        cli = work / "oneliner" / "data" / "axiom" / "bin" / "axm"
        if not cli.is_file():  # immutable older release inputs
            cli = cli.with_name("axiom-cli")
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith("AXIOM")}
        env.update(HOME=str(work / "oneliner"), XDG_DATA_HOME=str(work / "oneliner" / "data"))
        version = subprocess.run([str(cli), "version"], env=env, capture_output=True, text=True)
        case("installed_version_answers", version.returncode == 0 and ("installed:" in version.stdout or "Axiom Graph Engine" in version.stdout),
             stdout_tail=version.stdout[-400:])
    r = run("switches", f"curl -fsSL {url(T[1])} | sh -s -- --yes --no-modify-path")
    case("yes_and_no_modify_path_pass_through", r["exit_code"] == 0 and r["installed"] and not r["path_record"]
         and r["profile"] is None and not r["temp_leftovers"], run=r)
    r = run("pin", f"curl -fsSL {url(T[1])} | sh -s -- --yes --no-modify-path --version {T[2][1:]}")
    case("version_pin_runs_that_tags_installer", r["exit_code"] == 0 and r["installed"]
         and any(f"downloading {T[2]}" in line for line in r["download_markers"]), run=r)
    r = run("noyes", f"curl -fsSL {url(T[1])} | sh")
    case("no_terminal_without_yes_changes_nothing", r["exit_code"] == 4 and not r["root_exists"]
         and not r["temp_leftovers"], run=r)
    r = run("tampered", f"curl -fsSL {url(T[3])} | sh -s -- --yes")
    case("tampered_archive_refused", r["exit_code"] != 0 and not r["root_exists"] and "SHA-256 mismatch" in r["stderr_tail"]
         and not r["temp_leftovers"], run=r)
    r = run("truncated", f"curl -fsSL {url(T[4])} | sh -s -- --yes")
    case("truncated_archive_refused", r["exit_code"] != 0 and not r["root_exists"] and "SHA-256 mismatch" in r["stderr_tail"]
         and not r["temp_leftovers"], run=r)
    r = run("network", f"sh {dead} --yes")
    case("network_failure_refused", r["exit_code"] != 0 and not r["root_exists"] and "download failed" in r["stderr_tail"]
         and not r["temp_leftovers"], run=r)
    regen = work / "regen"
    subprocess.run([sys.executable, str(GENERATOR), "--tag", T[1], "--sums", str(srv / T[1] / "SHA256SUMS"),
                    "--out", str(regen), "--base-url", base], check=True, capture_output=True)
    case("generator_is_deterministic", (regen / "install.sh").read_bytes() == (srv / T[1] / "install.sh").read_bytes())
    if LANE == "macos-x64" and os.environ.get("GITHUB_ACTIONS") == "true":
        # Only the early root-refusal branch is exercised; no elevated installation is allowed.
        refused = subprocess.run(["sudo", "-n", "sh", str(dead), "--yes"], capture_output=True, text=True, timeout=15)
        case("native_root_refused_before_download", refused.returncode == 1
             and "refusing to run as root" in refused.stderr and "download" not in refused.stdout,
             exit_code=refused.returncode, stdout=refused.stdout, stderr=refused.stderr)
    server.shutdown()
    doc = {"task_id": "M-001" if LANE == "macos-x64" else "L-004", "lane": LANE,
           "execution": "native", "asset_source": "candidate",
           "root_refusal": "native CI refusal" if os.environ.get("GITHUB_ACTIONS") == "true" and LANE == "macos-x64"
                           else "unit identity fixture, not an elevated native execution",
           "host": {"system": platform.system(), "release": platform.release(), "machine": platform.machine(),
                    "uid": os.getuid()},
           "archive_sha256": sha(good), "cases": cases, "passed": all(c["passed"] for c in cases)}
    print(json.dumps(doc, indent=2))
    return 0 if doc["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
