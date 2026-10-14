#!/usr/bin/env python3
"""L-005 real-host harness (Windows x64 lane): bare `axiom-cli update` against a channel manifest.

Release A is the extracted published release with the CLI under test. Release B is the same bytes
with every channel component version raised to 0.1.3, so the graphd engine runs a real update
transaction into a new version directory. Both are served from a local HTTP server laid out like
GitHub release downloads, with `channel.json` documents in the ADR-0033 format (a `release` block
pinning tag, archive URL, size and SHA-256). Per-user data and the PATH target are redirected.

    python tests/l005_one_command_update.py --release <extracted-release> --cli target/release/axiom-cli.exe --work <short-dir>
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
import zipfile
from pathlib import Path

TEST_KEY = r"Software\AxiomCliTest\L005"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def tree_digest(root: Path) -> dict:
    return {p.relative_to(root).as_posix(): sha(p.read_bytes()) for p in sorted(root.rglob("*")) if p.is_file()}


def stage(release: Path, cli: Path, target: Path, version: str | None) -> Path:
    shutil.copytree(release, target, ignore=shutil.ignore_patterns("axiom-*-*.zip", "axiom-*-*.tar.gz"))
    shutil.copy2(cli, target / "axiom-cli.exe")
    if version:
        channel = json.loads((target / "channel.json").read_text(encoding="utf-8"))
        for component in channel["components"]:
            if component["component"] == "axiom-graphd":
                component["version"] = version  # MCP and skills version independently
        (target / "channel.json").write_text(json.dumps(channel, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return target


def zip_dir(source: Path, out: Path) -> bytes:
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(source.rglob("*")):
            if path.is_file():
                archive.write(path, path.relative_to(source).as_posix())
    return out.read_bytes()


def channel_doc(version: str, url: str, digest: str, size: int, tag: str | None = None) -> bytes:
    return json.dumps({"channel": "stable", "manifest_version": 1,
                       "release": {"version": version, "tag": tag or f"v{version}",
                                   "archives": [{"platform": "windows-x64", "url": url, "sha256": digest, "size_bytes": size}]}},
                      indent=2, sort_keys=True).encode()


def serve(directory: Path):
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(directory))
    handler.log_message = lambda *a, **k: None
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--release", type=Path, required=True)
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--work", type=Path, required=True)
    args = ap.parse_args()
    if os.name != "nt":
        print(json.dumps({"task_id": "L-005", "lane": "windows-x64", "status": "not_run", "reason": "not a Windows host"}))
        return 0
    work = args.work.resolve()
    if work.exists():
        shutil.rmtree(work)
    srv = work / "srv"
    srv.mkdir(parents=True)
    rel_a = stage(args.release, args.cli, work / "rel-a", None)
    rel_b = stage(args.release, args.cli, work / "stage-b", "0.1.3")
    (srv / "v0.1.3").mkdir()
    b_zip = zip_dir(rel_b, srv / "v0.1.3" / "axiom-0.1.3-windows-x64.zip")
    shutil.rmtree(rel_b)
    server = serve(srv)
    base = f"http://127.0.0.1:{server.server_address[1]}"
    b_url = f"{base}/v0.1.3/axiom-0.1.3-windows-x64.zip"
    docs = {
        "same.json": channel_doc("0.1.2", f"{base}/v0.1.2/axiom-0.1.2-windows-x64.zip", "a" * 64, 1),
        "b.json": channel_doc("0.1.3", b_url, sha(b_zip), len(b_zip)),
        "alias.json": channel_doc("latest", b_url, sha(b_zip), len(b_zip), "vlatest"),
        "branch.json": channel_doc("0.1.3", f"{base}/main/axiom-0.1.3-windows-x64.zip", sha(b_zip), len(b_zip)),
        "star.json": channel_doc("*", b_url, sha(b_zip), len(b_zip), "v*"),
        "latest-url.json": channel_doc("0.1.3", f"{base}/latest/download/axiom-0.1.3-windows-x64.zip", sha(b_zip), len(b_zip)),
        "corrupt.json": channel_doc("0.1.3", b_url, "b" * 64, len(b_zip)),
    }
    for name, data in docs.items():
        (srv / name).write_bytes(data)

    data_home = work / "d1"
    env = {k: v for k, v in os.environ.items() if not k.upper().startswith("AXIOM")}
    env.update(LOCALAPPDATA=str(data_home), AXIOM_CLI_TEST_USER_ENV_KEY=TEST_KEY)
    root = data_home / "Axiom"
    cli = root / "bin" / "axiom-cli.exe"
    cases = []

    def run(argv, extra=None):
        e = dict(env)
        e.update(extra or {})
        proc = subprocess.run([str(a) for a in argv], env=e, capture_output=True, text=True, cwd=work,
                              stdin=subprocess.DEVNULL, timeout=900)
        return {"argv": [str(a) for a in argv[1:]], "exit_code": proc.returncode,
                "stdout_tail": proc.stdout[-1500:], "stderr_tail": proc.stderr[-1500:]}

    def case(name, passed, **detail):
        cases.append({"case": name, "passed": bool(passed), **detail})

    def installed():
        return json.loads((root / "installed.json").read_text(encoding="utf-8"))

    def engine_current():
        path = root / "installs" / "ecosystem" / "current"
        return path.read_text(encoding="utf-8").strip() if path.is_file() else None

    out = run([rel_a / "axiom-cli.exe", "install", "--yes", "--no-modify-path"])
    case("install_a", out["exit_code"] == 0 and (root / "installed.json").is_file(), run=out)
    workspace = work / "workspace"
    (workspace / "graph-output").mkdir(parents=True)
    (workspace / "notes.md").write_text("user data", encoding="utf-8")
    (workspace / "graph-output" / "graph.json").write_text('{"nodes": 1}', encoding="utf-8")
    user_before = tree_digest(workspace)
    record_a = (root / "installed.json").read_bytes()
    engine_a = engine_current()

    out = run([cli, "update", "--channel", f"{base}/same.json"])
    case("up_to_date", out["exit_code"] == 0 and "up to date" in out["stdout_tail"]
         and (root / "installed.json").read_bytes() == record_a, run=out)
    out = run([cli, "update", "--channel", f"{base}/b.json"])
    case("no_terminal_without_yes_changes_nothing", out["exit_code"] == 4 and "0.1.2 -> 0.1.3" in out["stderr_tail"]
         and (root / "installed.json").read_bytes() == record_a, run=out)
    for name, reason in [("alias.json", "forbidden_pin"), ("branch.json", "not an exact URL"),
                         ("star.json", "forbidden_pin"), ("latest-url.json", "not an exact URL")]:
        out = run([cli, "--json", "update", "--yes", "--channel", f"{base}/{name}"])
        case(f"refuses_{name.split('.')[0]}", out["exit_code"] == 2 and (reason in out["stdout_tail"])
             and (root / "installed.json").read_bytes() == record_a, run=out)
    out = run([cli, "--json", "update", "--yes", "--channel", f"{base}/corrupt.json"])
    case("corrupted_download_refused", out["exit_code"] == 2 and "artifact_digest_mismatch" in out["stdout_tail"]
         and (root / "installed.json").read_bytes() == record_a and engine_current() == engine_a, run=out)
    out = run([cli, "--json", "update", "--yes", "--channel", f"{base}/b.json"], {"AXIOM_CLI_TEST_FAIL_HEALTH": "1"})
    case("failed_health_check_rolls_back", out["exit_code"] == 8 and '"rolled_back":true' in out["stdout_tail"]
         and (root / "installed.json").read_bytes() == record_a and engine_current() == engine_a, run=out,
         engine_current=engine_current())

    out = run([cli, "update", "--yes", "--channel", f"{base}/b.json"])
    record_b = installed() if (root / "installed.json").is_file() else {}
    versions = {c["component"]: c["version"] for c in record_b.get("components", [])}
    version_out = run([cli, "version"])
    case("update_applies_newer_release", out["exit_code"] == 0 and versions.get("axiom-graphd") == "0.1.3"
         and versions.get("bin/axiom-cli") == "0.1.3"
         and record_b.get("previous_generation") and (root / "installed.previous.json").is_file()
         and engine_current() != engine_a and ("installed: 0.1.3" in version_out["stdout_tail"] or ("Axiom Graph Engine" in version_out["stdout_tail"] and "0.1.3" in version_out["stdout_tail"])),
         run=out, version=version_out, engine_current=engine_current(), last_update=record_b.get("last_update"))
    case("user_data_unchanged_after_update", tree_digest(workspace) == user_before)

    out = run([cli, "update", "rollback", "--transaction", "previous"])
    case("rollback_restores_previous", out["exit_code"] == 0 and (root / "installed.json").read_bytes() == record_a
         and engine_current() == engine_a, run=out, engine_current=engine_current())
    case("user_data_unchanged_after_rollback", tree_digest(workspace) == user_before)
    out = run([cli, "update", "check"])
    case("check_subcommand_still_answers", out["exit_code"] in (0, 4), run=out)

    server.shutdown()
    subprocess.run(["reg", "delete", r"HKCU\Software\AxiomCliTest", "/f"], capture_output=True)
    doc = {"task_id": "L-005", "lane": "windows-x64",
           "host": {"system": platform.system(), "release": platform.release(), "machine": platform.machine()},
           "cli_sha256": sha(args.cli.read_bytes()), "release_b_sha256": sha(b_zip), "cases": cases,
           "passed": all(c["passed"] for c in cases)}
    print(json.dumps(doc, indent=2))
    return 0 if doc["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
