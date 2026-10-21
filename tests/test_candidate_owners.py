"""Candidate composition cannot substitute moving/foreign/failed owner CI inputs."""
import copy
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("candidate_owners", Path(__file__).resolve().parents[1] / "release/candidate_owners.py")
owner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(owner)


class CandidateOwnerTests(unittest.TestCase):
    def test_canonical_successful_push_is_bound_to_explicit_source_and_run(self):
        pin = dict(run_id=123, source_revision="a" * 40)
        observed = dict(id=123, head_sha="a" * 40, head_repository=dict(full_name="cognivex006/axiom-mcp"), path=".github/workflows/mcp-ci.yml", event="push", status="completed", conclusion="success")
        self.assertEqual(owner.checked_run("axiom-mcp", pin, observed), "a" * 40)
        for key, bad in [("id", 124), ("head_sha", "b" * 40), ("head_repository", dict(full_name="foreign/axiom-mcp")), ("path", ".github/workflows/other.yml"), ("event", "pull_request"), ("status", "in_progress"), ("conclusion", "failure")]:
            changed = copy.deepcopy(observed); changed[key] = bad
            with self.subTest(key=key), self.assertRaises(ValueError):
                owner.checked_run("axiom-mcp", pin, changed)
        for bad in [dict(run_id=123, source_revision="main"), dict(run_id=True, source_revision="a" * 40), dict(run_id=0, source_revision="a" * 40), {}]:
            with self.assertRaises(ValueError): owner.checked_run("axiom-mcp", bad, observed)

    def test_checksums_cover_exact_payload_and_source(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "SOURCE-REVISION.txt").write_text("a" * 40 + "\n")
            (root / "component.zip").write_bytes(b"verified owner bytes")
            sums = "".join(hashlib.sha256(p.read_bytes()).hexdigest() + "  " + p.name + "\n" for p in sorted(root.iterdir()))
            (root / "SHA256SUMS").write_text(sums)
            owner.verify_payload(root, "a" * 40)
            with self.assertRaises(ValueError): owner.verify_payload(root, "b" * 40)
            (root / "extra").write_bytes(b"undeclared")
            with self.assertRaises(ValueError): owner.verify_payload(root, "a" * 40)
            (root / "extra").unlink()
            (root / "component.zip").write_bytes(b"changed")
            with self.assertRaises(ValueError): owner.verify_payload(root, "a" * 40)
            (root / "SHA256SUMS").write_text("0" * 64 + "  ../escape\n")
            with self.assertRaises(ValueError): owner.verify_payload(root, "a" * 40)


if __name__ == "__main__": unittest.main()
