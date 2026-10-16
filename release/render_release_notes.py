#!/usr/bin/env python3
"""Render version-specific release notes from Changelog and the installation guide."""
from __future__ import annotations

import argparse
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def render(tag: str, root: Path = ROOT) -> str:
    if not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        raise ValueError("release tag must be an exact vMAJOR.MINOR.PATCH version")
    version = tag[1:]
    changelog = (root / "Changelog.md").read_text()
    sections = re.findall(r"^## (\d+\.\d+\.\d+)\s*\n(.*?)(?=^## |\Z)", changelog, re.M | re.S)
    selected = [body.strip() for name, body in sections if name == version]
    if len(selected) != 1:
        raise ValueError(f"require exactly one Changelog entry for {version}")
    changes = re.sub(r"^### ", "## ", selected[0], flags=re.M)
    guide = (root / "release/oneline-release-notes.md").read_text().split("\n## Changes\n", 1)[0]
    guide = guide.split("\n", 1)[1].strip()
    # Preserve the actual command layout of immutable historical releases.
    if tuple(map(int, version.split("."))) < (0, 1, 10):
        guide = guide.replace("The public commands are `axm`, `axge` and `axmp`. Existing owned `axsk` terminal commands remain deprecated compatibility forwarders; `$axsk` is the agent skill.",
                              "The public commands are `axm`, `axsk`, `axge` and `axmp`.")
        guide = re.sub(r"## Agent setup\n.*?(?=## Release integrity)", "", guide, flags=re.S)
        guide = guide.replace("## Release integrity\n\n", "")
    if tuple(map(int, version.split("."))) <= (0, 1, 6):
        guide = guide.replace("The public commands are `axm`, `axsk`, `axge` and `axmp`.",
                              "`axm` and `axiom-cli` run the same program. `axiom` is the graph engine's separate operations CLI.")
    if tuple(map(int, version.split("."))) < (0, 1, 3):
        historical = root / f"release/{tag}-notes.md"
        if historical.is_file():
            return historical.read_text()
        return f"# Axiom {version}\n\n{changes}\n"
    if tuple(map(int, version.split("."))) < (0, 1, 4):
        guide = re.sub(r"## Update\n.*?(?=## Commands)",
                       "## Update\n\nRun `axiom-cli update` from the installed release.\n\n", guide, flags=re.S)
    if tuple(map(int, version.split("."))) < (0, 1, 5):
        guide = guide.replace("`axm` and `axiom-cli` run the same program.",
                              "Use `axiom-cli` to manage this release.")
        guide = re.sub(r"\baxm\b", "axiom-cli", guide)
    # Published notes stay attached to their immutable version, even after latest advances.
    guide = guide.replace("/latest/download/", f"/download/{tag}/")
    return f"# Axiom {version}\n\n{changes}\n\n{guide}\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    try:
        text = render(args.tag)
    except ValueError as error:
        parser.error(str(error))
    # Refuse accidental overwrite of a prepared note; no release mutation happens here.
    with args.out.open("x", encoding="utf-8", newline="\n") as output:
        output.write(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
