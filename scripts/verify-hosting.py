#!/usr/bin/env python3
"""Read-only verification of owner-authorized automated gates. Never changes GitHub.

ADR 0004 permits autonomous self-review and merge after required CI. Privileged
credentials are disclosed as a trust limitation, not misreported as an absent
branch rule or a requirement for the owner to recruit another reviewer.
"""
from __future__ import annotations
import argparse
import base64
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def problems(policy: dict, protection: dict, collaborators: list[dict], agent: str,
             codeowners: str | None = None, workflow: dict | None = None) -> list[str]:
    errors: list[str] = []
    checks = protection.get("required_status_checks") or {}
    if checks.get("strict") is not True:
        errors.append("Status checks must require an up-to-date branch")
    actual = set(checks.get("contexts", [])) | {entry.get("context") for entry in checks.get("checks", [])}
    expected_contexts = {entry["context"] for entry in policy["required_status_checks"]["checks"]}
    missing = expected_contexts - actual
    if missing:
        errors.append("Missing required checks: " + ", ".join(sorted(missing)))
    required_apps = {entry["context"]: entry["app_id"] for entry in policy["required_status_checks"].get("checks", [])}
    if not required_apps or len(required_apps) != len(policy["required_status_checks"]["checks"]):
        errors.append("Expected policy must bind every required context to an explicit GitHub App")
    actual_apps = {entry.get("context"): entry.get("app_id") for entry in checks.get("checks", [])}
    for context, app_id in required_apps.items():
        if not isinstance(app_id, int) or isinstance(app_id, bool) or app_id <= 0 or actual_apps.get(context) != app_id:
            errors.append(f"Required check {context!r} is not bound to the approved GitHub App")
    for key in ("enforce_admins", "required_conversation_resolution"):
        if (protection.get(key) or {}).get("enabled") is not True:
            errors.append(f"{key} must be enabled")
    for key in ("allow_force_pushes", "allow_deletions"):
        if (protection.get(key) or {}).get("enabled") is not False:
            errors.append(f"{key} must be explicitly disabled")

    # Keeping PR protection enabled is distinct from requiring a second human.
    reviews = protection.get("required_pull_request_reviews")
    expected_reviews = policy["required_pull_request_reviews"]
    if not isinstance(reviews, dict):
        errors.append("Pull-request protection must remain enabled even with zero required approvals")
    else:
        for key in ("dismiss_stale_reviews", "require_code_owner_reviews", "require_last_push_approval"):
            if reviews.get(key) is not expected_reviews[key]:
                errors.append(f"Review setting {key} differs from the owner-authorized policy")
        if reviews.get("required_approving_review_count") != expected_reviews["required_approving_review_count"]:
            errors.append("Required approving review count differs from the owner-authorized policy")
        if any((reviews.get("bypass_pull_request_allowances") or {}).get(kind) for kind in ("users", "teams", "apps")):
            errors.append("Pull-request bypass allowances must be empty")

    identity = next((person for person in collaborators if person.get("login", "").casefold() == agent.casefold()), None)
    if not identity or not (identity.get("permissions") or {}).get("push"):
        errors.append("The authorized execution identity must be verified with repository write access")
    if codeowners is None:
        errors.append("Protected-branch CODEOWNERS has not been verified")
    else:
        rows = [line.split("#", 1)[0].strip().split() for line in codeowners.splitlines() if line.split("#", 1)[0].strip()]
        if len(rows) != 1 or rows[0][0] != "*" or len(rows[0]) < 2:
            errors.append("Cannot verify ownership routing: expected one wildcard CODEOWNERS rule")
        else:
            writers = {person["login"].casefold() for person in collaborators if (person.get("permissions") or {}).get("push")}
            for owner in rows[0][1:]:
                if not re.fullmatch(r"@[A-Za-z0-9-]+", owner):
                    errors.append("CODEOWNERS teams or complex owners need explicit membership verification")
                elif owner[1:].casefold() not in writers:
                    errors.append(f"Code owner {owner} does not have verified repository write access")
    if not workflow or workflow.get("state") != "active" or workflow.get("path") != ".github/workflows/spec-drift.yml":
        errors.append("Trusted-base specification workflow is missing or inactive on the default branch")
    return errors


def trust_limitations(collaborators: list[dict], agent: str) -> list[str]:
    limits = ["GitHub Actions App binding authenticates the App, not an adversarially isolated workflow identity."]
    identity = next((person for person in collaborators if person.get("login", "").casefold() == agent.casefold()), {})
    permissions = identity.get("permissions") or {}
    if permissions.get("admin") or permissions.get("maintain") or identity.get("role_name") in ("admin", "maintain"):
        limits.append("The execution identity has administrative or maintenance authority and can change protections. This verifies current controls, not isolation from that administrator.")
    if identity.get("role_name") not in ("admin", "maintain", "write", "read", "triage"):
        limits.append("Custom-role and organization-level bypass capabilities are outside this branch-settings verification.")
    return limits


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
    if not re.fullmatch(r"[A-Za-z0-9-]+", args.agent_login):
        parser.error("invalid execution identity")
    try:
        policy = json.loads((ROOT / "docs/architecture/hosting-protection.json").read_text())
        protection = api(f"repos/{args.repo}/branches/main/protection")
        collaborators = api(f"repos/{args.repo}/collaborators?per_page=100")
        if not isinstance(collaborators, list) or len(collaborators) == 100:
            raise RuntimeError("Collaborator listing is invalid or needs pagination; verification is incomplete")
        owners = api(f"repos/{args.repo}/contents/.github/CODEOWNERS?ref=main")
        if owners.get("type") != "file" or owners.get("encoding") != "base64" or owners.get("size", 0) > 1024 * 1024:
            raise RuntimeError("Cannot verify protected-branch CODEOWNERS content")
        codeowners = base64.b64decode(owners["content"]).decode("utf-8")
        workflow = api(f"repos/{args.repo}/actions/workflows/spec-drift.yml")
        errors = problems(policy, protection, collaborators, args.agent_login, codeowners, workflow)
        print(json.dumps({"verified": not errors, "repository": args.repo, "agentIdentity": args.agent_login,
                          "approvalModel": "owner-authorized-autonomous", "independentHumanApprovalRequired": False,
                          "errors": errors, "trustLimitations": trust_limitations(collaborators, args.agent_login)}, indent=2))
        raise SystemExit(1 if errors else 0)
    except (RuntimeError, OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as error:
        print(json.dumps({"verified": False, "status": "incomplete", "error": str(error)}))
        raise SystemExit(2) from error


if __name__ == "__main__":
    main()
