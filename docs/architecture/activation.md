# Activating the external enforcement boundary

The specification, architecture guard, fixture tests and workflows are repository code. They become mandatory only after protected-branch hosting settings enforce them. Do not describe a feature branch containing these files as an already protected production repository.

## Human-controlled prerequisites

The agent must use an identity with branch/PR write access but no repository administration, maintenance, review-bypass or organization-rule privileges. That identity must not be a code owner. At least one distinct human code owner must have write access and approve the final pushed revision. Sharing an administrator's credentials between a human and an agent does not provide separation of authority.

At initial inspection, `alexcodeplace` was the sole collaborator and the available tool identity, with administrator permission. This is an activation blocker, not something an implementation agent can solve by approving its own PR or editing a checksum.

## Bootstrap sequence

1. A human reviews ADR 0001, the architecture specification, the proposed CI/ownership files and the completed acceptance/performance evidence.
2. Configure a separate human reviewer and non-admin, non-owner agent identity. Adjust CODEOWNERS through human review as needed.
3. Merge the reviewed bootstrap through the repository owner's controlled process. The trusted drift workflow must exist on the trusted base branch before it can enforce subsequent pull requests. Never run candidate code to simulate trusted-base enforcement during bootstrap.
4. Apply the settings described by `hosting-protection.json`: PRs and code-owner approval, stale-review dismissal, latest-push approval by someone other than its pusher, up-to-date required checks, conversation resolution, no force pushes/deletions, and restrictions applying to administrators. Bind required statuses to their expected GitHub App where supported, rather than accepting arbitrary writers of a matching status name.
5. Run `python3 scripts/verify-hosting.py --repo alexcodeplace/slopgate --agent-login <dedicated-agent-account>`. This is read-only and must not report success while required settings or independent identities are missing. GitHub App installations and organization-level bypass roles require an administrator's additional review.
6. Open a disposable PR that changes a protected policy without an ADR and confirm that the trusted status blocks it. Add a complete ADR and confirm that this does not bypass the separate human approval requirement. Verify that changing the candidate drift script or workflow cannot replace the trusted-base check. Close the test PR without merging policy weakening.

## What the checks prove

The syntax-aware guard checks declared crate dependencies, concrete-checker isolation, resource ownership and requirement-to-evidence references. Unit and acceptance tests check behavior, including representative attempts to violate those boundaries. The trusted-base drift check reads candidate Git blobs as data and requires a new decision record for protected changes. It does not grant approval.

The system is not a mathematical proof of arbitrary program correctness, an adapter sandbox, or a defense against an administrator who can remove hosting rules. Runtime adapter code and dependency build scripts must run only on disposable CI workers without deployment secrets. A required unavailable checker is an incomplete result, not clean code.

## Documentation sources

- GitHub protected branches: https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches
- GitHub workflow trigger security: https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#pull_request_target
- Hosted runner architectures: https://github.com/actions/runner-images
