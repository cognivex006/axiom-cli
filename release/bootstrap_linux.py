"""Fresh per-user Linux/WSL installation: compose existing owner transactions only."""

import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import sys


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def command(argv, env):
    result = subprocess.run(
        [str(x) for x in argv], env=env, capture_output=True, text=True
    )
    if result.returncode:
        raise ValueError((result.stdout + result.stderr)[-1000:])
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--approve-digest")
    args = parser.parse_args()
    if os.getuid() == 0 or args.dry_run == args.apply:
        raise ValueError("non-root user and exactly one execution mode required")
    release = Path(__file__).resolve().parents[1]
    root = args.root.absolute()
    if root.is_symlink() or (root.exists() and any(root.iterdir())):
        raise ValueError(
            "fresh empty per-user root required; existing installations use CLI update"
        )
    inventory = json.loads((release / "DISTRIBUTION-FILES.json").read_bytes())
    for row in inventory["files"]:
        path = release / row["path"]
        if path.is_symlink() or sha(path) != row["sha256"]:
            raise ValueError("distribution bytes changed")
    seal = {
        "action": "fresh-install",
        "root": str(root),
        "inventory_sha256": sha(release / "DISTRIBUTION-FILES.json"),
    }
    digest = hashlib.sha256(
        json.dumps(seal, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()
    manifest = json.loads((release / "runtime/manifest.json").read_bytes())
    version = manifest["version"]
    if not isinstance(version, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ValueError("invalid pinned release version")
    if args.dry_run:
        print(
            json.dumps({"status": "planned", "plan_digest": digest, "version": version})
        )
        return 0
    if args.approve_digest != digest:
        raise ValueError("exact reviewed distribution approval required")
    env = dict(
        os.environ,
        AXIOM_HOME=str(root),
        AXIOM_CLI_INSTALL_ROOT=str(root),
        PYTHONNOUSERSITE="1",
    )
    runtime_args = [
        sys.executable,
        release / "runtime/provision.py",
        "provision",
        "--root",
        root / "mcp-runtime",
        "--version",
        version,
        "--source-revision",
        manifest["mcp_revision"],
    ]
    for key, name in [
        ("runtime", "python.tar.gz"),
        ("wheelhouse", "wheelhouse.tar.gz"),
        ("wheel", f"axiom_mcp-{version}-py3-none-any.whl"),
        ("lock", "requirements.txt"),
    ]:
        runtime_args += [
            "--" + key,
            release / "runtime" / name,
            "--" + key + "-sha256",
            manifest["files"][name],
        ]
    runtime = command(runtime_args, env)
    env["PATH"] = str(Path(runtime["python"]).parent) + os.pathsep + env.get("PATH", "")
    cli = release / "axiom-cli"
    plan = command([cli, "install", "--from", release, "--dry-run", "--json"], env)
    approval = plan.get("plan_digest") or plan.get("details", {}).get("plan_digest")
    if not approval:
        raise ValueError("engine install plan approval missing")
    applied = command(
        [
            cli,
            "install",
            "--from",
            release,
            "--apply",
            "--approve-digest",
            approval,
            "--json",
        ],
        env,
    )
    print(
        json.dumps(
            {
                "status": "installed",
                "version": version,
                "root": str(root),
                "runtime": runtime,
                "engine": applied,
            }
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError) as error:
        print(json.dumps({"status": "refused", "reason": str(error)}), file=sys.stderr)
        raise SystemExit(9)
