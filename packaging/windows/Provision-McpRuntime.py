"""Provision an owned Windows MCP runtime from exact offline artifacts.

Invoke with the pinned Python embeddable package through Provision-McpRuntime.ps1.
No system Python or prepared development environment is used by the installed runtime.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

PYTHON_SHA256 = "d1f04d990aee1253d8569e8e5104e30fa9f5fa830899f14843448872d936a2cf"
PIP_SHA256 = "382ff9f685ee3bc25864f820aa50505825f10f5458ffff07e30a6d96e5715cab"


def sha(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def checked(path: Path, expected: str) -> Path:
    if not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise ValueError("digest is not a lowercase SHA-256")
    if path.is_symlink() or not path.is_file() or sha(path) != expected:
        raise ValueError("input is missing, linked, or differs from its declared SHA-256")
    return path.resolve()


def atomic_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=".axiom-", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as out:
            json.dump(value, out, sort_keys=True)
            out.write("\n")
            out.flush()
            os.fsync(out.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def read_json(path: Path) -> dict | None:
    if path.is_symlink():
        raise ValueError("runtime record is linked")
    if not path.exists():
        return None
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError("runtime record is not an object")
    return value


def safe_root(root: Path) -> Path:
    if not root.is_absolute() or root.is_symlink() or ";" in str(root):
        raise ValueError("runtime root must be an absolute non-linked path")
    runtime = root.resolve() / "mcp-runtime"
    if runtime.is_symlink() or (runtime / "versions").is_symlink():
        raise ValueError("runtime path is linked")
    return runtime


def safe_remove(version: Path, versions: Path) -> None:
    resolved = version.resolve()
    if resolved.parent != versions.resolve() or not re.fullmatch(r"[0-9a-f]{24}", version.name):
        raise ValueError("owned version path escapes runtime root")
    shutil.rmtree(version)


def contents(version: Path) -> dict[str, str]:
    found = {}
    for path in sorted(version.rglob("*")):
        if path.is_symlink():
            raise ValueError("runtime contains a linked path")
        # Python may add bytecode while an installed command runs later. These
        # files are derived cache inside the owned environment, not user input.
        if path.is_file() and path.name != "owner.json" and "__pycache__" not in path.parts:
            found[path.relative_to(version).as_posix()] = sha(path)
    return found


def owned_version(root: Path, key: str, *, complete: bool = True) -> Path:
    if not re.fullmatch(r"[0-9a-f]{24}", key):
        raise ValueError("invalid runtime generation")
    path = root / "versions" / key
    owner = read_json(path / "owner.json")
    if path.is_symlink() or owner is None or owner.get("owner") != "axiom-cli" or owner.get("generation") != key:
        raise ValueError("runtime generation is not owned")
    if complete and (not isinstance(owner.get("files"), dict) or owner["files"] != contents(path)):
        raise ValueError("owned runtime bytes changed or gained files")
    return path


def pointer(root: Path, name: str = "current.json") -> dict | None:
    value = read_json(root / name)
    if value is None:
        return None
    version = owned_version(root, value.get("generation", ""), complete=False)
    bin_dir = version / "venv" / "Scripts"
    launcher_name = "axmp.exe" if (bin_dir / "axmp.exe").exists() else "axiom-mcp.exe"
    for file, field in (("python.exe", "python_sha256"), (launcher_name, "executable_sha256")):
        target = bin_dir / file
        if not target.is_file() or target.is_symlink() or sha(target) != value.get(field):
            raise ValueError("active runtime executable changed")
    return value


def safe_zip(archive: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive) as zipped:
        members = zipped.infolist()
        if not members or len({row.filename for row in members}) != len(members):
            raise ValueError("runtime archive members are missing or duplicated")
        for row in members:
            name = row.filename
            if (row.is_dir() or name.startswith("/") or "\\" in name or ":" in name
                    or any(part in ("", ".", "..") for part in name.split("/"))):
                raise ValueError("runtime archive contains unsafe member")
            target = destination / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(zipped.read(row))


def unpack_inputs(archive: Path, destination: Path, source_revision: str) -> dict:
    destination.mkdir(parents=True)
    with tarfile.open(archive) as tar:
        members = tar.getmembers()
        names = {row.name for row in members}
        if not 4 <= len(members) <= 2048 or len(names) != len(members) or any(
            not row.isfile() or row.name.startswith("/") or "\\" in row.name
            or ".." in row.name.split("/") for row in members
        ):
            raise ValueError("MCP bundle contains unsafe or missing members")
        tar.extractall(destination, filter="data")
    manifest = read_json(destination / "input-manifest.json")
    if (manifest is None or manifest.get("source_revision") != source_revision
            or manifest.get("platform") != "windows-x64" or manifest.get("python") != "3.13"):
        raise ValueError("MCP input identity mismatch")
    wheels = manifest.get("wheels", [])
    expected = {"input-manifest.json", "windows-x64-py313-requirements.txt",
                manifest.get("owner_wheel", {}).get("name", "")}
    for row in wheels:
        expected.add("wheelhouse/" + row["name"])
        checked(destination / "wheelhouse" / row["name"], row["sha256"])
    if names != expected:
        raise ValueError("MCP input bundle membership mismatch")
    checked(destination / manifest["owner_wheel"]["name"], manifest["owner_wheel"]["sha256"])
    checked(destination / "windows-x64-py313-requirements.txt", manifest["requirements_sha256"])
    return manifest


def command(argv: list[str], env: dict[str, str]) -> str:
    result = subprocess.run(argv, env=env, capture_output=True, text=True, encoding="utf-8")
    if result.returncode:
        raise ValueError(f"{Path(argv[0]).name} exited {result.returncode}: {(result.stderr or result.stdout)[-300:]}")
    return result.stdout


def report(root: Path, current: dict, status: str) -> dict:
    version = owned_version(root, current["generation"])
    return {"status": status, "generation": current["generation"],
            "version": current["version"], "python_sha256": current["python_sha256"],
            "executable_sha256": current["executable_sha256"],
            "bundle_sha256": current["inputs_sha256"], "runtime_sha256": current["runtime_sha256"],
            "source_revision": current["source_revision"],
            "runtime_bin": str((version / "venv" / "Scripts").resolve()), "certified": False}


def provision(args: argparse.Namespace) -> dict:
    root = safe_root(args.root)
    if not re.fullmatch(r"[0-9a-f]{40}", args.source_revision):
        raise ValueError("source revision is not immutable")
    if args.runtime_sha256 != PYTHON_SHA256 or args.pip_wheel_sha256 != PIP_SHA256:
        raise ValueError("runtime or pip input differs from the pinned Windows policy")
    inputs = {name: checked(getattr(args, name), getattr(args, name + "_sha256"))
              for name in ("runtime", "pip_wheel", "inputs")}
    identity = {name + "_sha256": getattr(args, name + "_sha256")
                for name in ("runtime", "pip_wheel", "inputs")}
    key = hashlib.sha256(json.dumps([args.version, args.source_revision, identity], sort_keys=True).encode()).hexdigest()[:24]
    old = pointer(root)
    if old and old["generation"] == key:
        return report(root, old, "already_current")
    versions = root / "versions"
    versions.mkdir(parents=True, exist_ok=True)
    version = versions / key
    if version.exists():
        raise ValueError("target runtime generation already exists; refusing to overwrite it")
    version.mkdir()
    atomic_json(version / "owner.json", {"owner": "axiom-cli", "generation": key, "creating": True})
    try:
        bin_dir = version / "venv" / "Scripts"
        safe_zip(inputs["runtime"], bin_dir)
        site = bin_dir / "Lib" / "site-packages"
        site.mkdir(parents=True)
        (bin_dir / "python313._pth").write_text(
            "python313.zip\n.\nLib/site-packages\nLib/site-packages/win32\n"
            "Lib/site-packages/win32/lib\nLib/site-packages/pythonwin\n",
            encoding="utf-8", newline="\n",
        )
        safe_zip(inputs["pip_wheel"], site)
        package_dir = version / "packages"
        manifest = unpack_inputs(inputs["inputs"], package_dir, args.source_revision)
        python = bin_dir / "python.exe"
        env = {"SystemRoot": os.environ.get("SystemRoot", r"C:\Windows"),
               "PATH": str(bin_dir) + os.pathsep + os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32"),
               "PYTHONNOUSERSITE": "1", "PYTHONDONTWRITEBYTECODE": "1", "PYTHONPATH": "", "PIP_NO_INDEX": "1",
               "PIP_CONFIG_FILE": os.devnull, "TEMP": os.environ.get("TEMP", str(root)),
               "TMP": os.environ.get("TMP", str(root))}
        observed = command([str(python), "-c", "import sys;print('%d.%d.%d' % sys.version_info[:3])"], env).strip()
        if observed != "3.13.15":
            raise ValueError("runtime interpreter is unsupported: " + observed)
        combined = package_dir / "combined-requirements.txt"
        combined.write_bytes((package_dir / "windows-x64-py313-requirements.txt").read_bytes()
                             + f'axiom-mcp=={args.version} --hash=sha256:{manifest["owner_wheel"]["sha256"]}\n'.encode())
        command([str(python), "-m", "pip", "install", "--no-index", "--no-compile",
                 "--target", str(site), "--find-links", str(package_dir / "wheelhouse"),
                 "--find-links", str(package_dir), "--require-hashes", "-r", str(combined)], env)
        launcher_name = "axmp.exe" if (site / "bin" / "axmp.exe").exists() else "axiom-mcp.exe"
        for name in (launcher_name,):
            launcher = site / "bin" / name
            if not launcher.is_file():
                raise ValueError("installed MCP entrypoint is missing")
            shutil.copyfile(launcher, bin_dir / name)
        health = command([str(bin_dir / launcher_name), "--help"], env)
        if "doctor" not in health or "version" not in health:
            raise ValueError("installed MCP entrypoint failed health check")
        if args.interrupt_before_activate:
            raise InterruptedError("injected interruption before runtime activation")
        new = {"generation": key, "version": args.version, "source_revision": args.source_revision,
               "python_sha256": sha(python), "executable_sha256": sha(bin_dir / launcher_name),
               "runtime_sha256": identity["runtime_sha256"], "pip_wheel_sha256": identity["pip_wheel_sha256"],
               "inputs_sha256": identity["inputs_sha256"], "owner_wheel_sha256": manifest["owner_wheel"]["sha256"]}
        atomic_json(version / "owner.json", {"owner": "axiom-cli", "generation": key, "files": contents(version)})
        if old:
            atomic_json(root / "previous.json", old)
        atomic_json(root / "current.json", new)
        return report(root, new, "provisioned")
    except Exception:
        if version.is_dir() and read_json(version / "owner.json") is not None:
            safe_remove(version, versions)
        raise


def rollback(root: Path) -> dict:
    previous = pointer(root, "previous.json")
    if previous is None:
        raise ValueError("no previous owned runtime")
    owned_version(root, previous["generation"])
    current = pointer(root)
    if current:
        atomic_json(root / "previous.json", current)
    atomic_json(root / "current.json", previous)
    return report(root, previous, "rolled_back")


def remove(root: Path) -> dict:
    if root.is_symlink() or (root / "versions").is_symlink():
        raise ValueError("runtime root is linked")
    versions = root / "versions"
    entries = list(versions.iterdir()) if versions.exists() else []
    for entry in entries:
        owned_version(root, entry.name)
    for entry in entries:
        safe_remove(entry, versions)
    for name in ("current.json", "previous.json"):
        path = root / name
        if path.exists():
            path.unlink()
    if versions.exists():
        versions.rmdir()
    return {"status": "removed", "owned_generations": sorted(x.name for x in entries),
            "user_data_preserved": True}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    install = sub.add_parser("provision")
    install.add_argument("--root", type=Path, required=True)
    install.add_argument("--version", required=True)
    install.add_argument("--source-revision", required=True)
    for name in ("runtime", "pip_wheel", "inputs"):
        install.add_argument("--" + name.replace("_", "-"), type=Path, required=True)
        install.add_argument("--" + name.replace("_", "-") + "-sha256", required=True)
    install.add_argument("--interrupt-before-activate", action="store_true")
    for name in ("status", "rollback", "remove"):
        sub.add_parser(name).add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    try:
        root = safe_root(args.root)
        if args.action == "provision":
            result = provision(args)
        elif args.action == "rollback":
            result = rollback(root)
        elif args.action == "remove":
            result = remove(root)
        else:
            current = pointer(root)
            if current is None:
                raise ValueError("runtime is not installed")
            result = report(root, current, "current")
        print(json.dumps(result, sort_keys=True))
        return 0
    except (ValueError, OSError, InterruptedError, json.JSONDecodeError,
            zipfile.BadZipFile, tarfile.TarError) as error:
        print(json.dumps({"status": "refused", "reason": str(error)}, sort_keys=True), file=sys.stderr)
        return 9


if __name__ == "__main__":
    raise SystemExit(main())
