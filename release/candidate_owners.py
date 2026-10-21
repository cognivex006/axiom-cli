"""Read-only candidate inputs from successful canonical, immutable owner CI runs."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

WORKFLOWS = {
    "axiom-graphd": "core-ci.yml",
    "axiom-mcp": "mcp-ci.yml",
    "axiom-skills": "skills-ci.yml",
}


def checked_run(repo, pin, observed):
    if repo not in WORKFLOWS or not isinstance(pin, dict) or set(pin) != {"run_id", "source_revision"}:
        raise ValueError("explicit owner CI run/source required")
    revision = pin["source_revision"]
    if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("immutable owner source required")
    if type(pin["run_id"]) is not int or pin["run_id"] < 1:
        raise ValueError("numeric owner CI run required")
    if (
        observed.get("id") != pin["run_id"]
        or observed.get("head_sha") != revision
        or observed.get("head_repository", {}).get("full_name") != "cognivex006/" + repo
        or observed.get("path") != ".github/workflows/" + WORKFLOWS[repo]
        or observed.get("event") != "push"
        or observed.get("status") != "completed"
        or observed.get("conclusion") != "success"
    ):
        raise ValueError("owner CI identity or completion mismatch")
    return revision


def verify_payload(directory, revision):
    if (directory / "SOURCE-REVISION.txt").read_text().strip() != revision:
        raise ValueError("candidate owner source mismatch")
    declared = set()
    for line in (directory / "SHA256SUMS").read_text().splitlines():
        digest, name = line.split("  ", 1)
        if not re.fullmatch(r"[0-9a-f]{64}", digest) or not name or name in declared or "/" in name or "\\" in name or name in {".", ".."}:
            raise ValueError("unsafe candidate checksum record")
        path = directory / name
        if path.is_symlink() or not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError("candidate owner checksum mismatch")
        declared.add(name)
    actual = {p.name for p in directory.iterdir() if p.name != "SHA256SUMS"}
    if not declared or declared != actual or "SOURCE-REVISION.txt" not in declared:
        raise ValueError("candidate owner checksum inventory mismatch")


def fetch(repo, directory, pin):
    # Validate local shape before using any value in a command or API route.
    if repo not in WORKFLOWS or not isinstance(pin, dict) or set(pin) != {"run_id", "source_revision"} or type(pin.get("run_id")) is not int or pin["run_id"] < 1:
        raise ValueError("explicit numeric owner run required")
    if not isinstance(pin.get("source_revision"), str) or not re.fullmatch(r"[0-9a-f]{40}", pin["source_revision"]):
        raise ValueError("immutable owner source required")
    observed = json.loads(subprocess.check_output(["gh", "api", f"repos/cognivex006/{repo}/actions/runs/{pin['run_id']}"]))
    revision = checked_run(repo, pin, observed)
    raw = directory.with_name(repo + "-ci")
    raw.mkdir()
    patterns = ["portable-skills-" + revision] if repo == "axiom-skills" else ["native-*-" + revision]
    if repo == "axiom-mcp": patterns.append("portable-wheel-" + revision)
    for pattern in patterns:
        subprocess.run(["gh", "run", "download", str(pin["run_id"]), "--repo", "cognivex006/" + repo, "--pattern", pattern, "--dir", str(raw)], check=True)
    if repo == "axiom-skills":
        # Pattern selection retains the exact artifact-name directory in gh.
        shutil.copytree(raw / ("portable-skills-" + revision), directory)
    else:
        source = directory.with_name(repo + "-source")
        source.mkdir()
        def git(*words):
            subprocess.run(["git", *words], cwd=source, check=True, stdout=subprocess.DEVNULL)
        git("init", "--quiet")
        git("remote", "add", "origin", "https://github.com/cognivex006/" + repo + ".git")
        git("config", "core.autocrlf", "false")
        git("config", "core.sparseCheckout", "true")
        (source / ".git/info/sparse-checkout").write_text("/release/\n/Cargo.toml\n/pyproject.toml\n/Changelog.md\n", encoding="utf-8")
        git("fetch", "--quiet", "--depth=1", "--filter=blob:none", "origin", revision)
        git("checkout", "--quiet", "--detach", revision)
        collector = "collect_assets.py" if repo == "axiom-graphd" else "collect_portable_assets.py"
        subprocess.run([sys.executable, str(source / "release" / collector), "--input", str(raw), "--source-revision", revision, "--out", str(directory)], check=True)
    verify_payload(directory, revision)
    return revision
