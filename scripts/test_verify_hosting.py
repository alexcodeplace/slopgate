#!/usr/bin/env python3
"""Offline mutation checks for hosting verification; no API calls or writes."""
from __future__ import annotations
import copy
import importlib.util
import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("hosting_verifier", ROOT / "scripts/verify-hosting.py")
assert SPEC is not None and SPEC.loader is not None
VERIFIER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFIER)


class HostingContract(unittest.TestCase):
    def setUp(self) -> None:
        self.policy = json.loads((ROOT / "docs/architecture/hosting-protection.json").read_text())
        self.protection = copy.deepcopy(self.policy)
        for key in ("enforce_admins", "required_conversation_resolution", "allow_force_pushes", "allow_deletions"):
            self.protection[key] = {"enabled": self.protection[key]}
        self.people = [
            {"login": "implementation-agent", "type": "User", "role_name": "write", "permissions": {"push": True, "admin": False, "maintain": False}},
            {"login": "human-reviewer", "type": "User", "role_name": "admin", "permissions": {"push": True, "admin": True}},
        ]
        self.owners = "# protected ownership\n* @human-reviewer\n"
        self.workflow = {"path": ".github/workflows/spec-drift.yml", "state": "active"}

    def issues(self) -> list[str]:
        return VERIFIER.problems(self.policy, self.protection, self.people, "implementation-agent", self.owners, self.workflow)

    def test_complete_independent_controls_pass_without_network(self) -> None:
        self.assertEqual(self.issues(), [])

    def test_context_names_alone_do_not_prove_trusted_status_source(self) -> None:
        del self.protection["required_status_checks"]["checks"]
        self.assertTrue(any("GitHub App" in issue for issue in self.issues()))

    def test_wrong_unbound_or_any_app_source_is_rejected(self) -> None:
        for app_id in (None, -1, 0, 123456):
            with self.subTest(app_id=app_id):
                self.protection["required_status_checks"]["checks"][0]["app_id"] = app_id
                self.assertTrue(any("GitHub App" in issue for issue in self.issues()))

    def test_missing_stale_latest_push_or_owner_review_is_not_verified(self) -> None:
        original = copy.deepcopy(self.protection)
        for key in ("dismiss_stale_reviews", "require_code_owner_reviews", "require_last_push_approval"):
            with self.subTest(key=key):
                self.protection = copy.deepcopy(original)
                self.protection["required_pull_request_reviews"][key] = False
                self.assertTrue(any(key in issue for issue in self.issues()))
        self.protection = copy.deepcopy(original)
        self.protection["required_pull_request_reviews"]["required_approving_review_count"] = 0
        self.assertTrue(any("independent review" in issue for issue in self.issues()))

    def test_admin_maintainer_and_unreviewed_custom_agent_roles_are_rejected(self) -> None:
        for role in ("admin", "maintain", "custom-review-bypass"):
            with self.subTest(role=role):
                self.people[0]["role_name"] = role
                self.assertTrue(self.issues())
        self.people[0]["role_name"] = "write"
        self.people[0]["permissions"]["maintain"] = True
        self.assertTrue(self.issues())

    def test_agent_ownership_and_non_independent_owners_are_rejected(self) -> None:
        for owners in ("* @implementation-agent", "* @IMPLEMENTATION-AGENT", "* @unknown", "* @organization/team", "src/** @human-reviewer", "* @human-reviewer\n.github/**", "", None):
            with self.subTest(owners=owners):
                self.owners = owners
                self.assertTrue(self.issues())

    def test_no_distinct_human_writer_cannot_pass(self) -> None:
        self.people[1]["permissions"]["push"] = False
        self.assertTrue(self.issues())
        self.people[1]["permissions"]["push"] = True
        self.people[1]["type"] = "Bot"
        self.assertTrue(self.issues())

    def test_absent_or_disabled_trusted_workflow_is_incomplete(self) -> None:
        for workflow in (None, {}, {"state": "disabled_manually", "path": ".github/workflows/spec-drift.yml"}, {"state": "active", "path": ".github/workflows/candidate.yml"}):
            with self.subTest(workflow=workflow):
                self.workflow = workflow
                self.assertTrue(any("workflow" in issue for issue in self.issues()))

    def test_review_bypass_and_administrator_exemptions_are_rejected(self) -> None:
        self.protection["required_pull_request_reviews"]["bypass_pull_request_allowances"] = {"apps": [{"slug": "agent-app"}]}
        self.assertTrue(any("bypass" in issue for issue in self.issues()))
        del self.protection["required_pull_request_reviews"]["bypass_pull_request_allowances"]
        self.protection["enforce_admins"]["enabled"] = False
        self.assertTrue(any("enforce_admins" in issue for issue in self.issues()))


if __name__ == "__main__":
    unittest.main(verbosity=2)
