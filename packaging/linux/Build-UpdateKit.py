#!/usr/bin/env python3
"""Package a pinned, unsigned local update kit for Linux x64."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile


ROOT = Path(__file__).resolve().parents[2]


def sha(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def build(args: argparse.Namespace) -> dict:
    if args.output.exists():
        raise ValueError("kit output already exists")
    manifest = json.loads(args.candidate_files.read_text())
    if manifest.get("task_id") != "K-405" or manifest.get("candidate") is not True:
        raise ValueError("source candidate identity differs")
    if sha(args.candidate_archive) != args.candidate_sha256:
        raise ValueError("candidate archive changed")
    if sha(args.candidate_files) != args.candidate_files_sha256:
        raise ValueError("candidate file manifest changed")
    if sha(args.runtime_input) != manifest["inputs"]["runtime_input_sha256"]:
        raise ValueError("candidate runtime receipt changed")
    output = args.output
    release = output / "release"
    release.mkdir(parents=True)
    rows = {row["path"]: row for row in manifest["files"]}
    observed: set[str] = set()
    with tarfile.open(args.candidate_archive, "r:gz") as archive:
        for member in archive:
            name = PurePosixPath(member.name)
            if (
                not member.isfile()
                or name.is_absolute()
                or ".." in name.parts
                or member.name in observed
                or member.name not in rows
            ):
                raise ValueError("candidate archive has an unsafe member")
            observed.add(member.name)
            stream = archive.extractfile(member)
            if stream is None:
                raise ValueError("candidate member is unreadable")
            data = stream.read()
            if (
                hashlib.sha256(data).hexdigest() != rows[member.name]["sha256"]
                or len(data) != rows[member.name]["size_bytes"]
            ):
                raise ValueError("candidate member changed: " + member.name)
            path = release.joinpath(*name.parts)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            path.chmod(
                0o755 if path.name in ("axiom-cli", "axiom", "axiom-graphd") else 0o644
            )
    if observed != set(rows):
        raise ValueError("candidate archive membership differs")
    inputs = {
        "candidate-files.json": args.candidate_files,
        "runtime-input.json": args.runtime_input,
        "Update-Distribution.py": ROOT / "packaging/linux/Update-Distribution.py",
        "Install-AxiomCli.sh": ROOT / "installers/linux/Install-AxiomCli.sh",
        "AxiomCli.Linux.Common.sh": ROOT / "installers/linux/AxiomCli.Linux.Common.sh",
    }
    for name, path in inputs.items():
        if path.is_symlink() or not path.is_file():
            raise ValueError("kit input missing or linked: " + name)
        shutil.copyfile(path, output / name)
    release_version = json.loads((release / "release-set.json").read_text())[
        "release_version"
    ]
    result = subprocess.run(
        [
            "sh",
            str(ROOT / "packaging/linux/Build-ReleaseSet.sh"),
            "--out-dir",
            str(output / "cli-release"),
            "--cli-binary",
            str(release / "axiom-cli"),
            "--release-version",
            release_version,
            "--json",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode:
        raise ValueError(
            "Linux CLI release-set builder refused: " + result.stderr[-500:]
        )
    for name in ("cli-release/release-set.json", "cli-release/axiom-cli"):
        inputs[name] = output / name
    kit = {
        "task_id": "K-406",
        "lane": "container-linux-x64",
        "candidate_archive_sha256": args.candidate_sha256,
        "published": False,
        "certified": False,
        "files": {name: sha(output / name) for name in sorted(inputs)},
    }
    (output / "kit-manifest.json").write_text(
        json.dumps(kit, indent=2, sort_keys=True) + "\n"
    )
    archive_path = (
        output / f"axiom-{release_version}-container-linux-x64-update-kit.tar.gz"
    )
    paths = sorted(
        path for path in output.rglob("*") if path.is_file() and path != archive_path
    )
    with archive_path.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as zipped:
            with tarfile.open(
                fileobj=zipped, mode="w", format=tarfile.PAX_FORMAT
            ) as archive:
                for path in paths:
                    relative = path.relative_to(output).as_posix()
                    data = path.read_bytes()
                    info = tarfile.TarInfo(relative)
                    info.uid = info.gid = info.mtime = 0
                    info.uname = info.gname = ""
                    info.mode = (
                        0o755
                        if path.name in ("axiom-cli", "axiom", "axiom-graphd")
                        else 0o644
                    )
                    info.size = len(data)
                    archive.addfile(info, io.BytesIO(data))
    return {
        "status": "candidate",
        "published": False,
        "archive_sha256": sha(archive_path),
        "kit_manifest_sha256": sha(output / "kit-manifest.json"),
        "files": len(paths),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate-archive", type=Path, required=True)
    parser.add_argument("--candidate-sha256", required=True)
    parser.add_argument("--candidate-files", type=Path, required=True)
    parser.add_argument("--candidate-files-sha256", required=True)
    parser.add_argument("--runtime-input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(build(args), sort_keys=True))
        return 0
    except (OSError, ValueError, KeyError, tarfile.TarError) as error:
        parser.exit(2, f"update kit refused: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
