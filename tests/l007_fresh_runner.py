#!/usr/bin/env python3
"""L-007 fresh-runner check: the documented one-liner against the CI-built release candidate.

Run by `.github/workflows/distribution-ci.yml` on fresh hosted runners. The candidate archive is
served from a local HTTP server with the GitHub release-download layout (`<base>/<tag>/<asset>`),
`release/oneline_assets.py` generates `install.ps1` / `install.sh` / `channel.json` for it, and then
the exact documented command runs, followed by `version`, `doctor` (must exit 0), `update`,
`uninstall` and a reinstall. L-013 (ADR-0035) adds the documented update one-liner: with an
installed release it runs the installation's own `axiom-cli update`; with a leftover `AXIOM_*`
variable or with nothing installed it refuses without change; on a 0.1.2 bootstrap layout it makes
the verified install hand-off. No `AXIOM_*` variable is set except, on Windows, the elevated-runner
seam `AXIOM_CLI_TEST_ALLOW_ELEVATED` that hosted runners require.

    python tests/l007_fresh_runner.py --artifacts <dir with axiom-X.Y.Z-<platform>.*> --platform windows-x64 --out <evidence.json>
"""
from __future__ import annotations

import argparse
import functools
import hashlib
import http.server
import importlib.util
import json
import os
import platform as host
import shutil
import subprocess
import sys
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WINDOWS = os.name == "nt"


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--artifacts", type=Path, required=True)
    ap.add_argument("--platform", required=True)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    if args.platform == "macos-x64" and (host.system() != "Darwin" or host.machine() != "x86_64" or os.getuid() == 0):
        ap.error("M-001 requires a native macOS Intel non-root runner")
    archive = next(args.artifacts.rglob(f"axiom-*-{args.platform}.*"))
    version = archive.name[len("axiom-"):].split(f"-{args.platform}")[0]
    tag = f"v{version}"
    srv = Path(os.environ.get("RUNNER_TEMP", "/tmp")) / "oneliner-srv"
    shutil.rmtree(srv, ignore_errors=True)
    (srv / tag).mkdir(parents=True)
    shutil.copy2(archive, srv / tag / archive.name)
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(srv))
    handler.log_message = lambda *a, **k: None
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{server.server_address[1]}"
    spec = importlib.util.spec_from_file_location("oneline", ROOT / "release/oneline_assets.py")
    oneline = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(oneline)
    ext = "zip" if args.platform == "windows-x64" else "tar.gz"
    oneline.build(srv / tag, tag, base, ((args.platform, ext),))

    env = {k: v for k, v in os.environ.items() if not k.upper().startswith("AXIOM")}
    if WINDOWS:
        env["AXIOM_CLI_TEST_ALLOW_ELEVATED"] = "1"
        root = Path(os.environ["LOCALAPPDATA"]) / "Axiom"
        oneliner = ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-c",
                    f"& ([scriptblock]::Create((irm {base}/{tag}/install.ps1))) -Yes"]
        update_oneliner = ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-c",
                           f"& ([scriptblock]::Create((irm {base}/{tag}/update.ps1))) -Yes"]
        legacy_root = Path(os.environ["USERPROFILE"]) / "axiom"
    else:
        data = os.environ.get("XDG_DATA_HOME") or str(Path.home() / ".local" / "share")
        root = Path(data) / "axiom"
        oneliner = ["sh", "-c", f"curl -fsSL {base}/{tag}/install.sh | sh -s -- --yes"]
        update_oneliner = ["sh", "-c", f"curl -fsSL {base}/{tag}/update.sh | sh -s -- --yes"]
        legacy_root = Path.home() / "axiom"
    cli = root / "bin" / ("axm.exe" if WINDOWS else "axm")
    if root.exists() or legacy_root.exists():
        ap.error("requires a fresh runner without an Axiom install or legacy root; existing files preserved")
    axm = root / "bin" / ("axm.exe" if WINDOWS else "axm")  # ADR-0036 short command
    steps = []

    def step(name, argv, expect=0, check=None, extra_env=None):
        proc = subprocess.run([str(a) for a in argv], env={**env, **(extra_env or {})}, capture_output=True, text=True,
                              stdin=subprocess.DEVNULL, timeout=1800)
        ok = proc.returncode == expect and (check is None or check(proc.stdout + proc.stderr))
        steps.append({"step": name, "argv": [str(a) for a in argv], "exit_code": proc.returncode,
                      "expected": expect, "passed": ok,
                      "stdout_tail": proc.stdout[-1500:], "stderr_tail": proc.stderr[-1500:]})
        print(f"[{'ok' if ok else 'FAIL'}] {name} exit={proc.returncode}", flush=True)
        if not ok:
            print(proc.stdout[-3000:], proc.stderr[-3000:], sep="\n", flush=True)
        return ok

    def on_path() -> bool:
        if WINDOWS:
            out = subprocess.run(["reg", "query", r"HKCU\Environment", "/v", "Path"], capture_output=True, text=True).stdout
            return str(root / "bin").lower() in out.lower()
        profiles = [Path.home() / n for n in (".profile", ".bash_profile", ".zprofile")]
        return any(str(root / "bin") in p.read_text() for p in profiles if p.exists())

    step("one-liner install", oneliner)
    steps.append({"step": "PATH announced and added", "passed": on_path()})
    step("version", [cli, "version"], check=lambda t: (f"installed: {version}" in t or ("Axiom Graph Engine" in t and version in t)))
    step("doctor", [cli, "doctor"])
    steps.append({"step": "only four canonical public commands are installed",
                  "passed": {p.stem for p in (root / "bin").iterdir() if p.is_file()} == {"axm", "axsk", "axge", "axmp"}})
    step("axm version", [axm, "version"], check=lambda t: (f"installed: {version}" in t or ("Axiom Graph Engine" in t and version in t)) and ("Axiom CLI" in t and version in t) and (f"Skills: {version}" in t or ("Axiom Skills" in t and version in t)))
    step("axm doctor", [axm, "doctor"])
    step("update (channel offers the installed release)", [cli, "update", "--yes", "--channel", f"{base}/{tag}/channel.json"],
         check=lambda t: "up to date" in t)
    installed_json = root / "installed.json"
    step("update one-liner with an installed release (runs its axiom-cli update)", update_oneliner,
         check=lambda t: "running its axm update" in t and "up to date" in t)
    before = sha(installed_json)
    step("update one-liner refuses a leftover AXIOM_ENGINE_BIN", update_oneliner, expect=1,
         check=lambda t: "leftover variables" in t and "AXIOM_ENGINE_BIN" in t,
         extra_env={"AXIOM_ENGINE_BIN": str(root / "elsewhere" / "axiom")})
    steps.append({"step": "leftover refusal changed nothing", "passed": sha(installed_json) == before})
    step("uninstall through axm", [axm, "uninstall", "--yes"])
    steps.append({"step": "uninstall removed canonical bin commands", "passed": not cli.exists() and not axm.exists()})
    steps.append({"step": "PATH entry removed by uninstall", "passed": not on_path()})
    step("update one-liner with nothing installed refuses", update_oneliner, expect=1,
         check=lambda t: "not installed" in t)
    steps.append({"step": "nothing-installed refusal changed nothing", "passed": not installed_json.exists()})
    current = legacy_root / "installs" / "ecosystem" / "current"
    current.parent.mkdir(parents=True, exist_ok=True)
    current.write_text("{}", encoding="utf-8")
    step("update one-liner on a 0.1.2 bootstrap layout takes the verified install hand-off", update_oneliner,
         check=lambda t: "bootstrap root" in t and "installed: generation" in t)
    steps.append({"step": "legacy update recorded installed.json", "passed": installed_json.is_file()})
    shutil.rmtree(legacy_root, ignore_errors=True)
    step("reinstall with the same one-liner", oneliner)
    step("doctor after reinstall", [cli, "doctor"])
    server.shutdown()
    run_url = "{}/{}/actions/runs/{}".format(os.environ.get("GITHUB_SERVER_URL", ""), os.environ.get("GITHUB_REPOSITORY", ""),
                                              os.environ.get("GITHUB_RUN_ID", "local"))
    doc = {"task_id": "M-001" if args.platform == "macos-x64" else "L-007",
           "tasks": ["M-001"] if args.platform == "macos-x64" else ["L-007", "L-013", "L-016"],
           "execution": "native", "asset_source": "candidate", "clean_install_preflight": True,
           "lane": args.platform, "run_url": run_url, "source_revision": os.environ.get("GITHUB_SHA"),
           "host": {"system": host.system(), "release": host.release(), "machine": host.machine()},
           "assets": {p.name: sha(p) for p in sorted((srv / tag).iterdir())},
           "axiom_env_set": sorted(k for k in env if k.upper().startswith("AXIOM")),
           "steps": steps, "passed": all(s["passed"] for s in steps)}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(doc, indent=2), encoding="utf-8")
    return 0 if doc["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
