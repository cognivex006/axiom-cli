"""Retained-generation recovery must validate before changing live pointers."""
import argparse
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import shutil
from unittest.mock import patch
import zipfile

SPEC = importlib.util.spec_from_file_location(
    "windows_runtime", Path(__file__).resolve().parents[1] / "packaging/windows/Provision-McpRuntime.py")
runtime = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime)


class RetainedRuntime(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base / "installed/mcp-runtime"
        self.args = argparse.Namespace(root=self.base / "installed", version="0.1.11",
            source_revision="a" * 40, interrupt_before_activate=False)
        python = self.base / "python.zip"
        with zipfile.ZipFile(python, "w") as z:
            z.writestr("python.exe", b"verified interpreter")
        pip = self.base / "pip.whl"
        pip.write_bytes(b"pip")
        bundle = self.base / "inputs.tar"
        manifest = {"source_revision": self.args.source_revision, "platform": "windows-x64",
            "python": "3.13", "owner_wheel": {"name": "axiom_mcp-0.1.11-py3-none-any.whl",
            "sha256": hashlib.sha256(b"wheel").hexdigest()}}
        self.members = {"input-manifest.json": json.dumps(manifest).encode(),
                        manifest["owner_wheel"]["name"]: b"wheel"}
        with tarfile.open(bundle, "w") as tar:
            for name, data in self.members.items():
                info = tarfile.TarInfo(name)
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))
        for name, path in (("runtime", python), ("pip_wheel", pip), ("inputs", bundle)):
            setattr(self.args, name, path)
            setattr(self.args, name + "_sha256", runtime.sha(path))
        identity = {name + "_sha256": getattr(self.args, name + "_sha256")
                    for name in ("runtime", "pip_wheel", "inputs")}
        self.key = hashlib.sha256(json.dumps(
            [self.args.version, self.args.source_revision, identity], sort_keys=True).encode()).hexdigest()[:24]
        self.version = self.root / "versions" / self.key
        self.bin = self.version / "venv/Scripts"
        self.bin.mkdir(parents=True)
        (self.bin / "python.exe").write_bytes(b"verified interpreter")
        (self.bin / "axmp.exe").write_bytes(b"verified launcher")
        for name, data in self.members.items():
            path = self.version / "packages" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        self.seal()

    def seal(self):
        runtime.atomic_json(self.version / "owner.json", {"owner": "axiom-cli",
            "generation": self.key, "files": runtime.contents(self.version)})

    def provision(self):
        with patch.object(runtime, "PYTHON_SHA256", self.args.runtime_sha256), \
             patch.object(runtime, "PIP_SHA256", self.args.pip_wheel_sha256), \
             patch.object(runtime, "command", return_value=json.dumps({"version": "0.1.11"})):
            return runtime.provision(self.args)

    def test_retained_without_current_reactivated_then_idempotent(self):
        before = runtime.contents(self.version)
        self.assertEqual(self.provision()["status"], "reactivated")
        self.assertEqual(runtime.contents(self.version), before)
        self.assertEqual(self.provision()["status"], "already_current")

    def test_changed_inventory_refuses_without_pointer_changes(self):
        (self.bin / "axmp.exe").write_bytes(b"changed")
        with self.assertRaises(ValueError):
            self.provision()
        self.assertFalse((self.root / "current.json").exists())
        self.assertFalse((self.root / "previous.json").exists())

    def test_reactivation_preserves_previous_runtime_and_rollback(self):
        other_key = "b" * 24
        other = self.root / "versions" / other_key
        shutil.copytree(self.version, other)
        runtime.atomic_json(other / "owner.json", {"owner": "axiom-cli", "generation": other_key,
                                                  "files": runtime.contents(other)})
        old = {"generation": other_key, "version": "0.1.10",
               "python_sha256": runtime.sha(other / "venv/Scripts/python.exe"),
               "executable_sha256": runtime.sha(other / "venv/Scripts/axmp.exe"),
               "inputs_sha256": "1" * 64, "runtime_sha256": "2" * 64,
               "source_revision": "c" * 40}
        runtime.atomic_json(self.root / "current.json", old)
        self.assertEqual(self.provision()["status"], "reactivated")
        self.assertEqual(runtime.read_json(self.root / "previous.json"), old)
        self.assertEqual(runtime.rollback(self.root)["version"], "0.1.10")
        self.assertEqual(self.provision()["status"], "reactivated")

    def test_interrupted_reactivation_changes_no_pointers_or_payload(self):
        self.args.interrupt_before_activate = True
        before = runtime.contents(self.version)
        with self.assertRaises(InterruptedError):
            self.provision()
        self.assertFalse((self.root / "current.json").exists())
        self.assertFalse((self.root / "previous.json").exists())
        self.assertEqual(runtime.contents(self.version), before)

    def test_partial_and_foreign_generations_are_preserved(self):
        for owner in ({"owner": "axiom-cli", "generation": self.key, "creating": True},
                      {"owner": "foreign", "generation": self.key, "files": {}}):
            runtime.atomic_json(self.version / "owner.json", owner)
            with self.assertRaises(ValueError):
                self.provision()
            self.assertEqual(runtime.read_json(self.version / "owner.json"), owner)
            self.assertFalse((self.root / "current.json").exists())

    def test_resealed_wrong_input_or_interpreter_refused(self):
        for path in (self.version / "packages/input-manifest.json", self.bin / "python.exe"):
            original = path.read_bytes()
            path.write_bytes(b"different")
            self.seal()
            with self.assertRaises(ValueError):
                self.provision()
            self.assertFalse((self.root / "current.json").exists())
            path.write_bytes(original)
            self.seal()


if __name__ == "__main__":
    unittest.main()
