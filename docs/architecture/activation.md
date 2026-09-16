# Activating the owner-authorized automated gate

The owner explicitly asked the assistant to review and merge this work without requiring another human to read code. ADR 0004 records that authorization and supersedes the earlier assistant-added independent-review prerequisite. Core architecture, feature-correctness tests, performance budgets and fail-closed behavior are unchanged.

## Required controls

Pull requests remain mandatory. Required quality, architecture, Linux/macOS/Windows, performance, workflow-lint and trusted-base drift checks must all pass on the applicable revision. Checks are bound to the GitHub Actions App (15368); the branch must be up to date and conversations resolved. Force pushes and deletion are disabled, and branch rules apply to administrators. No administrative merge bypass is permitted.

The approving-review count is intentionally zero, with no mandatory code-owner or latest-push approval. The owner-authorized assistant performs a documented substantive review and may merge through the normal PR endpoint after checking the exact head SHA. CODEOWNERS remains review routing/audit metadata. This is not represented as independent human approval.

## Bootstrap and verification

1. Record the owner's authorization, review the actual implementation and evidence, and resolve review findings. Run all CI jobs at the final head.
2. Activate the seven ordinary App-bound CI requirements before merging the bootstrap. Do not require a trusted-base status that cannot yet run because its workflow has not reached main.
3. Merge the reviewed PR using the exact verified head SHA, without an admin bypass. Verify main's CI and files.
4. Immediately activate the complete policy in `hosting-protection.json`, adding `slopgate/trusted-spec-drift` after the base workflow exists. Verify the settings through the read-only checker.
5. Open a disposable PR with a protected change and no ADR. Confirm the trusted-base status fails and GitHub reports the PR blocked. Candidate review code must never be executed with elevated permissions. Add a complete ADR and restore candidate review code, confirm the trusted status and normal checks pass, then close the probe without merging the synthetic policy change.
6. Run `python3 scripts/verify-hosting.py --repo alexcodeplace/slopgate --agent-login alexcodeplace` and record the settings, test-PR result, merge SHA and final CI evidence.

## Honest limits

These controls prevent normal workflow drift and accidental ungated merges; they are not a sandbox or proof of arbitrary correctness. An administrator capable of rewriting repository protections can change them. GitHub Actions App binding authenticates the App, not an isolated workflow principal. The verifier reports privileged/shared credentials as a limitation, not as a fictional unresolvable code blocker. It must not call missing protections or skipped checks a success.

Untrusted adapters and dependency build scripts run on disposable CI workers without deployment secrets. Least-privileged credentials are useful defense in depth but are not a prerequisite the owner has accepted for this delivery. A new independent reviewer must not be demanded as substitute for the authorized assistant's own review.

## Sources

GitHub branch-protection API: https://docs.github.com/en/rest/branches/branch-protection
GitHub pull-request merge API: https://docs.github.com/en/rest/pulls/pulls
Trusted trigger behavior: https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#pull_request_target
