"""Public documentation and generated release-note regression checks."""
from __future__ import annotations

import importlib.util
import json
import shutil
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def load(name, relative):
    spec = importlib.util.spec_from_file_location(name, ROOT / relative)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


guard = load("public_guard", "release/check_public_docs.py")
notes = load("public_notes", "release/render_release_notes.py")
assets = load("public_assets", "release/oneline_assets.py")


class PublicDocumentationTests(unittest.TestCase):
    def fixture(self, root):
        for name in ("README.md", "Changelog.md", "Development.md", "AGENTS.md"):
            shutil.copyfile(ROOT / name, root / name)
        shutil.copytree(ROOT / "docs", root / "docs")
        (root / "release").mkdir()
        shutil.copyfile(ROOT / "release/oneline-release-notes.md", root / "release/oneline-release-notes.md")
        (root / "src").mkdir()
        return root

    def test_real_public_surfaces_are_clean(self):
        self.assertEqual(guard.check(ROOT), [])

    def test_nested_document_and_installer_leaks_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.fixture(Path(directory))
            p = root / "docs/nested/new-guide.md"
            p.parent.mkdir()
            p.write_text("Install per K-001 and ADR-0033; requirement R22.\n")
            p = root / "installers/new/install.sh.in"
            p.parent.mkdir(parents=True)
            p.write_text("# Task O-012\n")
            workflow = root / ".github/workflows/test.yml"
            workflow.parent.mkdir(parents=True)
            workflow.write_text("name: N-001 release check\n")
            helper = root / "packaging/new/tool.py"
            helper.parent.mkdir(parents=True)
            helper.write_text('"""J-003 packaging helper."""\nimport argparse\nparser = argparse.ArgumentParser(description=__doc__)\n')
            errors = guard.check(root)
            for identifier in ("K-001", "ADR-0033", "R22", "O-012", "N-001", "J-003"):
                self.assertTrue(any(identifier in item for item in errors), errors)

    def test_normal_versions_exit_codes_and_public_prs_are_allowed(self):
        self.assertEqual(guard.findings("guide", "Version 0.1.5, SHA-256, x86_64; exit 4. Fix #123. https://github.com/example/project/pull/123"), [])

    def test_comments_are_internal_but_product_strings_are_checked(self):
        source = '// K-001 "internal comment"\n/* ADR-0033 /* J-007 */ nested */\nlet s = "Follow O-012";\n'
        self.assertEqual([x.split()[-1] for x in guard.findings("product", guard.rust_strings(source))], ["O-012"])
        self.assertTrue(guard.findings("product", guard.rust_strings('let s = r#"https://example.test/K-001"#;')))
        self.assertFalse(guard.findings("product", guard.rust_strings('// "K-001"\nlet s = "fine";')))

    def test_generated_assets_are_clean_and_tampering_is_detected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            for platform, extension in assets.PLATFORMS:
                (path / f"axiom-0.1.5-{platform}.{extension}").write_bytes(b"fixture archive")
            assets.build(path, "v0.1.5")
            self.assertEqual(guard.check(ROOT, path), [])
            channel = json.loads((path / "channel.json").read_text())
            channel["note"] = "See ADR-0033"
            (path / "channel.json").write_text(json.dumps(channel))
            self.assertTrue(any("channel.json" in item for item in guard.check(ROOT, path)))
            (path / "update.sh").write_text("# K-001\n")
            self.assertTrue(any("update.sh" in item for item in guard.check(ROOT, path)))

    def test_missing_assets_and_duplicate_unreleased_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = self.fixture(Path(directory))
            p = root / "Changelog.md"
            p.write_text(p.read_text() + "\n## Unreleased\n")
            errors = guard.check(root, root / "missing-assets")
            self.assertTrue(any("one Unreleased" in item for item in errors))
            self.assertEqual(sum("missing generated" in item for item in errors), 5)

    def test_notes_010_agent_setup_and_historical_commands(self):
        current = notes.render("v0.1.10")
        self.assertIn("The public commands are `axm`, `axge` and `axmp`.", current)
        self.assertIn("axm skills install codex", current)
        self.assertIn("180 minutes", current)
        self.assertNotIn("axm skills install claude", current)
        latest = notes.render("v0.1.11")
        self.assertIn("axm skills install claude", latest)
        self.assertIn("start|list|status|stop", latest)
        previous = notes.render("v0.1.9")
        self.assertIn("The public commands are `axm`, `axsk`, `axge` and `axmp`.", previous)
        self.assertNotIn("## Agent setup", previous)

    def test_notes_select_only_requested_release_and_pin_install_links(self):
        text = notes.render("v0.1.5")
        self.assertIn("## New Features", text)
        self.assertIn("v0.1.4...v0.1.5", text)
        self.assertIn("/download/v0.1.5/install.sh", text)
        self.assertNotIn("/latest/download/", text)
        self.assertNotIn("## Unreleased", text)
        self.assertNotIn("## 0.1.4", text)
        self.assertEqual(guard.findings("notes", text), [])
        older = notes.render("v0.1.4")
        self.assertNotIn("Add `axm` as the short command", older)
        self.assertNotIn("`axm", older)
        old = notes.render("v0.1.3")
        self.assertNotIn("/update.sh", old)
        self.assertIn("axiom-cli update", old)
        self.assertIn("runtime/bootstrap.sh", notes.render("v0.1.1"))

    def test_prepared_version_notes_match_the_renderer(self):
        for tag in ("v0.1.3", "v0.1.4", "v0.1.5"):
            self.assertEqual((ROOT / "release" / f"{tag}-notes.md").read_text(), notes.render(tag))

    def test_unknown_and_moving_tags_are_refused(self):
        for tag in ("latest", "main", "v0.1.99", "v0.1.5-beta", "0.1.5"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                notes.render(tag)


if __name__ == "__main__":
    unittest.main()
