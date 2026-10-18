#!/usr/bin/env python3
"""Candidate-only, per-user MCP runtime transaction for Linux x64.

All payloads are local, hash-pinned inputs. The caller supplies the Python
archive; this tool makes no choice about where that archive was obtained.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def checked(path: Path, expected: str) -> Path:
    if not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise ValueError("digest is not a lowercase SHA-256")
    if path.is_symlink() or not path.is_file() or digest(path) != expected:
        raise ValueError(
            "input is missing, linked, or differs from its declared SHA-256: "
            + str(path)
        )
    return path.resolve()


def atomic_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=".axiom-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(value, stream, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def read_json(path: Path) -> dict | None:
    if not path.is_file() or path.is_symlink():
        return None
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError("runtime pointer is not an object")
    return value


def unpack(archive: Path, destination: Path) -> None:
    destination.mkdir(parents=True)
    with tarfile.open(archive, "r:gz") as source:
        members = source.getmembers()
        for row in members:
            name = PurePosixPath(row.name)
            if (
                name.is_absolute()
                or not row.name
                or ".." in name.parts
                or row.isdev()
                or row.isfifo()
                or row.issym()
                or row.islnk()
            ):
                raise ValueError("runtime archive contains an unsafe member")
            if not row.isfile() and not row.isdir():
                raise ValueError("runtime archive contains an unsupported member")
        for row in members:
            target = destination.joinpath(*PurePosixPath(row.name).parts)
            if row.isdir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with source.extractfile(row) as input_file, target.open("wb") as output:
                    shutil.copyfileobj(input_file, output)
                target.chmod(row.mode & 0o755)


def command(argv: list[str], env: dict[str, str]) -> str:
    result = subprocess.run(argv, env=env, capture_output=True, text=True, check=False)
    if result.returncode:
        raise ValueError(
            f"{Path(argv[0]).name} exited {result.returncode}: {(result.stderr or result.stdout)[-400:]}"
        )
    return result.stdout


def verify_entrypoint(launcher: Path, env: dict[str, str], version: str, revision: str) -> None:
    output = command(
        [str(launcher), "version", "--json"],
        {**env, "AXIOM_MCP_BUILD_REVISION": revision},
    )
    try:
        identity = json.loads(output)
    except ValueError as error:
        raise ValueError("installed entrypoint version response is not valid JSON") from error
    if (
        not isinstance(identity, dict)
        or identity.get("component") != "axiom-mcp"
        or identity.get("version") != version
        or identity.get("build_revision") != revision
    ):
        raise ValueError("installed entrypoint reports the wrong version or source")


def owned_version(root: Path, key: str) -> Path:
    if not re.fullmatch(r"[0-9a-f]{24}", key):
        raise ValueError("invalid runtime generation")
    version = root / "versions" / key
    if version.is_symlink() or not (version / "owner.json").is_file():
        raise ValueError("runtime generation is not owned")
    return version


def pointer(root: Path) -> dict | None:
    value = read_json(root / "current.json")
    if value is not None:
        owned_version(root, value.get("generation", ""))
    return value


def report(root: Path, current: dict, state: str) -> dict:
    version = owned_version(root, current["generation"])
    runtime_bin = version / "venv" / "bin"
    executable = runtime_bin / ("axmp" if (runtime_bin / "axmp").exists() else "axiom-mcp")
    python = version / "venv" / "bin" / "python"
    if not executable.is_file() or not python.is_file():
        raise ValueError("owned runtime executable is missing")
    if (
        digest(executable) != current["executable_sha256"]
        or digest(python) != current["python_sha256"]
    ):
        raise ValueError("owned runtime executable digest changed")
    return {
        "status": state,
        "generation": current["generation"],
        "version": current["version"],
        "python": str(python.resolve()),
        "executable": str(executable.resolve()),
        "executable_sha256": digest(executable),
        "python_sha256": digest(python),
        "wheel_sha256": current["wheel_sha256"],
        "runtime_sha256": current["runtime_sha256"],
        "wheelhouse_sha256": current["wheelhouse_sha256"],
        "lock_sha256": current["lock_sha256"],
    }


def provision(args: argparse.Namespace) -> dict:
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise ValueError("native Linux x64 host required")
    root = args.root.resolve()
    if args.root.is_symlink() or not re.fullmatch(
        r"[A-Za-z0-9][A-Za-z0-9._-]*", args.version
    ):
        raise ValueError("unsafe runtime root or version")
    if not re.fullmatch(r"[0-9a-f]{40}", args.source_revision):
        raise ValueError("source revision must be an immutable 40-hex commit")
    inputs = {
        name: checked(getattr(args, name), getattr(args, name + "_sha256"))
        for name in ("runtime", "wheelhouse", "wheel", "lock")
    }
    identity = {
        name + "_sha256": getattr(args, name + "_sha256")
        for name in ("runtime", "wheelhouse", "wheel", "lock")
    }
    key = hashlib.sha256(
        json.dumps([args.version, identity], sort_keys=True).encode()
    ).hexdigest()[:24]
    old = pointer(root)
    if old and old["generation"] == key:
        return report(root, old, "already_current")
    versions = root / "versions"
    if versions.is_symlink():
        raise ValueError("versions directory is a symlink")
    versions.mkdir(parents=True, exist_ok=True)
    version = versions / key
    if version.exists():
        owned_version(root, key)
        shutil.rmtree(version)
    version.mkdir()
    atomic_json(version / "owner.json", {"owner": "axiom-cli", "generation": key})
    try:
        unpack(inputs["runtime"], version / "runtime")
        candidates = list((version / "runtime").rglob("bin/python3.13"))
        if len(candidates) != 1:
            raise ValueError(
                "runtime archive must contain exactly one Python 3.13 executable"
            )
        runtime_python = candidates[0]
        runtime_python.chmod(0o755)
        env = {
            "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
            "HOME": str(root),
            "TMPDIR": str(root),
            "LANG": "C.UTF-8",
            "PIP_CONFIG_FILE": os.devnull,
            "PYTHONNOUSERSITE": "1",
        }
        observed = command(
            [
                str(runtime_python),
                "-c",
                "import sys; print('%d.%d' % sys.version_info[:2])",
            ],
            env,
        ).strip()
        if observed != "3.13":
            raise ValueError("runtime interpreter is unsupported: " + observed)
        command(
            [str(runtime_python), "-m", "venv", "--copies", str(version / "venv")], env
        )
        unpack(inputs["wheelhouse"], version / "packages")
        wheels = version / "packages" / "wheelhouse"
        if not wheels.is_dir():
            raise ValueError("wheelhouse directory missing")
        staged_wheel = version / inputs["wheel"].name
        shutil.copyfile(inputs["wheel"], staged_wheel)
        staged_lock = version / "requirements.txt"
        shutil.copyfile(inputs["lock"], staged_lock)
        python = version / "venv" / "bin" / "python"
        command(
            [
                str(python),
                "-m",
                "pip",
                "install",
                "--no-index",
                "--find-links",
                str(wheels),
                "--require-hashes",
                "-r",
                str(staged_lock),
            ],
            env,
        )
        command(
            [
                str(python),
                "-m",
                "pip",
                "install",
                "--no-index",
                "--no-deps",
                str(staged_wheel),
            ],
            env,
        )
        runtime_bin = version / "venv" / "bin"
        launcher = runtime_bin / ("axmp" if (runtime_bin / "axmp").exists() else "axiom-mcp")
        verify_entrypoint(launcher, env, args.version, args.source_revision)
        new = {
            "generation": key,
            "version": args.version,
            "executable_sha256": digest(launcher),
            "python_sha256": digest(python),
            **identity,
        }
        atomic_json(version / "record.json", new)
        if args.interrupt_before_activate:
            raise InterruptedError("injected interruption before pointer activation")
        if old:
            atomic_json(root / "previous.json", old)
        atomic_json(root / "current.json", new)
        return report(root, new, "provisioned")
    except Exception:
        if version.is_dir() and (version / "owner.json").is_file():
            shutil.rmtree(version)
        raise


def rollback(root: Path, *, dry_run: bool = False) -> dict:
    previous = read_json(root / "previous.json")
    if previous is None:
        raise ValueError("no previous owned runtime to restore")
    owned_version(root, previous["generation"])
    verified = report(root, previous, "rollback_planned" if dry_run else "rolled_back")
    current = pointer(root)
    if dry_run:
        return verified
    if current:
        atomic_json(root / "previous.json", current)
    atomic_json(root / "current.json", previous)
    return verified


def remove(root: Path) -> dict:
    if root.is_symlink():
        raise ValueError("runtime root is a symlink")
    for name in ("current.json", "previous.json"):
        if (root / name).is_symlink():
            raise ValueError("runtime pointer is a symlink")
    versions = root / "versions"
    removed = []
    if versions.exists():
        if versions.is_symlink():
            raise ValueError("versions directory is a symlink")
        entries = list(versions.iterdir())
        for entry in entries:
            owned_version(root, entry.name)
        for entry in entries:
            shutil.rmtree(entry)
            removed.append(entry.name)
        versions.rmdir()
    for name in ("current.json", "previous.json"):
        path = root / name
        if path.exists():
            path.unlink()
    return {
        "status": "removed",
        "owned_generations": sorted(removed),
        "user_data_preserved": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    install = sub.add_parser("provision")
    install.add_argument("--root", type=Path, required=True)
    install.add_argument("--version", required=True)
    install.add_argument("--source-revision", required=True)
    for name in ("runtime", "wheelhouse", "wheel", "lock"):
        install.add_argument("--" + name, type=Path, required=True)
        install.add_argument("--" + name + "-sha256", required=True)
    install.add_argument("--interrupt-before-activate", action="store_true")
    for name in ("status", "rollback", "remove"):
        action = sub.add_parser(name)
        action.add_argument("--root", type=Path, required=True)
        if name == "rollback":
            action.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    try:
        if args.action == "provision":
            result = provision(args)
        elif args.action == "rollback":
            result = rollback(args.root.resolve(), dry_run=args.dry_run)
        elif args.action == "remove":
            result = remove(args.root.resolve())
        else:
            current = pointer(args.root.resolve())
            if current is None:
                raise ValueError("runtime is not installed")
            result = report(args.root.resolve(), current, "current")
        print(json.dumps(result, sort_keys=True))
        return 0
    except (
        ValueError,
        OSError,
        tarfile.TarError,
        InterruptedError,
        json.JSONDecodeError,
    ) as error:
        print(
            json.dumps({"status": "refused", "reason": str(error)}, sort_keys=True),
            file=sys.stderr,
        )
        return 9


if __name__ == "__main__":
    raise SystemExit(main())
