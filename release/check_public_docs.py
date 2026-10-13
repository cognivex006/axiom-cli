#!/usr/bin/env python3
"""Reject internal tracking identifiers in published CLI documentation and text."""
from __future__ import annotations

import argparse
import ast
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INTERNAL_ID = re.compile(r"(?<![A-Za-z0-9])(?:[A-Z]-\d{3}(?:-\d+)?|ADR-\d{4}|R\d{2}|W\d+-[A-Z][A-Z-]+)(?![A-Za-z0-9])")


def findings(label: str, text: str) -> list[str]:
    return [f"{label}:{text.count(chr(10), 0, match.start()) + 1}: internal identifier {match.group()}"
            for match in INTERNAL_ID.finditer(text)]


def rust_strings(text: str) -> str:
    """Read product string literals, skipping comments and the trailing unit-test module."""
    text = text.split("#[cfg(test)]", 1)[0]
    tokens = re.compile(r'//[^\n]*|/\*|"(?:\\.|[^"\\])*"', re.S)
    strings = []
    cursor = 0
    while match := tokens.search(text, cursor):
        value = match.group()
        cursor = match.end()
        if value == "/*":
            depth = 1
            while depth:
                comment = re.search(r'/\*|\*/', text[cursor:])
                if not comment:
                    cursor = len(text)
                    break
                depth += 1 if comment.group() == "/*" else -1
                cursor += comment.end()
        elif value.startswith('"'):
            strings.append(value)
    return "\n".join(strings)


def public_files(root: Path) -> list[Path]:
    paths = [root / name for name in ("README.md", "Changelog.md", "Development.md", "AGENTS.md")]
    for directory in ("docs", "release", "packaging", "installers"):
        paths.extend((root / directory).rglob("*.md"))
    # Native installer scripts and their generated templates ship in release archives.
    paths.extend(p for p in (root / "installers").rglob("*")
                 if p.is_file() and p.suffix in (".sh", ".ps1", ".in"))
    return sorted(set(paths))


def check(root: Path, assets: Path | None = None, notes: Path | None = None) -> list[str]:
    errors = []
    for path in public_files(root):
        if not path.is_file():
            errors.append(f"missing public document: {path.relative_to(root)}")
            continue
        errors.extend(findings(str(path.relative_to(root)), path.read_text(encoding="utf-8-sig")))
    for directory in ("release", "packaging"):
        for path in sorted((root / directory).rglob("*.py")):
            tree = ast.parse(path.read_text())
            calls = [node for node in ast.walk(tree) if isinstance(node, ast.Call)]
            for call in calls:
                if not isinstance(call.func, ast.Attribute):
                    continue
                for keyword in call.keywords:
                    if keyword.arg not in ("description", "help"):
                        continue
                    value = keyword.value
                    if isinstance(value, ast.Constant) and isinstance(value.value, str):
                        text = value.value
                    elif any(isinstance(node, ast.Name) and node.id == "__doc__" for node in ast.walk(value)):
                        text = ast.get_docstring(tree) or ""
                    else:
                        continue
                    errors.extend(findings(str(path.relative_to(root)) + " (tool help)", text))
    for path in sorted((root / "src").rglob("*.rs")):
        errors.extend(findings(str(path.relative_to(root)) + " (product strings)", rust_strings(path.read_text())))
    for path in sorted((root / ".github/workflows").glob("*.yml")):
        names = "\n".join(re.findall(r"^\s*(?:-\s*)?name:\s*(.+)$", path.read_text(), re.M))
        errors.extend(findings(str(path.relative_to(root)) + " (visible names)", names))
    changelog = (root / "Changelog.md").read_text()
    if changelog.count("\n## Unreleased\n") != 1 or not changelog.startswith("# Changelog\n"):
        errors.append("Changelog.md: require one title and one Unreleased section")
    versions = re.findall(r"^## (\d+\.\d+\.\d+)\s*$", changelog, re.M)
    keys = [tuple(map(int, version.split("."))) for version in versions]
    if not versions or keys != sorted(set(keys), reverse=True):
        errors.append("Changelog.md: releases must be unique and in descending version order")
    if assets:
        # Opaque archives are hash-verified by the distribution collector; scan generated text assets.
        for name in ("install.sh", "install.ps1", "update.sh", "update.ps1", "channel.json"):
            path = assets / name
            if not path.is_file():
                errors.append(f"missing generated release asset: {name}")
            else:
                errors.extend(findings(f"release assets/{name}", path.read_text(encoding="utf-8-sig")))
    if notes:
        errors.extend(findings("rendered release notes", notes.read_text()))
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--assets", type=Path)
    parser.add_argument("--notes", type=Path)
    args = parser.parse_args()
    errors = check(args.root, args.assets, args.notes)
    if errors:
        print("\n".join(errors))
        return 1
    print("Public documentation, product text and selected release assets: no internal identifiers.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
