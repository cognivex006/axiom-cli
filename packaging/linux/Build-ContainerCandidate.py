#!/usr/bin/env python3
"""Assemble an unsigned Linux x64 candidate from pinned owner bytes."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
from pathlib import Path, PurePosixPath
import tarfile


ROOT = Path(__file__).resolve().parents[2]


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(path: Path, expected: str) -> None:
    if path.is_symlink() or not path.is_file() or digest(path) != expected:
        raise ValueError(f"missing, linked, or changed input: {path.name}")


def unpack_regular(archive: Path, destination: Path) -> set[str]:
    names: set[str] = set()
    with tarfile.open(archive) as source:
        for member in source:
            name = PurePosixPath(member.name)
            if (
                not member.isfile()
                or name.is_absolute()
                or ".." in name.parts
                or member.name in names
            ):
                raise ValueError("owner archive has an unsafe member")
            names.add(member.name)
            target = destination.joinpath(*name.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            stream = source.extractfile(member)
            if stream is None:
                raise ValueError("owner archive member is unreadable")
            target.write_bytes(stream.read())
    return names


def component(name: str, version: str, revision: str, artifact: Path | None) -> dict:
    rows = []
    if artifact is not None:
        rows.append(
            {
                "platform": "linux-x64",
                "class": "per-user-installer",
                "url": f"https://candidate.invalid/{artifact.name}",
                "sha256": digest(artifact),
                "size_bytes": artifact.stat().st_size,
            }
        )
    return {
        "component": name,
        "status": "declared",
        "version": version,
        "revision": revision,
        "declared_source": "K-405 pinned local candidate",
        "needs_restart": name != "skills",
        "artifacts": rows,
    }


def build(args: argparse.Namespace) -> dict:
    for path, expected in [
        (args.cli_binary, args.cli_sha256),
        (args.core_archive, args.core_archive_sha256),
        (args.core_manifest, args.core_manifest_sha256),
        (args.skills_archive, args.skills_archive_sha256),
        (args.mcp_wheel, args.mcp_sha256),
        (args.runtime_input, args.runtime_input_sha256),
    ]:
        require(path, expected)
    if args.out.exists():
        raise ValueError("candidate output already exists")
    core = json.loads(args.core_manifest.read_text())
    runtime = json.loads(args.runtime_input.read_text())
    if (
        core["platform"] != "linux-x64"
        or core["archive"]["sha256"] != args.core_archive_sha256
        or core["signing"] != "unsigned"
        or core["publication"] != "not_published"
        or runtime["task_id"] != "K-404"
        or runtime["lane"] != "container-linux-x64"
        or runtime["mcp"]["wheel_sha256"] != args.mcp_sha256
    ):
        raise ValueError("input platform, authority, or MCP identity differs")
    release = args.out / "release"
    release.mkdir(parents=True)
    core_names = unpack_regular(args.core_archive, release)
    if core_names != {"axiom", "axiom-graphd", "release-info.json"}:
        raise ValueError("core archive membership differs")
    (release / "release-info.json").unlink()
    binaries = {row["name"]: row for row in core["binaries"]}
    if set(binaries) != {"axiom", "axiom-graphd"}:
        raise ValueError("core binary set differs")
    for name, row in binaries.items():
        if digest(release / name) != row["sha256"]:
            raise ValueError(f"core binary changed: {name}")
        if row["version_report"]["build_revision"] != core["source_revision"]:
            raise ValueError("core build revision differs")
        (release / name).chmod(0o755)
    (release / "axiom-cli").write_bytes(args.cli_binary.read_bytes())
    (release / "axiom-cli").chmod(0o755)
    skill_names = unpack_regular(args.skills_archive, release)
    if digest(release / "skills-manifest.json") != args.skills_manifest_sha256:
        raise ValueError("skills manifest changed")
    skill_manifest = json.loads((release / "skills-manifest.json").read_text())
    declared = {row["path"] for row in skill_manifest["files"]}
    if skill_names != declared | {"skills-manifest.json"}:
        raise ValueError("skills archive membership differs")
    for row in skill_manifest["files"]:
        path = release / row["path"]
        if digest(path) != row["sha256"] or path.stat().st_size != row["bytes"]:
            raise ValueError(f"skills payload changed: {row['path']}")
    wheel = release / args.mcp_wheel.name
    wheel.write_bytes(args.mcp_wheel.read_bytes())
    core_revision = core["source_revision"]
    cli_revision = args.cli_revision
    skills_revision = args.skills_revision
    mcp_revision = runtime["mcp"]["source_revision"]
    artifacts = []
    for name, version, revision in [
        ("axiom-cli", args.cli_version, cli_revision),
        ("axiom", core["version"], core_revision),
        ("axiom-graphd", core["version"], core_revision),
    ]:
        path = release / name
        artifacts.append(
            {
                "name": name,
                "component": name,
                "version": version,
                "revision": revision,
                "arch": "x86_64",
                "sha256": digest(path),
                "size_bytes": path.stat().st_size,
                "archive": name,
                "kind": "executable",
            }
        )
    release_set = {
        "release_set_version": 1,
        "release_version": core["version"],
        "platform": "linux-x64",
        "target_triple": "x86_64-unknown-linux-gnu",
        "libc": "glibc",
        "arch": "x86_64",
        "generated_by": "K-405 candidate assembly",
        "carries_components": [row["name"] for row in artifacts],
        "user_service": None,
        "artifacts": artifacts,
        "signing": "unsigned",
        "notarization": "not_notarized",
    }
    (release / "release-set.json").write_text(json.dumps(release_set, indent=2) + "\n")
    trust = json.loads((ROOT / "channels/stable.json").read_text())["trust"]
    channel = {
        "manifest_version": 1,
        "channel": "stable",
        "updated_at": args.updated_at,
        "published": False,
        "note": "Unsigned local Linux x64 container candidate. No release authority.",
        "trust": trust,
        "components": [
            component("axiom-graphd", core["version"], core_revision, release / "axiom-graphd"),
            component("axiom-mcp", args.mcp_version, mcp_revision, wheel),
            component("skills", skill_manifest["component_version"], skills_revision, None),
        ],
    }
    (release / "channel.json").write_text(json.dumps(channel, indent=2, sort_keys=True) + "\n")
    inputs = {
        "task_id": "K-405",
        "lane": "container-linux-x64",
        "status": "candidate",
        "certified": False,
        "container_image_id": args.image_id,
        "cli_binary_sha256": args.cli_sha256,
        "core_archive_sha256": args.core_archive_sha256,
        "core_source_revision": core_revision,
        "skills_archive_sha256": args.skills_archive_sha256,
        "skills_manifest_sha256": args.skills_manifest_sha256,
        "mcp_wheel_sha256": args.mcp_sha256,
        "mcp_source_revision": mcp_revision,
        "runtime_input_sha256": args.runtime_input_sha256,
        "runtime_archive_sha256": runtime["runtime"]["sha256"],
        "wheelhouse_archive_sha256": runtime["wheelhouse"]["sha256"],
        "dependency_lock_sha256": runtime["mcp"]["dependency_lock_sha256"],
        "published": False,
        "signing": "unsigned",
    }
    (release / "candidate-inputs.json").write_text(json.dumps(inputs, indent=2, sort_keys=True) + "\n")
    files = []
    for path in sorted(x for x in release.rglob("*") if x.is_file()):
        files.append({"path": path.relative_to(release).as_posix(), "sha256": digest(path), "size_bytes": path.stat().st_size})
    manifest = {"task_id": "K-405", "candidate": True, "files": files, "inputs": inputs}
    (args.out / "candidate-files.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    archive_path = args.out / f"axiom-{core['version']}-container-linux-x64-candidate.tar.gz"
    with archive_path.open("wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", mtime=0, filename="") as gz, tarfile.open(fileobj=gz, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for row in files:
            path = release / row["path"]
            info = tarfile.TarInfo(row["path"])
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = 0
            info.mode = 0o755 if path.name in {"axiom-cli", "axiom", "axiom-graphd"} else 0o644
            info.size = path.stat().st_size
            with path.open("rb") as stream:
                archive.addfile(info, stream)
    return {"status": "candidate", "published": False, "files": len(files), "archive_sha256": digest(archive_path), "channel_sha256": digest(release / "channel.json")}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("cli-binary", "core-archive", "core-manifest", "skills-archive", "mcp-wheel", "runtime-input", "out"):
        parser.add_argument("--" + name, required=True, type=Path)
    for name in ("cli-sha256", "core-archive-sha256", "core-manifest-sha256", "skills-archive-sha256", "skills-manifest-sha256", "mcp-sha256", "runtime-input-sha256", "cli-revision", "skills-revision", "image-id", "updated-at"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--cli-version", default="0.1.0")
    parser.add_argument("--mcp-version", default="0.1.0")
    args = parser.parse_args()
    try:
        print(json.dumps(build(args), sort_keys=True))
        return 0
    except (OSError, ValueError, KeyError, tarfile.TarError) as error:
        parser.exit(2, f"container candidate refused: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
