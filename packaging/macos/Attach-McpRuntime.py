#!/usr/bin/env python3
"""Attach the pinned runtime inputs to an unpublished Mac candidate set."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tempfile

ROOT = Path(__file__).resolve().parents[2]
FILES = {
    "python_archive": "python.tar.gz",
    "wheelhouse_archive": "wheelhouse.tar.gz",
    "wheel": "",
    "lock": "requirements.txt",
}


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def attach(args: argparse.Namespace) -> dict:
    match = re.fullmatch(
        r"axiom_mcp-([0-9]+\.[0-9]+\.[0-9]+)-py3-none-any\.whl", args.wheel.name
    )
    if match is None:
        raise ValueError("MCP wheel filename must contain a release version")
    version = match.group(1)
    files = dict(FILES, wheel=args.wheel.name)
    release = args.release_set.resolve()
    if not release.is_dir() or not (release / "channel.json").is_file():
        raise ValueError("composed candidate channel is missing")
    channel = json.loads((release / "channel.json").read_text())
    if channel.get("published") is not False:
        raise ValueError("only an unpublished candidate channel may be attached")
    if args.runtime_input.is_symlink() or not args.runtime_input.is_file():
        raise ValueError("K-104 runtime input is missing or linked")
    if not re.fullmatch(r"[0-9a-f]{64}", args.runtime_input_sha256):
        raise ValueError("K-104 runtime input SHA-256 is invalid")
    if digest(args.runtime_input) != args.runtime_input_sha256:
        raise ValueError("K-104 runtime input differs from its pin")
    data = json.loads(args.runtime_input.read_text())
    if data.get("task_id") != "K-104" or data.get("lane") != "macos-x64":
        raise ValueError("K-104 Mac x64 runtime input is required")
    expected = {
        "python_archive": data["runtime"]["sha256"],
        "wheelhouse_archive": data["wheelhouse"]["sha256"],
        "wheel": data["mcp"]["wheel_sha256"],
        "lock": data["mcp"]["dependency_lock_sha256"],
    }
    sources = {key: getattr(args, key) for key in FILES}
    for key, path in sources.items():
        if not re.fullmatch(r"[0-9a-f]{64}", expected[key]):
            raise ValueError(f"{key} digest is invalid")
        if path.is_symlink() or not path.is_file() or digest(path) != expected[key]:
            raise ValueError(f"{key} differs from its K-104 pin")
    if digest(release / files["wheel"]) != expected["wheel"]:
        raise ValueError("channel MCP wheel differs from runtime input")
    target = release / "runtime"
    if target.exists() or target.is_symlink():
        raise ValueError("runtime candidate output already exists")
    scripts = {
        "provision.py": ROOT / "packaging/macos/Provision-McpRuntime.py",
        "install.py": ROOT / "installers/macos/Install-Distribution.py",
        "bootstrap.sh": ROOT / "installers/macos/Install-Distribution.sh",
        "entrypoints.sh": ROOT / "installers/macos/Manage-Entrypoints.sh",
    }
    with tempfile.TemporaryDirectory(prefix=".runtime-candidate-", dir=release) as name:
        stage = Path(name) / "runtime"
        stage.mkdir()
        for key, filename in files.items():
            shutil.copyfile(sources[key], stage / filename)
            if digest(stage / filename) != expected[key]:
                raise ValueError("staged runtime input changed")
        for filename, source in scripts.items():
            shutil.copyfile(source, stage / filename)
        manifest = {
            "schema_version": 1,
            "task_id": "K-105",
            "lane": "macos-x64",
            "published": False,
            "python_relative": "cpython-3.13.15-macos-x86_64-none/bin/python3.13",
            "python_sha256": expected["python_archive"],
            "installer_sha256": digest(stage / "install.py"),
            "python_version": data["runtime"]["version"],
            "mcp_version": version,
            "mcp_revision": data["mcp"]["source_revision"],
            "files": {
                filename: digest(stage / filename)
                for filename in [*files.values(), *scripts]
            },
        }
        (stage / "manifest.json").write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n"
        )
        os.replace(stage, target)
    return {
        "status": "candidate",
        "runtime_manifest_sha256": digest(target / "manifest.json"),
        "runtime_archive_sha256": expected["python_archive"],
        "published": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-set", required=True, type=Path)
    parser.add_argument("--runtime-input", required=True, type=Path)
    parser.add_argument("--runtime-input-sha256", required=True)
    for name in FILES:
        parser.add_argument("--" + name.replace("_", "-"), required=True, type=Path)
    args = parser.parse_args()
    try:
        print(json.dumps(attach(args), sort_keys=True))
        return 0
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.exit(2, f"runtime attachment refused: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
