"""Collect three source-bound native distributions without replacing outputs."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil

VERSION = (Path(__file__).resolve().parents[1] / "VERSION").read_text(encoding="utf-8").strip()


def collect(root, source, output, platforms=None):
    supported = {"windows-x64", "linux-x64", "macos-x64"}
    required = supported if platforms is None else set(platforms)
    if not required or not required <= supported:
        raise ValueError("invalid declared native platform scope")
    if output.exists():
        raise ValueError("new output required")
    selected = []
    lanes = set()
    for proof in root.rglob("native-proof.json"):
        data = json.loads(proof.read_bytes())
        lane = data["platform"]
        if (
            lane not in required
            or lane in lanes
            or data["source_revision"] != source
            or data["execution"] != "native"
            or data["version"] != VERSION
        ):
            raise ValueError("native proof source/lane mismatch")
        if not data["cases"] or any(x["exit_code"] != 0 for x in data["cases"]):
            raise ValueError("native case failed")
        archive = proof.parent / data["archive"]
        if (
            Path(data["archive"]).name != data["archive"]
            or hashlib.sha256(archive.read_bytes()).hexdigest()
            != data["archive_sha256"]
        ):
            raise ValueError("distribution archive changed")
        lanes.add(lane)
        for p in proof.parent.iterdir():
            if p.is_file():
                if p.name in {"native-proof.json", "progress-parity.json"}:
                    name = lane + "-" + p.name
                else:
                    name = p.name
                selected.append((p, name))
    if lanes != required:
        raise ValueError("all declared native lanes required")
    if len({n for _, n in selected}) != len(selected):
        raise ValueError("duplicate asset name")
    output.mkdir(parents=True)
    for p, name in selected:
        shutil.copyfile(p, output / name)
    (output / "SOURCE-REVISION.txt").write_text(
        source + "\n", encoding="utf-8", newline="\n"
    )
    (output / "SHA256SUMS").write_text(
        "".join(
            hashlib.sha256(p.read_bytes()).hexdigest() + "  " + p.name + "\n"
            for p in sorted(output.iterdir())
        ),
        encoding="utf-8",
        newline="\n",
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--source-revision", required=True)
    parser.add_argument("--platforms", help="explicit owner-approved native scope, comma separated")
    a = parser.parse_args()
    collect(a.input, a.source_revision, a.out, a.platforms.split(",") if a.platforms else None)
