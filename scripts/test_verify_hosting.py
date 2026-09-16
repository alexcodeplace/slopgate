#!/usr/bin/env python3
"""Offline hosting mutations for ADR 0004. No network calls or permission writes."""
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
        self.people = [{"login": "authorized-owner", "type": "User", "role_name": "admin",
                        "permissions": {"push": True, "admin": True, "maintain": True}}]
        self.owners = "# ownership routing\n* @authorized-owner\n"
        self.workflow = {"path": ".github/workflows/spec-drift.yml", "state": "active"}

    def issues(self) -> list[str]:
        return VERIFIER.problems(self.policy, self.protection, self.people, "authorized-owner", self.owners, self.workflow)

    def test_owner_authorized_single_actor_policy_passes_with_trust_limit_disclosed(self) -> None:
        self.assertEqual(self.issues(), [])
        limits = VERIFIER.trust_limitations(self.people, "authorized-owner")
        self.assertTrue(any("administrative" in limitation for limitation in limits))

    def test_context_names_alone_do_not_prove_trusted_status_source(self) -> None:
        del self.protection["required_status_checks"]["checks"]
        self.assertTrue(any("GitHub App" in issue for issue in self.issues()))

    def test_wrong_unbound_or_any_app_source_is_rejected(self) -> None:
        for app_id in (None, -1, 0, 123456):
            with self.subTest(app_id=app_id):
                self.protection["required_status_checks"]["checks"][0]["app_id"] = app_id
                self.assertTrue(any("GitHub App" in issue for issue in self.issues()))

    def test_zero_approvals_does_not_allow_disabling_pull_request_protection(self) -> None:
        self.protection["required_pull_request_reviews"] = None
        self.assertTrue(any("Pull-request protection" in issue for issue in self.issues()))

    def test_owner_policy_does_not_silently_reintroduce_an_external_reviewer(self) -> None:
        original = copy.deepcopy(self.protection)
        for key in ("require_code_owner_reviews", "require_last_push_approval"):
            with self.subTest(key=key):
                self.protection = copy.deepcopy(original)
                self.protection["required_pull_request_reviews"][key] = True
                self.assertTrue(any(key in issue for issue in self.issues()))
        self.protection = copy.deepcopy(original)
        self.protection["required_pull_request_reviews"]["required_approving_review_count"] = 1
        self.assertTrue(any("review count" in issue for issue in self.issues()))

    def test_required_checks_and_up_to_date_branch_are_not_optional(self) -> None:
        self.protection["required_status_checks"]["strict"] = False
        self.assertTrue(any("up-to-date" in issue for issue in self.issues()))
        self.protection["required_status_checks"]["strict"] = True
        self.protection["required_status_checks"]["checks"] = []
        self.protection["required_status_checks"]["contexts"] = []
        self.assertTrue(any("Missing required checks" in issue for issue in self.issues()))

    def test_ownership_routing_must_be_real_but_can_include_execution_identity(self) -> None:
        self.owners = "* @AUTHORIZED-OWNER"
        self.assertEqual(self.issues(), [])
        for owners in ("* @unknown", "* @organization/team", "src/** @authorized-owner", "* @authorized-owner\n.github/**", "", None):
            with self.subTest(owners=owners):
                self.owners = owners
                self.assertTrue(self.issues())

    def test_unverified_or_read_only_execution_identity_cannot_pass(self) -> None:
        self.people[0]["permissions"]["push"] = False
        self.assertTrue(self.issues())
        self.people = []
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

    def test_no_force_push_or_deletion_even_for_zero_review_policy(self) -> None:
        for key in ("allow_force_pushes", "allow_deletions"):
            with self.subTest(key=key):
                self.protection[key]["enabled"] = True
                self.assertTrue(any(key in issue for issue in self.issues()))
                self.protection[key]["enabled"] = False


if __name__ == "__main__":
    unittest.main(verbosity=2)
