"""Compose an unsigned Windows channel from verified skills, core, MCP and Python inputs."""

from __future__ import annotations

import argparse
import email
import hashlib
import json
import os
import re
import shutil
import tarfile
import tempfile
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HEX40 = re.compile(r"[0-9a-f]{40}\Z")
HEX64 = re.compile(r"[0-9a-f]{64}\Z")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(path: Path, expected: str) -> None:
    if (
        path.is_symlink()
        or not path.is_file()
        or not HEX64.fullmatch(expected)
        or digest(path) != expected
    ):
        raise ValueError(
            "candidate input is missing, linked or differs from its SHA-256"
        )


def relative(name: str) -> bool:
    return (
        bool(name)
        and not name.startswith("/")
        and "\\" not in name
        and ":" not in name
        and all(part not in ("", ".", "..") for part in name.split("/"))
    )


def owner_skills(
    archive: Path, expected: str, manifest_sha: str, revision: str, stage: Path
) -> dict:
    require(archive, expected)
    if not HEX40.fullmatch(revision):
        raise ValueError("skills revision is not immutable")
    with tarfile.open(archive) as tar:
        members = tar.getmembers()
        names = [row.name for row in members]
        if (
            not 2 <= len(members) <= 4096
            or len(set(names)) != len(members)
            or not all(row.isfile() and relative(row.name) for row in members)
        ):
            raise ValueError("skills source archive membership is invalid")
        if "skills-manifest.json" not in names:
            raise ValueError("skills source manifest is missing")
        for row in members:
            target = stage / row.name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(tar.extractfile(row).read())
    require(stage / "skills-manifest.json", manifest_sha)
    manifest = json.loads((stage / "skills-manifest.json").read_text(encoding="utf-8"))
    if (
        manifest.get("component") != "axiom-skills"
        or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", str(manifest.get("component_version", "")))
        or manifest.get("spec_version") != "2.0.0-draft.1"
        or not HEX40.fullmatch(manifest.get("spec_revision", ""))
    ):
        raise ValueError("skills source identity or compatibility is invalid")
    rows = manifest.get("files")
    if not isinstance(rows, list) or not 1 <= len(rows) <= 4095:
        raise ValueError("skills payload is incomplete")
    declared = {"skills-manifest.json"}
    for row in rows:
        name = row.get("path")
        if not isinstance(name, str) or not relative(name) or name in declared:
            raise ValueError("skills path is unsafe or duplicated")
        declared.add(name)
        path = stage / name
        require(path, row.get("sha256", ""))
        if path.stat().st_size != row.get("bytes"):
            raise ValueError("skills payload size changed")
    if {
        p.relative_to(stage).as_posix() for p in stage.rglob("*") if p.is_file()
    } != declared:
        raise ValueError("skills archive has undeclared or missing files")
    return {
        "version": manifest["component_version"],
        "revision": revision,
        "spec_revision": manifest["spec_revision"],
        "manifest_sha256": manifest_sha,
    }


def component(name: str, version: str, revision: str, artifact: Path | None) -> dict:
    rows = (
        []
        if artifact is None
        else [
            {
                "platform": "windows-x64",
                "class": "per-user-installer",
                "url": f"https://candidate.invalid/{artifact.name}",
                "sha256": digest(artifact),
                "size_bytes": artifact.stat().st_size,
            }
        ]
    )
    return {
        "component": name,
        "status": "declared",
        "version": version,
        "revision": revision,
        "declared_source": "immutable K-305 candidate owner handoff",
        "needs_restart": name != "skills",
        "artifacts": rows,
    }


def build(args: argparse.Namespace) -> dict:
    release = args.release_set.resolve()
    source = release / "release-set.json"
    if not source.is_file() or release.is_symlink():
        raise ValueError("Windows release set is missing")
    base = json.loads(source.read_text(encoding="utf-8"))
    release_version = base.get("release_version", "")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", str(release_version)):
        raise ValueError("release version is not SemVer")
    if (
        base.get("document_kind"),
        base.get("platform"),
        base.get("arch"),
        base.get("release_version"),
    ) != ("axiom-cli-release-set", "windows-x64", "x86_64", release_version):
        raise ValueError("release set identity or version mismatch")
    rows = {row["name"]: row for row in base["artifacts"]}
    if len(rows) != len(base["artifacts"]):
        raise ValueError("release set has duplicate artifacts")
    core = json.loads(args.core_manifest.read_text(encoding="utf-8"))
    if (
        core.get("platform"),
        core.get("version"),
        core.get("signing"),
        core.get("publication"),
    ) != ("windows-x64", release_version, "unsigned", "not_published") or not HEX40.fullmatch(
        core.get("source_revision", "")
    ):
        raise ValueError("core candidate provenance is invalid")
    binaries = {row["name"]: row for row in core["binaries"]}
    if set(binaries) != {"axiom.exe", "axiom-graphd.exe"}:
        raise ValueError("core candidate must have both native executables")
    for name in ("axiom-cli.exe", *sorted(binaries)):
        row = rows.get(name)
        path = release / name
        if row is None:
            raise ValueError(f"release set is missing {name}")
        require(path, row["sha256"])
        if path.stat().st_size != row["size_bytes"]:
            raise ValueError("release set artifact size mismatch")
        if name in binaries and (
            row["sha256"] != binaries[name]["sha256"]
            or row["size_bytes"] != binaries[name]["size_bytes"]
        ):
            raise ValueError("core release set differs from owner candidate")
    if args.core_archive_sha != core["archive"]["sha256"]:
        raise ValueError("core archive digest differs from owner candidate")
    archive_name = core["archive"]["name"]
    if (
        archive_name not in rows
        or rows[archive_name]["sha256"] != args.core_archive_sha
    ):
        raise ValueError("release set omits the exact core candidate archive")
    require(release / archive_name, args.core_archive_sha)
    runtime = json.loads(args.runtime_manifest.read_text(encoding="utf-8"))
    if runtime.get("task_id") != "K-304" or runtime.get("lane") != "windows-x64":
        raise ValueError("runtime handoff is not Windows K-304")
    expected = {
        "python-3.13.15-embed-amd64.zip": runtime["python"]["sha256"],
        "pip-26.1.2-py3-none-any.whl": runtime["pip"]["sha256"],
        "windows-x64-py313-inputs.tar": runtime["mcp_inputs"]["sha256"],
        args.mcp_wheel.name: runtime["mcp_inputs"]["wheel_sha256"],
    }
    if args.mcp_sha != runtime["mcp_inputs"]["wheel_sha256"]:
        raise ValueError("MCP wheel differs from runtime handoff")
    for name, sha in expected.items():
        row = rows.get(name)
        if row is None or row["sha256"] != sha:
            raise ValueError(f"release set omits or changes pinned {name}")
        require(release / name, sha)
    require(args.mcp_wheel, args.mcp_sha)
    if digest(release / args.mcp_wheel.name) != args.mcp_sha:
        raise ValueError("staged MCP wheel differs from K-303")
    if not HEX40.fullmatch(args.mcp_revision):
        raise ValueError("MCP source revision is not immutable")
    if args.mcp_revision != runtime["mcp_inputs"]["source_revision"]:
        raise ValueError("MCP source revision differs from runtime handoff")
    with zipfile.ZipFile(args.mcp_wheel) as wheel:
        metadata = [
            name for name in wheel.namelist() if name.endswith(".dist-info/METADATA")
        ]
        if len(metadata) != 1:
            raise ValueError("MCP wheel metadata is missing or ambiguous")
        fields = email.message_from_bytes(wheel.read(metadata[0]))
        mcp_version = fields.get("Version")
        if fields.get("Name") != "axiom-mcp" or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", str(mcp_version)):
            raise ValueError("MCP wheel name or version differs from the candidate")
    outputs = [release / "channel.json", release / "skills-manifest.json"]
    for path in outputs:
        if path.exists() or path.is_symlink():
            raise ValueError("candidate output already exists")
    trust = json.loads((ROOT / "channels/stable.json").read_text(encoding="utf-8"))[
        "trust"
    ]
    with tempfile.TemporaryDirectory(
        prefix=".windows-candidate-", dir=release
    ) as temporary:
        stage = Path(temporary)
        skill_info = owner_skills(
            args.skills_tar,
            args.skills_sha,
            args.skills_manifest_sha,
            args.skills_revision,
            stage,
        )
        targets = [release / entry.name for entry in stage.iterdir()]
        for target in targets:
            if target.exists() or target.is_symlink():
                raise ValueError("skills payload conflicts with release set")
        channel = {
            "manifest_version": 1,
            "channel": "stable",
            "updated_at": "2026-10-01T00:00:00Z",
            "published": False,
            "note": "Unsigned local Windows x64 candidate; no release certification.",
            "trust": trust,
            "components": [
                component(
                    "axiom-graphd",
                    core["version"],
                    core["source_revision"],
                    release / "axiom-graphd.exe",
                ),
                component(
                    "axiom-mcp",
                    mcp_version,
                    args.mcp_revision,
                    release / args.mcp_wheel.name,
                ),
                component(
                    "skills", skill_info["version"], skill_info["revision"], None
                ),
            ],
        }
        lock = {
            "task_id": "K-305",
            "lane": "windows-x64",
            "candidate": True,
            "certified": False,
            "release_set_sha256": digest(source),
            "core_archive_sha256": args.core_archive_sha,
            "core_revision": core["source_revision"],
            "mcp_revision": args.mcp_revision,
            "mcp_wheel_sha256": args.mcp_sha,
            "skills_source_sha256": args.skills_sha,
            "skills_manifest_sha256": skill_info["manifest_sha256"],
            "runtime_sha256": runtime["python"]["sha256"],
            "inputs_sha256": runtime["mcp_inputs"]["sha256"],
        }
        (stage / "channel.json").write_text(
            json.dumps(channel, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
            newline="\n",
        )
        (stage / "candidate-lock.json").write_text(
            json.dumps(lock, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
            newline="\n",
        )
        installed = []
        try:
            for item in [*stage.iterdir()]:
                target = release / item.name
                if target.exists() or target.is_symlink():
                    raise ValueError("candidate output conflict")
                os.replace(item, target)
                installed.append(target)
        except Exception:
            for item in installed:
                if item.is_dir():
                    shutil.rmtree(item)
                else:
                    item.unlink()
            raise
    return {
        "status": "candidate",
        "published": False,
        "channel_sha256": digest(release / "channel.json"),
        "lock_sha256": digest(release / "candidate-lock.json"),
        "core_revision": core["source_revision"],
        "mcp_revision": args.mcp_revision,
        "skills_revision": skill_info["revision"],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-set", type=Path, required=True)
    parser.add_argument("--core-manifest", type=Path, required=True)
    parser.add_argument("--core-archive-sha", required=True)
    parser.add_argument("--mcp-wheel", type=Path, required=True)
    parser.add_argument("--mcp-sha", required=True)
    parser.add_argument("--mcp-revision", required=True)
    parser.add_argument("--skills-tar", type=Path, required=True)
    parser.add_argument("--skills-sha", required=True)
    parser.add_argument("--skills-manifest-sha", required=True)
    parser.add_argument("--skills-revision", required=True)
    parser.add_argument("--runtime-manifest", type=Path, required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(build(args), sort_keys=True))
        return 0
    except (
        OSError,
        ValueError,
        KeyError,
        TypeError,
        tarfile.TarError,
        zipfile.BadZipFile,
        json.JSONDecodeError,
    ) as error:
        print(json.dumps({"status": "refused", "reason": str(error)}, sort_keys=True))
        return 9


if __name__ == "__main__":
    raise SystemExit(main())
