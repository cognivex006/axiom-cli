#!/usr/bin/env python3
"""L-006 real-host harness (Windows x64 lane): reproduce the 2026-10-06 state and adopt it.

The state observed on a real Windows x64 host on 2026-10-06 is rebuilt inside a work directory:

* `%LOCALAPPDATA%\\Axiom` - a J-004 CLI store fixture (`bin\\axiom-cli.exe` = the published
  0.1.2 CLI bytes, `cli\\`, `install-manifest.json` with `release_version` 0.1.0), first on PATH;
* `%USERPROFILE%\\axiom` - a real 0.1.2 bootstrap root produced by the published release's own
  `bootstrap_windows.ps1` (engine ecosystem + nested `mcp-runtime\\mcp-runtime`), plus user data
  and graph output placed inside it;
* `AXIOM_CLI_INSTALL_ROOT` / `AXIOM_ENGINE_BIN` leftovers (for the doctor case only).

`LOCALAPPDATA`, `USERPROFILE`, `PATH` and the PATH registry target are redirected, so the real
account is not touched.

    python tests/l006_adopt_legacy.py --release <extracted 0.1.2 release> --cli target/release/axiom-cli.exe --work <short-dir>
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

TEST_KEY = r"Software\AxiomCliTest\L006"
INSTALL_DIRS = {"bin", "generations", "installs", "staging", "mcp-runtime", "legacy", "journal", "state"}


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def tree(root: Path, skip=()) -> dict:
    out = {}
    for path in sorted(root.rglob("*")):
        rel = path.relative_to(root)
        if path.is_file() and rel.parts[0] not in skip:
            out[rel.as_posix()] = sha(path.read_bytes())
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--release", type=Path, required=True)
    ap.add_argument("--cli", type=Path, required=True)
    ap.add_argument("--work", type=Path, required=True)
    args = ap.parse_args()
    if os.name != "nt":
        print(json.dumps({"task_id": "L-006", "lane": "windows-x64", "status": "not_run", "reason": "not a Windows host"}))
        return 0
    work = args.work.resolve()
    if work.exists():
        shutil.rmtree(work)
    local = work / "Local"
    profile = work / "Profile"
    store = local / "Axiom"
    boot = profile / "axiom"
    for path in (store / "bin", store / "cli" / "generations" / "0.1.0", profile):
        path.mkdir(parents=True)
    published_cli = args.release / "axiom-cli.exe"
    shutil.copy2(published_cli, store / "bin" / "axiom-cli.exe")
    shutil.copy2(published_cli, store / "cli" / "generations" / "0.1.0" / "axiom-cli.exe")
    (store / "cli" / "state.json").write_text(json.dumps({"current": "0.1.0"}), encoding="utf-8")
    (store / "install-manifest.json").write_text(json.dumps({
        "schema_version": 1, "document_kind": "axiom-cli-install-manifest", "platform": "windows-x64",
        "release_version": "0.1.0", "components": [], "preserved_roots": []}, indent=2), encoding="utf-8")

    base_env = {k: v for k, v in os.environ.items() if not k.upper().startswith("AXIOM")}
    base_env.update(LOCALAPPDATA=str(local), USERPROFILE=str(profile), AXIOM_CLI_TEST_USER_ENV_KEY=TEST_KEY)
    base_env["PATH"] = str(store / "bin") + os.pathsep + base_env.get("PATH", "")

    # The real 0.1.2 bootstrap root, made by the published bootstrap script.
    ps = ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(args.release / "bootstrap_windows.ps1"), "-Root", str(boot)]
    dry = subprocess.run(ps + ["-DryRun"], env=base_env, capture_output=True, text=True)
    digest = json.loads(dry.stdout.strip().splitlines()[-1])["plan_digest"]
    boot_run = subprocess.run(ps + ["-Apply", "-ApproveDigest", digest], env=base_env, capture_output=True, text=True, timeout=900)
    (boot / "graph-output").mkdir()
    (boot / "graph-output" / "graph.json").write_text('{"nodes": 3}', encoding="utf-8")
    (boot / "instances" / "demo").mkdir(parents=True)
    (boot / "instances" / "demo" / "state.json").write_text('{"queue": []}', encoding="utf-8")
    odd = work / "odd"
    (odd / "installs").mkdir(parents=True)
    (odd / "thing.txt").write_text("not axiom", encoding="utf-8")

    rel = work / "rel-new"
    shutil.copytree(args.release, rel, ignore=shutil.ignore_patterns("axiom-*-*.zip", "axiom-*-*.tar.gz"))
    shutil.copy2(args.cli, rel / "axiom-cli.exe")
    cli = rel / "axiom-cli.exe"
    cases = []

    def run(argv, extra=None, cwd=None):
        env = dict(base_env)
        env.update(extra or {})
        proc = subprocess.run([str(a) for a in argv], env=env, capture_output=True, text=True,
                              cwd=cwd or work, stdin=subprocess.DEVNULL, timeout=900)
        return {"argv": [str(a) for a in argv[1:]], "exit_code": proc.returncode,
                "stdout_tail": proc.stdout[-2500:], "stderr_tail": proc.stderr[-1500:], "_full": proc.stdout}

    def case(name, passed, **detail):
        cases.append({"case": name, "passed": bool(passed), **detail})

    case("bootstrap_fixture_built", boot_run.returncode == 0 and (boot / "installs/ecosystem/current").is_file()
         and (boot / "mcp-runtime/mcp-runtime/current.json").is_file(),
         run={"exit_code": boot_run.returncode, "stderr_tail": boot_run.stderr[-800:]})

    # A. doctor names every legacy layout, the shadowing CLI and the leftover variables.
    out = run([cli, "--json", "doctor"], {"AXIOM_CLI_INSTALL_ROOT": str(boot), "AXIOM_ENGINE_BIN": str(args.release / "axiom.exe")})
    doc = json.loads(out["_full"]) if out["_full"].strip().startswith("{") else {}
    legacy = next((f for f in doc.get("details", {}).get("findings", []) if f.get("check") == "legacy"), {})
    text = json.dumps(legacy)
    case("doctor_names_legacy_state", "cli-store" in text and "0.1.0" in text and "bootstrap-root" in text
         and "0.1.2" in text and '"before_this_install": true' in text.replace('":true', '": true')
         and "AXIOM_ENGINE_BIN" in text and "AXIOM_CLI_INSTALL_ROOT" in text
         and any(row.get("version") == "0.1.2" and row["path"].startswith(str(store))
                 for row in legacy.get("path_shadowing", [])),
         finding=legacy, exit_code=out["exit_code"])

    store_keep = {name: tree(store, skip={"bin", "generations", "installs", "staging", "mcp-runtime", "legacy", "journal", "state"})
                  for name in ["before"]}["before"]
    old_cli = sha((store / "bin" / "axiom-cli.exe").read_bytes())
    boot_before = tree(boot)

    # B. Default one-line install: adopt the CLI store in place, leave the bootstrap root side by side.
    out = run([cli, "install", "--yes", "--no-modify-path"])
    backup = store / "legacy" / "cli-store-0.1.0" / "axiom-cli.exe"
    case("cli_store_adopted_in_place", out["exit_code"] == 0 and (store / "installed.json").is_file()
         and backup.is_file() and sha(backup.read_bytes()) == old_cli
         and sha((store / "bin" / "axiom-cli.exe").read_bytes()) == sha(args.cli.read_bytes())
         and "adopt the 0.1.0 CLI store" in out["stdout_tail"] and "left untouched (side by side)" in out["stdout_tail"]
         and tree(store, skip=INSTALL_DIRS | {"installed.json", "recorded-manifest.json"}) == store_keep,
         run=out)
    case("bootstrap_root_untouched_side_by_side", tree(boot) == boot_before)

    # D. Interrupted adoption of the bootstrap root, then recovery on the next run.
    obstacle = boot / "installed.json.tmp"
    obstacle.mkdir()
    out_fail = run([cli, "install", "--yes", "--no-modify-path", "--adopt", boot])
    obstacle.rmdir()
    mcp_pointer = sha((boot / "mcp-runtime/mcp-runtime/current.json").read_bytes())
    versions_before = sorted(p.name for p in (boot / "installs/ecosystem/versions").iterdir())
    out = run([cli, "install", "--yes", "--no-modify-path", "--adopt", boot])
    case("interrupted_adoption_recovers", out_fail["exit_code"] != 0 and out["exit_code"] == 0
         and (boot / "installed.json").is_file(), interrupted=out_fail, recovered=out)
    # C. Adoption reused the engine ecosystem and the nested MCP runtime: nothing reinstalled.
    user_after = {k: v for k, v in tree(boot).items() if k.startswith(("graph-output/", "instances/"))}
    user_before = {k: v for k, v in boot_before.items() if k.startswith(("graph-output/", "instances/"))}
    case("bootstrap_root_adopted_without_reinstall", out["exit_code"] == 0
         and sha((boot / "mcp-runtime/mcp-runtime/current.json").read_bytes()) == mcp_pointer
         and not (boot / "mcp-runtime" / "current.json").exists()
         and sorted(p.name for p in (boot / "installs/ecosystem/versions").iterdir()) == versions_before
         and "already-installed" in out["stdout_tail"] + out["stderr_tail"]
         and user_after == user_before and len(user_before) == 2, run=out)
    version = run([boot / "bin" / "axiom-cli.exe", "version"])
    case("adopted_root_found_without_env", version["exit_code"] == 0 and ("installed: 0.1.2" in version["stdout_tail"] or ("Axiom Graph Engine" in version["stdout_tail"] and "0.1.2" in version["stdout_tail"])), run=version)

    # E. An unknown layout is reported and never modified.
    odd_before = tree(odd)
    doctor = run([cli, "--json", "doctor"], {"AXIOM_HOME": str(odd)})
    refused = run([cli, "install", "--yes", "--adopt", odd])
    case("unknown_layout_reported_not_modified", '"unknown"' in doctor["_full"] and refused["exit_code"] == 2
         and tree(odd) == odd_before, doctor_exit=doctor["exit_code"], refused=refused)

    subprocess.run(["reg", "delete", r"HKCU\Software\AxiomCliTest", "/f"], capture_output=True)
    def strip(node):
        if isinstance(node, dict):
            return {k: strip(v) for k, v in node.items() if k != "_full"}
        if isinstance(node, list):
            return [strip(v) for v in node]
        return node
    cases[:] = strip(cases)
    doc = {"task_id": "L-006", "lane": "windows-x64",
           "host": {"system": platform.system(), "release": platform.release(), "machine": platform.machine()},
           "cli_sha256": sha(args.cli.read_bytes()), "cases": cases, "passed": all(c["passed"] for c in cases)}
    print(json.dumps(doc, indent=2))
    return 0 if doc["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
