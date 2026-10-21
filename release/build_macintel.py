#!/usr/bin/env python3
"""Compose the 0.1.1 test distribution from checksum-verified owner releases."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]
OUT = Path(os.environ["OUT"])
WORK = Path(os.environ["RUNNER_TEMP"]) / "compose"
WORK.mkdir()


def run(*args, **kw):
    subprocess.run([str(x) for x in args], check=True, **kw)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


pins = {}
for repo in ("axiom-graphd", "axiom-mcp", "axiom-skills"):
    dest = WORK / repo
    dest.mkdir()
    run(
        "gh",
        "release",
        "download",
        "v0.1.1",
        "--repo",
        "cognivex006/" + repo,
        "--dir",
        dest,
    )
    run("shasum", "-a", "256", "-c", "SHA256SUMS", cwd=dest)
    revision = (dest / "SOURCE-REVISION.txt").read_text().strip()
    remote = subprocess.check_output(
        [
            "git",
            "ls-remote",
            "https://github.com/cognivex006/" + repo + ".git",
            "refs/tags/v0.1.1",
        ],
        text=True,
    ).split()[0]
    if revision != remote:
        raise ValueError("release source does not match tag: " + repo)
    pins[repo] = revision
core = WORK / "core"
skills = WORK / "skills"
core.mkdir()
skills.mkdir()
for archive, destination in [
    (WORK / "axiom-graphd/axiom-0.1.1-macos-x64.tar.gz", core),
    (WORK / "axiom-skills/axiom-skills-0.1.1-owner-source.tar.gz", skills),
]:
    with tarfile.open(archive) as stream:
        stream.extractall(destination, filter="data")
release = WORK / "axiom-0.1.1-macos-x64"
run(
    "sh",
    ROOT / "packaging/macos/Build-ReleaseSet.sh",
    "--out-dir",
    release,
    "--cli-binary",
    ROOT / "target/release/axiom-cli",
    "--engine-cli",
    core / "axiom",
    "--engine-daemon",
    core / "axiom-graphd",
    "--core-version",
    "0.1.1",
    "--core-revision",
    pins["axiom-graphd"],
)
wheel = WORK / "axiom-mcp/axiom_mcp-0.1.1-py3-none-any.whl"
lock = WORK / "axiom-mcp/macos-x64-py313-requirements.txt"
wheelhouse = WORK / "axiom-mcp/axiom-mcp-0.1.1-macos-x64-wheelhouse.tar.gz"
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
runtime_input = json.loads((ROOT / "evidence/K-104/runtime-input.json").read_text())
if sha(runtime) != runtime_input["runtime"]["sha256"]:
    raise ValueError("CPython archive differs from recorded pin")
runtime_input["mcp"] = {
    "source_revision": pins["axiom-mcp"],
    "wheel_sha256": sha(wheel),
    "dependency_lock_sha256": sha(lock),
}
runtime_input["wheelhouse"]["sha256"] = sha(wheelhouse)
input_path = WORK / "runtime-input.json"
input_path.write_text(json.dumps(runtime_input, indent=2) + "\n")
run(
    "python3",
    ROOT / "packaging/macos/Attach-McpRuntime.py",
    "--release-set",
    release,
    "--runtime-input",
    input_path,
    "--runtime-input-sha256",
    sha(input_path),
    "--python-archive",
    runtime,
    "--wheelhouse-archive",
    wheelhouse,
    "--wheel",
    wheel,
    "--lock",
    lock,
)
(release / "RELEASE-SOURCES.json").write_text(
    json.dumps(dict(pins, **{"axiom-cli": os.environ["GITHUB_SHA"]}), indent=2) + "\n"
)
shutil.copyfile(ROOT / "release/v0.1.1-notes.md", release / "README.md")
# Verify all staged inputs and produce a plan in a fresh HOME without installing.
home = WORK / "test-home"
home.mkdir()
env = dict(os.environ, HOME=str(home))
env.pop("AXIOM_HOME", None)
run(
    "sh",
    release / "runtime/bootstrap.sh",
    "install",
    "--release-set",
    release,
    "--dry-run",
    env=env,
)
run("tar", "-czf", OUT / "axiom-0.1.1-macos-x64.tar.gz", "-C", WORK, release.name)
shutil.copyfile(release / "RELEASE-SOURCES.json", OUT / "RELEASE-SOURCES.json")
