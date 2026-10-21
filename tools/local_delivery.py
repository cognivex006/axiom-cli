"""Run owner checks/builds on this machine; never install tools or publish releases."""

import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def native_lane():
    if platform.system() == "Linux":
        if "microsoft" in platform.release().lower() or any(
            os.environ.get(name) for name in ("WSL_INTEROP", "WSL_DISTRO_NAME")
        ):
            raise ValueError("WSL results cannot be recorded as native Linux evidence")
        if any(
            Path(path).exists() for path in ("/.dockerenv", "/run/.containerenv")
        ) or os.environ.get("container"):
            raise ValueError("container results cannot be recorded as native host evidence")
    machine = platform.machine().lower()
    if machine not in ("amd64", "x86_64"):
        raise ValueError("only native x64 build machines are currently supported")
    return {"Windows": "windows-x64", "Linux": "linux-x64", "Darwin": "macos-x64"}[
        platform.system()
    ]


def verify_assets(directory, revision):
    """Fail closed on partial inventories, unsafe names, links and source drift."""
    if not directory.is_dir() or directory.is_symlink():
        raise ValueError("asset directory must be a real directory")
    seen = set()
    for line in (directory / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
        parts = line.split("  ", 1)
        if len(parts) != 2:
            raise ValueError("invalid checksum inventory")
        digest, name = parts
        if (
            len(digest) != 64
            or any(c not in "0123456789abcdef" for c in digest)
            or name in seen
            or name in ("", ".", "..", "SHA256SUMS")
            or any(c in name for c in "/\\:")
        ):
            raise ValueError("unsafe or duplicate inventory entry")
        path = directory / name
        if path.is_symlink() or not path.is_file():
            raise ValueError("missing or linked release asset")
        with path.open("rb") as handle:
            observed = hashlib.file_digest(handle, "sha256").hexdigest()
        if observed != digest:
            raise ValueError("release asset checksum mismatch")
        seen.add(name)
    if not seen or {p.name for p in directory.iterdir()} != seen | {"SHA256SUMS"}:
        raise ValueError("incomplete release inventory")
    if (directory / "SOURCE-REVISION.txt").read_text().strip() != revision:
        raise ValueError("release asset source mismatch")


def execute(stage, output):
    config = json.loads((ROOT / ".delivery/local-build.json").read_text(encoding="utf-8"))
    expected = "https://github.com/cognivex006/" + config["repository"]
    if git("remote", "get-url", "origin").removesuffix(".git") != expected:
        raise ValueError("canonical remote mismatch")
    revision = git("rev-parse", "HEAD")
    dirty = bool(git("status", "--porcelain"))
    if stage == "build" and dirty:
        raise ValueError("build requires clean committed source")
    output = output.resolve()
    if output == ROOT or ROOT in output.parents or output.exists():
        raise ValueError("new output directory outside source required")
    commands = config[stage]
    if not commands:
        raise ValueError("no software build is declared for this repository")
    lane = native_lane()
    output.mkdir(parents=True)
    env = dict(os.environ, PYTHONUTF8="1", PYTHONDONTWRITEBYTECODE="1")
    env["AXIOM_BUILD_REVISION"] = revision
    if config["repository"] == "axiom-cli" and stage == "check":
        env["AXIOM_CLI_INSTALL_ROOT"] = str(output / "private-install")
    report = dict(
        repository=config["repository"],
        source_revision=revision,
        dirty=dirty,
        platform=lane,
        execution="native",
        stage=stage,
        publication="not_published",
        commands=[],
        passed=False,
    )
    code = 0
    try:
        for index, command in enumerate(commands):
            argv = [arg.format(out=str(output / "artifacts"), source=revision) for arg in command]
            if argv[0] == "python":
                argv[0] = sys.executable
            log = output / f"{index:02d}.log"
            started = time.monotonic()
            with log.open("wb") as handle:
                result = subprocess.run(
                    argv, cwd=ROOT, env=env, stdout=handle, stderr=subprocess.STDOUT
                )
            with log.open("rb") as handle:
                checksum = hashlib.file_digest(handle, "sha256").hexdigest()
            report["commands"].append(
                dict(
                    argv=argv,
                    template=command,
                    exit_code=result.returncode,
                    duration_seconds=round(time.monotonic() - started, 3),
                    log=log.name,
                    sha256=checksum,
                )
            )
            print(f"{config['repository']}: {command} -> {result.returncode}", flush=True)
            if result.returncode:
                code = result.returncode
                break
        report["passed"] = code == 0
    finally:
        (output / "local-build-report.json").write_text(
            json.dumps(report, indent=2) + "\n", encoding="utf-8", newline="\n"
        )
    return code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("check", "build", "verify-assets"))
    parser.add_argument("--out", type=Path)
    parser.add_argument("--assets", type=Path)
    parser.add_argument("--source-revision")
    args = parser.parse_args()
    if args.stage == "verify-assets":
        if not args.assets or not args.source_revision:
            parser.error("--assets and --source-revision required")
        if len(args.source_revision) != 40 or any(
            c not in "0123456789abcdef" for c in args.source_revision
        ):
            raise ValueError("immutable source revision required")
        verify_assets(args.assets, args.source_revision)
        print("OK exact asset inventory and source binding; native acceptance is separate")
        return 0
    if not args.out:
        parser.error("--out required")
    return execute(args.stage, args.out)


if __name__ == "__main__":
    raise SystemExit(main())
