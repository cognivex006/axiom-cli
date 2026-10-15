"""Distribution delivery refuses unsafe or changed owner payloads."""

import importlib.util
import hashlib
import json
import os
import subprocess
import sys
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

    @unittest.skipUnless(hasattr(os, "getuid") and os.getuid() != 0, "non-root POSIX bootstrap")
    def test_bootstrap_binds_future_package_version_before_provision(self):
        with tempfile.TemporaryDirectory() as temporary:
            release = Path(temporary) / "release"
            runtime = release / "runtime"
            runtime.mkdir(parents=True)
            root = Path(temporary) / "fresh-root"
            version = "0.2.7"
            wheel = f"axiom_mcp-{version}-py3-none-any.whl"
            for name in ("python.tar.gz", "wheelhouse.tar.gz", wheel, "requirements.txt"):
                (runtime / name).write_bytes(b"isolated fixture input")
            (runtime / "bootstrap.py").write_bytes((ROOT / "release/bootstrap_linux.py").read_bytes())
            provision = "import json,sys\nfrom pathlib import Path\nPath(__file__).with_name('observed-argv.json').write_text(json.dumps(sys.argv[1:]))\nprint(json.dumps({'python':sys.executable}))\n"
            (runtime / "provision.py").write_text(provision)
            manifest = {"version": version, "mcp_revision": "a" * 40,
                        "files": {name: hashlib.sha256((runtime / name).read_bytes()).hexdigest()
                                  for name in ("python.tar.gz", "wheelhouse.tar.gz", wheel, "requirements.txt")}}
            (runtime / "manifest.json").write_text(json.dumps(manifest))
            cli = release / "axiom-cli"
            cli.write_text("#!/usr/bin/env python3\nimport json\nprint(json.dumps({'details':{'plan_digest':'a'*64},'code':0}))\n")
            cli.chmod(0o755)
            inventory = {"files": [{"path": f.relative_to(release).as_posix(),
                                    "sha256": hashlib.sha256(f.read_bytes()).hexdigest()}
                                   for f in release.rglob("*") if f.is_file()]}
            (release / "DISTRIBUTION-FILES.json").write_text(json.dumps(inventory))
            argv = [sys.executable, str(runtime / "bootstrap.py"), "--root", str(root)]
            planned = subprocess.run([*argv, "--dry-run"], capture_output=True, text=True, check=True)
            plan = json.loads(planned.stdout)
            self.assertEqual(plan["version"], version)
            self.assertFalse(root.exists())
            applied = subprocess.run([*argv, "--apply", "--approve-digest", plan["plan_digest"]], capture_output=True, text=True, check=True)
            self.assertEqual(json.loads(applied.stdout)["version"], version)
            observed = json.loads((runtime / "observed-argv.json").read_text())
            self.assertEqual(observed[observed.index("--version") + 1], version)
            self.assertEqual(Path(observed[observed.index("--wheel") + 1]).name, wheel)

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


class MacOwnerPinTests(unittest.TestCase):
    """Synthetic packaging inputs exercise the real immutable-pin boundary."""

    def candidate(self):
        spec = importlib.util.spec_from_file_location(
            "mac_owner_candidate", ROOT / "packaging/macos/Build-EngineCandidate.py"
        )
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def owner(self, root, revision):
        payload = b"synthetic skill packaging fixture\n"
        path = root / "skills/axge/SKILL.md"
        path.parent.mkdir(parents=True)
        path.write_bytes(payload)
        manifest = root / "skills-manifest.json"
        manifest.write_text(json.dumps({
            "manifest_version": 1, "component": "axiom-skills",
            "component_version": "0.1.8", "spec_revision": revision,
            "files": [{"path": "skills/axge/SKILL.md", "bytes": len(payload),
                       "sha256": hashlib.sha256(payload).hexdigest()}],
        }), encoding="utf-8")
        return hashlib.sha256(manifest.read_bytes()).hexdigest()

    def test_approved_workflow_and_previous_release_pins_are_accepted(self):
        for revision in ("10b5e190695c75058fb176f35425ad01d840a4e8",
                         "73aec1d1ed190bcd7066574853340fe15102ebbe"):
            with self.subTest(revision=revision), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                digest = self.owner(root, revision)
                result = self.candidate().skills_bundle(root, "a" * 40, digest)
                self.assertEqual(result["spec_revision"], revision)

    def test_unreviewed_pin_and_changed_payload_are_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            digest = self.owner(root, "f" * 40)
            with self.assertRaisesRegex(ValueError, "spec revision differs"):
                self.candidate().skills_bundle(root, "a" * 40, digest)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            digest = self.owner(root, "10b5e190695c75058fb176f35425ad01d840a4e8")
            (root / "skills/axge/SKILL.md").write_text("changed")
            with self.assertRaisesRegex(ValueError, "payload has changed"):
                self.candidate().skills_bundle(root, "a" * 40, digest)


if __name__ == "__main__":
    unittest.main()
