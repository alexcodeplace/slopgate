#!/usr/bin/env python3
"""Read-only verification of the external trust boundary. Never changes GitHub."""
from __future__ import annotations
import argparse
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def problems(policy: dict, protection: dict, collaborators: list[dict], agent: str) -> list[str]:
    errors = []
    checks = protection.get("required_status_checks") or {}
    if checks.get("strict") is not True:
        errors.append("Status checks must require an up-to-date branch")
    actual = set(checks.get("contexts", [])) | {entry.get("context") for entry in checks.get("checks", [])}
    missing = set(policy["required_status_checks"]["contexts"]) - actual
    if missing:
        errors.append("Missing required checks: " + ", ".join(sorted(missing)))
    for key in ("enforce_admins", "required_conversation_resolution"):
        if protection.get(key, {}).get("enabled") is not True:
            errors.append(f"{key} must be enabled")
    for key in ("allow_force_pushes", "allow_deletions"):
        if protection.get(key, {}).get("enabled") is not False:
            errors.append(f"{key} must be explicitly disabled")
    reviews = protection.get("required_pull_request_reviews") or {}
    for key in ("dismiss_stale_reviews", "require_code_owner_reviews", "require_last_push_approval"):
        if reviews.get(key) is not True:
            errors.append(f"Review setting {key} must be enabled")
    if reviews.get("required_approving_review_count", 0) < 1:
        errors.append("At least one independent review is required")
    if any(reviews.get("bypass_pull_request_allowances", {}).get(kind) for kind in ("users", "teams", "apps")):
        errors.append("Pull-request review bypass allowances must be empty")
    identity = next((person for person in collaborators if person.get("login") == agent), None)
    if not identity:
        errors.append("Agent identity could not be verified as a collaborator; verify a GitHub App's installation permissions separately")
    elif identity.get("permissions", {}).get("admin") or identity.get("role_name") in ("admin", "maintain"):
        errors.append("Agent must not have repository administration or maintenance permissions")
    reviewers = [person for person in collaborators if person.get("login") != agent and person.get("type") != "Bot" and person.get("permissions", {}).get("push")]
    if not reviewers:
        errors.append("No distinct human reviewer with write access is available")
    return errors


def api(path: str):
    result = subprocess.run(["gh", "api", path], text=True, capture_output=True, timeout=20, check=False)
    if result.returncode:
        raise RuntimeError(f"Cannot verify GitHub state at {path}: {result.stderr.strip()[:500]}")
    return json.loads(result.stdout)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default="alexcodeplace/slopgate")
    parser.add_argument("--agent-login", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repo):
        parser.error("invalid repository identifier")
    try:
        policy = json.loads((ROOT / "docs/architecture/hosting-protection.json").read_text())
        protection = api(f"repos/{args.repo}/branches/main/protection")
        collaborators = api(f"repos/{args.repo}/collaborators?per_page=100")
        if len(collaborators) == 100:
            raise RuntimeError("Collaborator listing needs pagination; verification is incomplete")
        errors = problems(policy, protection, collaborators, args.agent_login)
        print(json.dumps({"verified": not errors, "repository": args.repo, "agentIdentity": args.agent_login, "errors": errors, "note": "This checks configured controls, not the identity of a person operating shared credentials. GitHub App and organization-level bypass permissions require independent administrator review."}, indent=2))
        raise SystemExit(1 if errors else 0)
    except (RuntimeError, OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(json.dumps({"verified": False, "status": "incomplete", "error": str(error)}))
        raise SystemExit(2) from error


if __name__ == "__main__":
    main()
