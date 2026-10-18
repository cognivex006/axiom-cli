"""L-004 generator tests: determinism, embedded tag/digests and refusals (no network, no install)."""
from __future__ import annotations

import importlib.util
import re
import subprocess
import shutil
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("gen", ROOT / "packaging/oneline/generate_bootstrappers.py")
gen = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gen)

SUMS = "\n".join(
    [
        "1" * 64 + "  axiom-1.2.3-windows-x64.zip",
        "2" * 64 + "  axiom-1.2.3-linux-x64.tar.gz",
        "3" * 64 + "  axiom-1.2.3-macos-x64.tar.gz",
        "4" * 64 + "  SOURCE-REVISION.txt",
    ]
) + "\n"


class GeneratorTests(unittest.TestCase):
    def test_same_inputs_give_byte_identical_scripts(self):
        first = gen.generate("v1.2.3", SUMS)
        second = gen.generate("v1.2.3", SUMS.replace("\n", "\r\n"))
        self.assertEqual(first, second)
        for data in first.values():
            self.assertNotIn(b"\r\n", data)

    def test_scripts_embed_the_exact_tag_assets_and_digests(self):
        scripts = gen.generate("v1.2.3", SUMS)
        ps1 = scripts["install.ps1"].decode()
        sh = scripts["install.sh"].decode()
        self.assertIn("$AxiomTag = 'v1.2.3'", ps1)
        self.assertIn("$AxiomSha256 = '" + "1" * 64 + "'", ps1)
        self.assertIn("axiom-1.2.3-windows-x64.zip", ps1)
        self.assertIn("AXIOM_TAG='v1.2.3'", sh)
        self.assertIn("2" * 64, sh)
        self.assertIn("3" * 64, sh)
        for text in (ps1, sh):
            self.assertNotRegex(text, r"@@[A-Z_]+@@")
            # Downloads only ever use the embedded or an explicitly pinned tag, never `latest`.
            for line in text.splitlines():
                if line.lstrip().startswith("#"):
                    continue
                self.assertNotIn("latest", line)

    def test_moving_tags_and_missing_assets_are_refused(self):
        for tag in ("latest", "main", "1.2.3", "v1.2", "v1.2.3;rm"):
            with self.assertRaises(gen.GenerationError, msg=tag):
                gen.generate(tag, SUMS)
        with self.assertRaises(gen.GenerationError):
            gen.generate("v1.2.3", SUMS.replace("axiom-1.2.3-macos-x64.tar.gz", "other"))
        with self.assertRaises(gen.GenerationError):
            gen.generate("v1.2.3", "nothex  axiom-1.2.3-windows-x64.zip\n")
        with self.assertRaises(gen.GenerationError):
            gen.generate("v1.2.3", SUMS, "https://github.com/orchex006/axiom-cli/releases/latest/download")

    def test_posix_script_is_valid_sh(self):
        sh = shutil.which("sh")
        if not sh:
            self.skipTest("no sh on this host")
        for name in ("install.sh", "update.sh"):
            script = gen.generate("v1.2.3", SUMS)[name]
            result = subprocess.run([sh, "-n"], input=script, capture_output=True)
            self.assertEqual(result.returncode, 0, (name, result.stderr))


@unittest.skipUnless(shutil.which("sh"), "no sh on this host")
class InstallRootRefusalTests(unittest.TestCase):
    """M-001: force the root identity boundary; fail if any download/placement is attempted."""

    def test_root_is_refused_before_platform_probe_download_or_mktemp(self):
        import os
        import tempfile
        with tempfile.TemporaryDirectory(prefix="axiom-install-root-") as directory:
            home = Path(directory)
            shim = home / "bin"
            shim.mkdir()
            identity = shim / "id"
            identity.write_text('#!/bin/sh\n[ "$1" = -u ] || exit 99\necho 0\n')
            identity.chmod(0o755)
            for name in ("uname", "curl", "wget", "mktemp", "tar"):
                probe = shim / name
                probe.write_text('#!/bin/sh\necho "unexpected side effect" >&2\nexit 99\n')
                probe.chmod(0o755)
            script = home / "install.sh"
            script.write_bytes(gen.generate("v1.2.3", SUMS)["install.sh"])
            before = sorted(p.relative_to(home).as_posix() for p in home.rglob("*"))
            env = {k: v for k, v in os.environ.items() if not k.upper().startswith("AXIOM")}
            env.update(HOME=str(home), PATH=str(shim))
            result = subprocess.run([shutil.which("sh"), str(script), "--yes"],
                                    env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn("refusing to run as root", result.stderr)
            self.assertNotIn("unexpected side effect", result.stderr)
            self.assertEqual(before, sorted(p.relative_to(home).as_posix() for p in home.rglob("*")))


class UpdateBootstrapperTests(unittest.TestCase):
    """L-013 (ADR-0035): update.ps1 / update.sh are pinned, logic-free and state-aware."""

    def test_update_scripts_are_generated_with_the_install_scripts(self):
        scripts = gen.generate("v1.2.3", SUMS)
        self.assertEqual(set(scripts), {"install.ps1", "install.sh", "update.ps1", "update.sh"})
        ps1 = scripts["update.ps1"].decode()
        sh = scripts["update.sh"].decode()
        self.assertIn("$AxiomTag = 'v1.2.3'", ps1)
        self.assertIn("$AxiomSha256 = '" + "1" * 64 + "'", ps1)
        self.assertIn("AXIOM_TAG='v1.2.3'", sh)
        self.assertIn("2" * 64, sh)
        self.assertIn("3" * 64, sh)
        for text in (ps1, sh):
            self.assertNotRegex(text, r"@@[A-Z_]+@@")
            code = [line for line in text.splitlines() if not line.lstrip().startswith("#")]
            self.assertFalse([line for line in code if "latest" in line], "the update script must never resolve latest")
            # The installed case delegates to the installation's own CLI; the script has no version option.
            self.assertIn("update", "\n".join(code))
            self.assertNotIn("--version", "\n".join(code))
            for name in ("AXIOM_CLI_INSTALL_ROOT", "AXIOM_ENGINE_BIN", "AXIOM_HOME"):
                self.assertIn(name, text)
        self.assertIn("IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)", ps1)
        # ADR-0036: the short command `axm` on PATH also identifies an installed root.
        self.assertIn("'axm', 'axiom-cli'", ps1)
        self.assertIn("command -v axm", sh)
        self.assertIn("[Environment]::GetEnvironmentVariable($name, 'User')", ps1)
        self.assertIn('id -u', sh)

    def test_update_generation_is_deterministic(self):
        first = gen.generate("v1.2.3", SUMS)
        self.assertEqual(first["update.ps1"], gen.generate("v1.2.3", SUMS.replace("\n", "\r\n"))["update.ps1"])
        self.assertNotIn(b"\r\n", first["update.sh"])


@unittest.skipUnless(shutil.which("sh"), "no sh on this host")
class UpdateShBehaviourTests(unittest.TestCase):
    """Run the generated update.sh against fixture homes; nothing is downloaded or installed."""

    def setUp(self):
        import tempfile
        self.home = Path(tempfile.mkdtemp(prefix="axiom-update-test-"))
        self.script = self.home / "update.sh"
        self.script.write_bytes(gen.generate("v1.2.3", SUMS, "http://127.0.0.1:9")["update.sh"])

    def tearDown(self):
        shutil.rmtree(self.home, ignore_errors=True)

    def run_update(self, *args, extra=None):
        import os
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith("AXIOM")}
        env.update({"HOME": str(self.home), "XDG_DATA_HOME": str(self.home / "data"),
                    "PATH": os.environ.get("PATH", "")})
        env.update(extra or {})
        # Fixture homes must never dispatch to a real installation on the caller's PATH.
        # Keep the explicitly selected fixture bin and ordinary host utilities.
        names = ("axiom-cli", "axm", "axiom-cli.exe", "axm.exe")
        fixture = self.home.resolve()
        entries = []
        for entry in env["PATH"].split(os.pathsep):
            path = Path(entry).resolve()
            if path.is_relative_to(fixture) or not any((path / name).is_file() for name in names):
                entries.append(entry)
        env["PATH"] = os.pathsep.join(entries)
        return subprocess.run([shutil.which("sh"), str(self.script), *args], env=env, capture_output=True,
                              text=True, stdin=subprocess.DEVNULL, timeout=60)

    def tree(self):
        return sorted(p.relative_to(self.home).as_posix() for p in self.home.rglob("*"))

    def test_nothing_installed_refuses_without_change(self):
        before = self.tree()
        result = self.run_update()
        self.assertEqual(result.returncode, 1)
        self.assertIn("not installed", result.stderr)
        self.assertIn("http://127.0.0.1:9/v1.2.3/install.sh", result.stderr)
        self.assertEqual(self.tree(), before)

    def test_leftover_variables_refuse_without_change(self):
        for name in ("AXIOM_CLI_INSTALL_ROOT", "AXIOM_ENGINE_BIN", "AXIOM_HOME"):
            result = self.run_update(extra={name: "/somewhere"})
            self.assertEqual(result.returncode, 1, name)
            self.assertIn("leftover variables", result.stderr)
            self.assertIn(name + "=/somewhere", result.stderr)

    def test_current_layout_requires_verified_worker_before_any_update(self):
        root = self.home / "data" / "axiom"
        (root / "bin").mkdir(parents=True)
        (root / "installed.json").write_text("{}", encoding="utf-8")
        stub = root / "bin" / "axm"
        stub.write_bytes(b'#!/bin/sh\necho "stub axm $*"\nexit 7\n')
        stub.chmod(0o755)
        (root / "bin/graphd-engine").mkdir()
        (root / "bin/graphd-engine/axiom").write_bytes(b"owned current layout fixture")
        result = self.run_update("--yes")
        self.assertIn("preparing the verified", result.stdout)
        self.assertNotIn("stub axm update", result.stdout)
        self.assertEqual(result.returncode, 1, "unavailable verified worker must not run the old updater")

    def test_canonical_command_root_wins_over_a_separate_legacy_installation(self):
        import os
        roots = []
        for directory, command, marker in [("legacy", "axiom-cli", "legacy-root"), ("current", "axm", "current-root")]:
            root = self.home / directory
            (root / "bin").mkdir(parents=True)
            (root / "installed.json").write_text("{}")
            stub = root / "bin" / command
            stub.write_text('#!/bin/sh\necho "' + marker + ' $*"\n')
            stub.chmod(0o755)
            if command == "axm":
                (root / "bin/graphd-engine").mkdir()
                (root / "bin/graphd-engine/axiom").write_bytes(b"owned current layout fixture")
            roots.append(str(root / "bin"))
        result = self.run_update("--yes", extra={"PATH": os.pathsep.join([*roots, os.environ.get("PATH", "")])})
        self.assertEqual(result.returncode, 1, result.stderr)
        # Git sh can render a Windows temp directory as /tmp/... rather than C:\... .
        selected = result.stdout.replace("\\", "/")
        self.assertIn(self.home.name + "/current; preparing the verified", selected)
        self.assertNotIn(self.home.name + "/legacy", selected)
        self.assertNotIn("legacy-root", result.stdout)

    def test_older_command_layout_preserves_it_on_bootstrap_refusal(self):
        root = self.home / "data/axiom"
        (root / "bin").mkdir(parents=True)
        record = root / "installed.json"
        record.write_text("{}")
        legacy = root / "bin/axiom-cli"
        legacy.write_text('#!/bin/sh\necho "unexpected legacy updater"\nexit 99\n')
        legacy.chmod(0o755)
        before = self.tree()
        result = self.run_update("--yes")
        self.assertEqual(result.returncode, 1)
        self.assertIn("preparing the verified", result.stdout)
        self.assertNotIn("unexpected legacy updater", result.stdout)
        if sys.platform == "win32":
            self.assertIn("unsupported platform", result.stderr)
            self.assertNotIn("downloading v1.2.3", result.stdout)
        else:
            self.assertNotIn("downloading v1.2.3", result.stdout)
            self.assertEqual(result.stderr.count("axiom update: downloading v1.2.3"), 1)
            self.assertNotIn("\x1b", result.stderr)
            self.assertIn("download failed", result.stderr)
        self.assertEqual(record.read_text(), "{}")
        self.assertEqual(self.tree(), before)

    def test_legacy_bootstrap_root_takes_the_verified_install_path(self):
        current = self.home / "axiom" / "installs" / "ecosystem" / "current"
        current.parent.mkdir(parents=True)
        current.write_text("{}", encoding="utf-8")
        before = self.tree()
        result = self.run_update("--yes")
        self.assertIn("Axiom 0.1.2 bootstrap root", result.stdout)
        self.assertEqual(result.returncode, 1)
        # Either the host is not a published platform or the fixture URL cannot be fetched; both stop
        # before extraction, so nothing outside the temporary directory changed.
        self.assertRegex(result.stderr, r"unsupported platform|download failed|curl or wget|sha256sum")
        self.assertEqual(self.tree(), before)

    def test_unknown_option_is_refused(self):
        result = self.run_update("--version", "1.2.3")
        self.assertEqual(result.returncode, 1)
        self.assertIn("unknown option '--version'", result.stderr)


if __name__ == "__main__":
    unittest.main()
