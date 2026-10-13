#!/usr/bin/env python3
"""Generate the per-release one-line bootstrappers `install.ps1`/`install.sh` and `update.ps1`/`update.sh`.

The generator fills `installers/oneline/{install,update}.{ps1,sh}.in` with one release's exact tag, archive
names and SHA-256 values taken from that release's `SHA256SUMS`. It is deterministic: the same
inputs produce byte-identical scripts (LF line endings, no timestamps, no environment input).

    python packaging/oneline/generate_bootstrappers.py --tag v0.1.3 --sums <SHA256SUMS> --out <dir>
        [--base-url https://github.com/orchex006/axiom-cli/releases/download]

`--base-url` exists so a staged release can be served from a test HTTP server; the published
scripts use the canonical GitHub release download URL. The scripts only ever download the
embedded tag (or an explicitly pinned tag's own script); they never resolve `latest`.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TEMPLATES = ROOT / "installers" / "oneline"
CANONICAL_BASE_URL = "https://github.com/orchex006/axiom-cli/releases/download"
TAG = re.compile(r"^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$")
DIGEST = re.compile(r"^[0-9a-f]{64}$")
SCRIPTS = ("install.ps1", "install.sh", "update.ps1", "update.sh")
PLATFORMS = {
    "WINDOWS": ("windows-x64", "zip"),
    "LINUX": ("linux-x64", "tar.gz"),
    "MACOS": ("macos-x64", "tar.gz"),
}


class GenerationError(ValueError):
    pass


def read_sums(text: str) -> dict[str, str]:
    sums = {}
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            continue
        parts = line.split(None, 1)
        if len(parts) != 2 or not DIGEST.match(parts[0]):
            raise GenerationError(f"SHA256SUMS line {number} is not '<sha256>  <name>'")
        sums[parts[1].lstrip("*").strip()] = parts[0]
    return sums


def values(tag: str, sums: dict[str, str], base_url: str) -> dict[str, str]:
    if not TAG.match(tag):
        raise GenerationError(f"tag '{tag}' is not an immutable release tag vX.Y.Z; 'latest' and branches are refused")
    if not re.match(r"^https?://[^\s'\"`$]+$", base_url) or "latest" in base_url.split("/"):
        raise GenerationError(f"base URL '{base_url}' is not a plain http(s) URL without 'latest'")
    version = tag[1:]
    out = {"TAG": tag, "BASE_URL": base_url.rstrip("/"), "CLI_PATH_WINDOWS": "axiom-cli.exe", "CLI_PATH_POSIX": "axiom-cli"}
    for key, (platform, extension) in PLATFORMS.items():
        asset = f"axiom-{version}-{platform}.{extension}"
        if asset not in sums:
            raise GenerationError(f"SHA256SUMS has no entry for {asset}")
        out[f"{key}_ASSET"] = asset
        out[f"{key}_SHA256"] = sums[asset]
    return out


def render(template: str, mapping: dict[str, str]) -> str:
    text = template.replace("\r\n", "\n")
    for key, value in mapping.items():
        text = text.replace(f"@@{key}@@", value)
    left = re.findall(r"@@[A-Z0-9_]+@@", text)
    if left:
        raise GenerationError(f"unfilled placeholders: {sorted(set(left))}")
    return text


def generate(tag: str, sums_text: str, base_url: str = CANONICAL_BASE_URL) -> dict[str, bytes]:
    mapping = values(tag, read_sums(sums_text), base_url)
    return {
        name: render((TEMPLATES / f"{name}.in").read_text(encoding="utf-8"), mapping).encode("utf-8")
        for name in SCRIPTS
    }


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--tag", required=True)
    ap.add_argument("--sums", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--base-url", default=CANONICAL_BASE_URL)
    args = ap.parse_args(argv)
    try:
        scripts = generate(args.tag, args.sums.read_text(encoding="utf-8"), args.base_url)
    except (GenerationError, OSError) as error:
        print(f"generate_bootstrappers: {error}", file=sys.stderr)
        return 2
    args.out.mkdir(parents=True, exist_ok=True)
    for name, data in scripts.items():
        (args.out / name).write_bytes(data)
        print(f"{name}  {len(data)} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
