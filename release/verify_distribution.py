"""Verify version-pinned native composed payload bytes and launch its real entrypoints."""

import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import subprocess
import tempfile
import zipfile
import email

VERSION = (Path(__file__).resolve().parents[1] / "VERSION").read_text(encoding="utf-8").strip()


def verify(root, output):
    manifest = json.loads((root / "DISTRIBUTION-FILES.json").read_bytes())
    if manifest["version"] != VERSION:
        raise ValueError("distribution version mismatch")
    for row in manifest["files"]:
        path = root / row["path"]
        if hashlib.sha256(path.read_bytes()).hexdigest() != row["sha256"]:
            raise ValueError("distribution hash mismatch")
    suffix = ".exe" if manifest["platform"] == "windows-x64" else ""
    cases = []
    for binary in ["axiom-cli", "axiom", "axiom-graphd"]:
        result = subprocess.run(
            [str(root / (binary + suffix)), "version", "--json"],
            capture_output=True,
            text=True,
            timeout=15,
        )
        if result.returncode:
            raise ValueError("native version command failed")
        body = json.loads(result.stdout)
        version = body.get("version") or body.get("details", {}).get("cli", {}).get(
            "version"
        )
        if version != VERSION:
            raise ValueError("native version differs: " + binary)
        cases.append(
            {
                "id": binary + " version",
                "exit_code": result.returncode,
                "stdout_sha256": hashlib.sha256(result.stdout.encode()).hexdigest(),
            }
        )
    wheel = root / f"axiom_mcp-{VERSION}-py3-none-any.whl"
    with zipfile.ZipFile(wheel) as archive:
        metadata = [x for x in archive.namelist() if x.endswith(".dist-info/METADATA")]
        if (
            len(metadata) != 1
            or email.message_from_bytes(archive.read(metadata[0]))["Version"] != VERSION
        ):
            raise ValueError("MCP wheel version mismatch")
    if (
        json.loads((root / "skills-manifest.json").read_bytes())["component_version"]
        != VERSION
    ):
        raise ValueError("skills version mismatch")
    # Fresh install approval preparation must remain bounded and non-mutating.
    with tempfile.TemporaryDirectory(prefix="axiom-distribution-smoke-") as temporary:
        target = Path(temporary).resolve() / "fresh-root"
        if suffix:
            argv = [
                "pwsh",
                "-NoProfile",
                "-File",
                str(root / "bootstrap_windows.ps1"),
                "-Root",
                str(target),
                "-DryRun",
            ]
        elif manifest["platform"] == "linux-x64":
            import sys

            argv = [
                sys.executable,
                str(root / "runtime/bootstrap.py"),
                "--root",
                str(target),
                "--dry-run",
            ]
        else:
            env = dict(os.environ, HOME=str(Path(temporary).resolve()))
            argv = [
                "sh",
                str(root / "runtime/bootstrap.sh"),
                "install",
                "--release-set",
                str(root),
                "--dry-run",
            ]
        result = subprocess.run(
            argv,
            capture_output=True,
            text=True,
            timeout=120,
            env=env if not suffix and manifest["platform"] == "macos-x64" else None,
        )
        if result.returncode:
            raise ValueError("native install plan refused: " + result.stderr[-1000:])
        if target.exists():
            raise ValueError("dry-run mutated chosen installation root")
        cases.append(
            {
                "id": "native install plan",
                "exit_code": result.returncode,
                "stdout_sha256": hashlib.sha256(result.stdout.encode()).hexdigest(),
            }
        )
        if suffix or manifest["platform"] == "linux-x64":
            body = json.loads(result.stdout)
            approval = body["plan_digest"]
            if suffix:
                apply = argv[:-1] + ["-Apply", "-ApproveDigest", approval]
            else:
                apply = argv[:-1] + ["--apply", "--approve-digest", approval]
            applied = subprocess.run(apply, capture_output=True, text=True, timeout=240)
            if applied.returncode:
                raise ValueError(
                    "native fresh install failed: "
                    + (applied.stdout + applied.stderr)[-1500:]
                )
            cases.append(
                {
                    "id": "native fresh install",
                    "exit_code": applied.returncode,
                    "stdout_sha256": hashlib.sha256(
                        applied.stdout.encode()
                    ).hexdigest(),
                }
            )
    archives = [
        p
        for p in root.parent.glob(f"axiom-{VERSION}-*")
        if p.is_file() and (p.name.endswith(".zip") or p.name.endswith(".tar.gz"))
    ]
    if len(archives) != 1:
        raise ValueError("exactly one assembled archive required")
    archive = archives[0]
    output.write_text(
        json.dumps(
            {
                "platform": manifest["platform"],
                "execution": "native",
                "arch": platform.machine(),
                "source_revision": manifest["source_revision"],
                "version": VERSION,
                "cases": cases,
                "owners": manifest["owners"],
                "archive": archive.name,
                "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
                "owner_input_kind": "ci_candidate" if "candidate_owner_ci" in manifest else "published_release",
                "elevation_test_seam_used": "AXIOM_CLI_TEST_ALLOW_ELEVATED" in os.environ,
                "scope": "Source-bound owner bytes and native install checks; exact executed cases are listed. GUI and non-elevated-user acceptance remain separate.",
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
        newline="\n",
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    verify(args.release.resolve(), args.out.resolve())
