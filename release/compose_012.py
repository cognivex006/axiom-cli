"""Compose version-pinned native distributions from actual checksum-verified owner Releases."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = (ROOT / "VERSION").read_text(encoding="utf-8").strip()


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(*args, **kwargs):
    subprocess.run([str(a) for a in args], check=True, **kwargs)


def module(path):
    spec = importlib.util.spec_from_file_location("compose_" + path.stem, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def unpack(path, out):
    out.mkdir(parents=True)
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as archive:
            names = archive.namelist()
            if len(names) != len(set(names)):
                raise ValueError("duplicate ZIP members")
            for name in names:
                p = Path(name)
                if p.is_absolute() or ".." in p.parts or "\\" in name:
                    raise ValueError("unsafe ZIP path")
                target = out / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(archive.read(name))
    else:
        with tarfile.open(path) as archive:
            names = set()
            for item in archive:
                p = Path(item.name)
                if (
                    not item.isfile()
                    or p.is_absolute()
                    or ".." in p.parts
                    or item.name in names
                ):
                    raise ValueError("unsafe tar member")
                names.add(item.name)
                target = out / item.name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(archive.extractfile(item).read())
                target.chmod(item.mode)


def owner(repo, directory):
    directory.mkdir()
    run(
        "gh",
        "release",
        "download",
        "v" + VERSION,
        "--repo",
        "orchex006/" + repo,
        "--dir",
        directory,
    )
    source = (directory / "SOURCE-REVISION.txt").read_text().strip()
    refs = dict(
        reversed(line.split("	", 1))
        for line in subprocess.check_output(
            [
                "git",
                "ls-remote",
                "https://github.com/orchex006/" + repo + ".git",
                "refs/tags/v" + VERSION,
                "refs/tags/v" + VERSION + "^{}",
            ],
            text=True,
        ).splitlines()
    )
    # An annotated tag lists the tag object and its peeled commit (`^{}`); use the commit.
    tag = refs.get("refs/tags/v" + VERSION + "^{}") or refs["refs/tags/v" + VERSION]
    if source != tag:
        raise ValueError("owner Release/tag source mismatch")
    for line in (directory / "SHA256SUMS").read_text().splitlines():
        digest, name = line.split("  ", 1)
        name = name.removeprefix("./")
        if Path(name).name != name or sha(directory / name) != digest:
            raise ValueError("owner Release asset hash mismatch")
    return source


def compose(lane, output, cli_revision):
    cli_revision = subprocess.check_output(
        ["git", "rev-parse", cli_revision], cwd=ROOT, text=True
    ).strip()
    if len(cli_revision) != 40:
        raise ValueError("immutable CLI revision required")
    if output.exists():
        raise ValueError("new distribution output required")
    output.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix="axiom-compose-012-") as temporary:
        work = Path(temporary).resolve()
        peers = {
            name: work / name for name in ["axiom-graphd", "axiom-mcp", "axiom-skills"]
        }
        pins = {name: owner(name, path) for name, path in peers.items()}
        core_manifest = peers["axiom-graphd"] / (lane + "-manifest.json")
        core = json.loads(core_manifest.read_bytes())
        if (
            core["version"] != VERSION
            or core["source_revision"] != pins["axiom-graphd"]
        ):
            raise ValueError("core version/source mismatch")
        core_archive = peers["axiom-graphd"] / core["archive"]["name"]
        binaries = work / "core"
        unpack(core_archive, binaries)
        skills = work / "skills"
        unpack(peers["axiom-skills"] / ("axiom-skills-" + VERSION + ".zip"), skills)
        skill_tar = work / "skills-owner.tar"
        with tarfile.open(skill_tar, "w") as archive:
            for path in sorted(p for p in skills.rglob("*") if p.is_file()):
                info = tarfile.TarInfo(path.relative_to(skills).as_posix())
                info.size = path.stat().st_size
                info.mode = 0o644
                with path.open("rb") as stream:
                    archive.addfile(info, stream)
        mcp = peers["axiom-mcp"]
        wheel = mcp / ("axiom_mcp-" + VERSION + "-py3-none-any.whl")
        dependencies = json.loads((mcp / (lane + "-runtime-inputs.json")).read_bytes())
        if dependencies["source_revision"] != pins["axiom-mcp"]:
            raise ValueError("MCP dependency source mismatch")
        lock = mcp / dependencies["dependency_lock"]["name"]
        house = mcp / dependencies["wheelhouse"]["name"]
        release = work / "release"
        cli = ROOT / (
            "target/release/axiom-cli.exe"
            if lane == "windows-x64"
            else "target/release/axiom-cli"
        )
        if lane == "macos-x64":
            run(
                "sh",
                ROOT / "packaging/macos/Build-ReleaseSet.sh",
                "--out-dir",
                release,
                "--cli-binary",
                cli,
                "--engine-cli",
                binaries / "axiom",
                "--engine-daemon",
                binaries / "axiom-graphd",
                "--release-version",
                VERSION,
                "--core-version",
                VERSION,
                "--core-revision",
                pins["axiom-graphd"],
            )
            run(
                "python3",
                ROOT / "packaging/macos/Build-EngineCandidate.py",
                "--release-set",
                release,
                "--mcp-wheel",
                wheel,
                "--mcp-sha256",
                sha(wheel),
                "--mcp-revision",
                pins["axiom-mcp"],
                "--skills-bundle",
                skills,
                "--skills-revision",
                pins["axiom-skills"],
                "--skills-manifest-sha256",
                sha(skills / "skills-manifest.json"),
            )
            runtime = ROOT / "evidence/K-104/cpython-3.13.15-macos-x64-candidate.tar.gz"
            record = json.loads(
                (ROOT / "evidence/K-104/runtime-input.json").read_bytes()
            )
            if sha(runtime) != record["runtime"]["sha256"]:
                raise ValueError("native Python archive mismatch")
            record["mcp"] = {
                "source_revision": pins["axiom-mcp"],
                "wheel_sha256": sha(wheel),
                "dependency_lock_sha256": sha(lock),
            }
            record["wheelhouse"]["sha256"] = sha(house)
            receipt = work / "runtime-input.json"
            receipt.write_text(json.dumps(record, indent=2))
            run(
                "python3",
                ROOT / "packaging/macos/Attach-McpRuntime.py",
                "--release-set",
                release,
                "--runtime-input",
                receipt,
                "--runtime-input-sha256",
                sha(receipt),
                "--python-archive",
                runtime,
                "--wheelhouse-archive",
                house,
                "--wheel",
                wheel,
                "--lock",
                lock,
            )
        elif lane == "linux-x64":
            runtime = ROOT / "evidence/K-404/cpython-3.13.15-linux-x64-candidate.tar.gz"
            record = json.loads(
                (ROOT / "evidence/K-404/runtime-input.json").read_bytes()
            )
            if sha(runtime) != record["runtime"]["sha256"]:
                raise ValueError("native Python archive mismatch")
            record["mcp"].update(
                source_revision=pins["axiom-mcp"],
                wheel_sha256=sha(wheel),
                dependency_lock_sha256=sha(lock),
            )
            record["wheelhouse"]["sha256"] = sha(house)
            receipt = work / "runtime-input.json"
            receipt.write_text(json.dumps(record, indent=2))
            args = argparse.Namespace(
                cli_binary=cli,
                cli_sha256=sha(cli),
                core_archive=core_archive,
                core_archive_sha256=sha(core_archive),
                core_manifest=core_manifest,
                core_manifest_sha256=sha(core_manifest),
                skills_archive=skill_tar,
                skills_archive_sha256=sha(skill_tar),
                skills_manifest_sha256=sha(skills / "skills-manifest.json"),
                mcp_wheel=wheel,
                mcp_sha256=sha(wheel),
                runtime_input=receipt,
                runtime_input_sha256=sha(receipt),
                cli_revision=cli_revision,
                skills_revision=pins["axiom-skills"],
                image_id=None,
                updated_at="2026-10-05T00:00:00Z",
                cli_version=VERSION,
                mcp_version=VERSION,
                out=work / "linux",
            )
            module(ROOT / "packaging/linux/Build-ContainerCandidate.py").build(args)
            shutil.move(str(args.out / "release"), str(release))
            runtime_dir = release / "runtime"
            runtime_dir.mkdir()
            for src, name in [
                (runtime, "python.tar.gz"),
                (house, "wheelhouse.tar.gz"),
                (lock, "requirements.txt"),
                (wheel, wheel.name),
                (ROOT / "packaging/linux/Provision-McpRuntime.py", "provision.py"),
                (ROOT / "release/bootstrap_linux.py", "bootstrap.py"),
            ]:
                shutil.copyfile(src, runtime_dir / name)
            manifest = {
                "platform": lane,
                "version": VERSION,
                "mcp_revision": pins["axiom-mcp"],
                "files": {p.name: sha(p) for p in runtime_dir.iterdir()},
            }
            (runtime_dir / "manifest.json").write_text(json.dumps(manifest, indent=2))
            for script in (ROOT / "installers/linux").glob("*.sh"):
                shutil.copyfile(script, release / script.name)
                (release / script.name).chmod(0o755)
        else:
            record = json.loads(
                (ROOT / "packaging/windows/runtime-input.json").read_bytes()
            )
            runtime = work / "python-3.13.15-embed-amd64.zip"
            run(
                "curl",
                "--fail",
                "--location",
                "--output",
                runtime,
                record["python"]["url"],
            )
            if sha(runtime) != record["python"]["sha256"]:
                raise ValueError("Windows Python archive mismatch")
            pip_dir = work / "pip"
            run(
                "python",
                "-m",
                "pip",
                "download",
                "--no-deps",
                "pip==26.1.2",
                "-d",
                pip_dir,
            )
            pip = pip_dir / "pip-26.1.2-py3-none-any.whl"
            if sha(pip) != record["pip"]["sha256"]:
                raise ValueError("pip archive mismatch")
            inputs = mcp / dependencies["inputs"]["name"]
            record["mcp_inputs"].update(
                source_revision=pins["axiom-mcp"],
                wheel_sha256=sha(wheel),
                sha256=sha(inputs),
            )
            receipt = work / "runtime-input.json"
            receipt.write_text(json.dumps(record, indent=2))
            declarations = [
                "axiom=" + str(binaries / "axiom.exe"),
                "axiom-graphd=" + str(binaries / "axiom-graphd.exe"),
                "core=" + str(core_archive),
                "axiom-mcp=" + str(wheel),
                "runtime=" + str(runtime),
                "runtime=" + str(pip),
                "runtime=" + str(inputs),
            ]
            command = work / "build.ps1"
            command.write_text(
                "$ErrorActionPreference='Stop'\n& '"
                + str(ROOT / "packaging/windows/Build-ReleaseSet.ps1").replace(
                    "'", "''"
                )
                + "' -OutDir '"
                + str(release).replace("'", "''")
                + "' -CliExe '"
                + str(cli).replace("'", "''")
                + "' -CoreCandidateManifest '"
                + str(core_manifest).replace("'", "''")
                + "' -Version '"
                + VERSION
                + "' -ComponentArtifact @( "
                + ",".join("'" + x.replace("'", "''") + "'" for x in declarations)
                + " ) -Json\n",
                encoding="utf-8",
            )
            run("pwsh", "-NoProfile", "-File", command)
            run(
                "python",
                ROOT / "packaging/windows/Build-EngineCandidate.py",
                "--release-set",
                release,
                "--core-manifest",
                core_manifest,
                "--core-archive-sha",
                sha(core_archive),
                "--mcp-wheel",
                wheel,
                "--mcp-sha",
                sha(wheel),
                "--mcp-revision",
                pins["axiom-mcp"],
                "--skills-tar",
                skill_tar,
                "--skills-sha",
                sha(skill_tar),
                "--skills-manifest-sha",
                sha(skills / "skills-manifest.json"),
                "--skills-revision",
                pins["axiom-skills"],
                "--runtime-manifest",
                receipt,
            )
            for src in [
                *(ROOT / "installers/windows").glob("*.ps1"),
                ROOT / "packaging/windows/Provision-McpRuntime.ps1",
                ROOT / "packaging/windows/Provision-McpRuntime.py",
                ROOT / "release/bootstrap_windows.ps1",
            ]:
                shutil.copyfile(src, release / src.name)
            shutil.copyfile(receipt, release / "runtime-input.json")
        pins["axiom-cli"] = cli_revision
        (release / "RELEASE-SOURCES.json").write_text(json.dumps(pins, indent=2))
        (release / "README.md").write_bytes(
            (ROOT / "release/oneline-release-notes.md").read_bytes()
        )
        manifest = {
            "version": VERSION,
            "platform": lane,
            "source_revision": cli_revision,
            "owners": pins,
            "files": [
                {
                    "path": p.relative_to(release).as_posix(),
                    "sha256": sha(p),
                    "bytes": p.stat().st_size,
                }
                for p in sorted(release.rglob("*"))
                if p.is_file()
            ],
        }
        (release / "DISTRIBUTION-FILES.json").write_text(json.dumps(manifest, indent=2))
        archive = output / (
            "axiom-"
            + VERSION
            + "-"
            + lane
            + (".zip" if lane == "windows-x64" else ".tar.gz")
        )
        if lane == "windows-x64":
            with zipfile.ZipFile(
                archive, "w", compression=zipfile.ZIP_DEFLATED
            ) as stream:
                for p in sorted(release.rglob("*")):
                    if p.is_file():
                        stream.write(p, p.relative_to(release).as_posix())
        else:
            with tarfile.open(archive, "w:gz") as stream:
                for p in sorted(release.rglob("*")):
                    if p.is_file():
                        stream.add(p, arcname=p.relative_to(release).as_posix())
        shutil.copyfile(
            release / "DISTRIBUTION-FILES.json", output / (lane + "-distribution.json")
        )
        # Preserve the assembled directory for native verification in this job.
        shutil.copytree(release, output / "assembled")
    return archive


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--platform", choices=["windows-x64", "linux-x64", "macos-x64"], required=True
    )
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--source-revision", required=True)
    args = parser.parse_args()
    compose(args.platform, args.out.resolve(), args.source_revision)
