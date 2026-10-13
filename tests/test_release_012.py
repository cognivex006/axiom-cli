"""Distribution delivery refuses unsafe or changed owner payloads."""

import importlib.util
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def load(name):
    spec = importlib.util.spec_from_file_location(
        name, ROOT / "release" / (name + ".py")
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


composer = load("compose_012")
collector = load("collect_distribution")


class ReleaseTests(unittest.TestCase):
    def test_archive_traversal_and_duplicate_members_refuse(self):
        for names in [["../outside"], ["same", "same"]]:
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                archive = root / "bad.zip"
                with zipfile.ZipFile(archive, "w") as stream:
                    for name in names:
                        stream.writestr(name, b"data")
                with self.assertRaises(ValueError):
                    composer.unpack(archive, root / "out")
                self.assertFalse((root / "outside").exists())

    def inputs(self, root):
        for lane in ["windows-x64", "linux-x64", "macos-x64"]:
            target = root / lane
            target.mkdir(parents=True)
            archive = target / (lane + ".zip")
            archive.write_bytes(b"synthetic collector fixture")
            (target / "native-proof.json").write_text(
                json.dumps(
                    {
                        "platform": lane,
                        "source_revision": "a" * 40,
                        "execution": "native",
                        "version": composer.VERSION,
                        "cases": [{"exit_code": 0}],
                        "archive": archive.name,
                        "archive_sha256": hashlib.sha256(
                            archive.read_bytes()
                        ).hexdigest(),
                    }
                )
            )

    def test_three_checked_archives_collect_with_source_and_checksums(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.inputs(root / "inputs")
            collector.collect(root / "inputs", "a" * 40, root / "out")
            self.assertTrue((root / "out/SHA256SUMS").is_file())

    def test_changed_archive_or_missing_lane_refuses_before_output(self):
        for condition in ["archive", "missing"]:
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.inputs(root / "inputs")
                if condition == "archive":
                    (root / "inputs/macos-x64/macos-x64.zip").write_bytes(b"changed")
                else:
                    (root / "inputs/macos-x64/native-proof.json").unlink()
                with self.assertRaises(ValueError):
                    collector.collect(root / "inputs", "a" * 40, root / "out")
                self.assertFalse((root / "out").exists())

    def test_existing_output_is_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.inputs(root / "inputs")
            out = root / "out"
            out.mkdir()
            (out / "human").write_text("keep")
            with self.assertRaises(ValueError):
                collector.collect(root / "inputs", "a" * 40, out)
            self.assertEqual((out / "human").read_text(), "keep")


if __name__ == "__main__":
    unittest.main()
