#!/usr/bin/env python3
"""Metadata-only drift review. Run the trusted BASE revision of this script.

Candidate files are read as Git blobs and never imported, checked out or executed.
Passing means an ADR accompanies a protected change, NOT that an agent approved it.
Independent code-owner approval remains a protected-branch hosting requirement.
"""
from __future__ import annotations
import argparse
import json
import re
import subprocess
from pathlib import Path

PROTECTED = re.compile(r"^(?:\.github/|AGENTS\.md$|(?:.*/)?Cargo\.(?:toml|lock)$|docs/(?:specs|architecture)/|tools/architecture-guard/|rules/|\.slopgate/|scripts/(?:spec_drift|test_spec_drift|verify-hosting|benchmark)\.py$|tests/acceptance/)")
ADR = re.compile(r"^docs/adr/\d{4}-[a-z0-9-]+\.md$")
HEADINGS = ("Decision", "Compatibility", "Performance", "Verification", "Human approval")


def git(repository: Path, *arguments: str) -> bytes:
    result = subprocess.run(["git", "-c", "core.fsmonitor=false", *arguments], cwd=repository, capture_output=True, timeout=20, check=False)
    if result.returncode:
        raise ValueError("Git metadata operation failed: " + result.stderr[:1000].decode("utf-8", errors="replace"))
    if len(result.stdout) > 4 * 1024 * 1024:
        raise ValueError("Git metadata exceeds the 4 MiB review bound")
    return result.stdout


def paths(repository: Path, base: str, head: str, *, added: bool = False) -> list[str]:
    flags = ["--diff-filter=A"] if added else []
    data = git(repository, "diff", "--name-only", "--no-renames", "--no-ext-diff", "-z", *flags, base, head, "--")
    if data and not data.endswith(b"\0"):
        raise ValueError("Git path stream is not NUL terminated")
    return [value.decode("utf-8") for value in data.split(b"\0") if value]


def blob(repository: Path, revision: str, path: str) -> str:
    identifier = f"{revision}:{path}"
    size = int(git(repository, "cat-file", "-s", identifier).strip())
    if size > 1024 * 1024:
        raise ValueError("ADR exceeds the 1 MiB review bound")
    return git(repository, "cat-file", "blob", identifier).decode("utf-8")


def valid_adr(contents: str) -> bool:
    for heading in HEADINGS:
        match = re.search(r"(?ms)^## " + re.escape(heading) + r"\s*\n(.+?)(?=^## |\Z)", contents)
        if not match or len(match.group(1).strip()) < 40:
            return False
    return True


def review(repository: Path, base: str, head: str) -> dict:
    if not all(re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", ref) for ref in (base, head)):
        raise ValueError("base and head must be complete hexadecimal commit IDs")
    for revision in (base, head):
        if git(repository, "cat-file", "-t", revision).strip() != b"commit":
            raise ValueError("review revisions must be commit objects")
    changed = sorted(path for path in paths(repository, base, head) if PROTECTED.search(path))
    decisions = [path for path in paths(repository, base, head, added=True) if ADR.fullmatch(path)]
    valid = [path for path in decisions if valid_adr(blob(repository, head, path))]
    passed = not changed or bool(valid)
    return {"schemaVersion": 1, "status": "passed" if passed else "blocked", "base": base, "head": head, "protectedChanges": changed, "decisionRecords": valid, "humanApprovalStillRequired": bool(changed), "reason": "Protected changes require a new ADR with decision, compatibility, performance, verification and independent human approval sections." if not passed else "Metadata reviewed; this result does not grant human approval."}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository", type=Path, default=Path("."))
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    args = parser.parse_args()
    try:
        result = review(args.repository.resolve(), args.base, args.head)
        print(json.dumps(result, indent=2))
        raise SystemExit(0 if result["status"] == "passed" else 1)
    except (ValueError, OSError, UnicodeError, subprocess.TimeoutExpired) as error:
        print(json.dumps({"schemaVersion": 1, "status": "incomplete", "error": str(error)}))
        raise SystemExit(2) from error


if __name__ == "__main__":
    main()
