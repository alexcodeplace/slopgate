#!/usr/bin/env python3
"""Mutation tests for the trusted-base metadata review, without network access."""
import subprocess
import tempfile
import unittest
from pathlib import Path
from spec_drift import HEADINGS, review


class DriftContract(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="slopgate-drift-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.git("init", "--quiet")
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "user.name", "Drift fixture")
        self.write("docs/specs/contract.md", "original contract\n")
        self.commit("base")
        self.base = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        result = subprocess.run(["git", *args], cwd=self.root, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def write(self, path, contents):
        file = self.root / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(contents, encoding="utf-8")

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", message)
        return self.git("rev-parse", "HEAD").strip()

    def test_protected_change_without_new_adr_is_blocked(self):
        self.write("docs/specs/contract.md", "weakened contract\n")
        result = review(self.root, self.base, self.commit("change"))
        self.assertEqual(result["status"], "blocked")
        self.assertTrue(result["humanApprovalStillRequired"])

    def test_deleting_governance_and_adding_empty_adr_is_blocked(self):
        (self.root / "docs/specs/contract.md").unlink()
        self.write("docs/adr/0001-empty.md", "## Decision\nNo details.\n")
        self.assertEqual(review(self.root, self.base, self.commit("delete"))["status"], "blocked")

    def test_complete_adr_is_not_human_approval_and_candidate_code_never_runs(self):
        self.write("docs/specs/contract.md", "changed contract\n")
        self.write("scripts/spec_drift.py", "raise RuntimeError('candidate must not execute')\n")
        self.write("docs/adr/0001-change.md", "\n".join(f"## {heading}\nThis is a detailed review fixture, not approval by an implementation agent.\n" for heading in HEADINGS))
        result = review(self.root, self.base, self.commit("review proposal"))
        self.assertEqual(result["status"], "passed")
        self.assertTrue(result["humanApprovalStillRequired"])
        self.assertEqual(result["decisionRecords"], ["docs/adr/0001-change.md"])

    def test_ref_arguments_are_not_command_line_options(self):
        with self.assertRaises(ValueError):
            review(self.root, "--help", self.base)

    def test_ordinary_implementation_change_does_not_require_architecture_adr(self):
        self.write("src/feature.rs", "fn feature() {}\n")
        self.assertEqual(review(self.root, self.base, self.commit("feature"))["status"], "passed")


if __name__ == "__main__":
    unittest.main(verbosity=2)
