# Universal Slopgate: merged and enforced

## Completed delivery

The owner authorized the assistant to review and merge without requiring a second human reviewer. That authorization is recorded in ADR 0004. PR #22 was substantively self-reviewed, all required CI passed at exact head `bfdda1680fd8dadcb0717a1b19ffdbf2bbb8e5be`, and the PR was merged normally into main as `e798201fabc0ed4c5659150de64045dc11ec9528`. No administrative bypass or fabricated independent approval was used.

- PR and review: https://github.com/alexcodeplace/slopgate/pull/22 and review comment 5222880095.
- Final implementation PR CI: https://github.com/alexcodeplace/slopgate/actions/runs/35098032005, successful.
- Merged-main CI: https://github.com/alexcodeplace/slopgate/actions/runs/35098588162, successful.

Both CI runs include quality, architecture, Linux, macOS, Windows, performance and workflow-lint jobs. The platform jobs run real compiler/executable acceptance, native release builds and actual npm-tarball consumer tests. Dedicated Linux verification also passed 321 Rust tests, 28 executable/compiler acceptance tests, 6 drift mutations and 12 hosting-verifier mutations. The final review additionally repaired a diagnostic-source FIFO hang path and the GitHub protection API's mutually exclusive status selector format.

## Active hosting controls

Main requires pull requests, up-to-date status checks bound to the GitHub Actions App (15368), resolved conversations, no force pushes/deletions, and enforcement for administrators. Eight required contexts are active: `Slopgate quality`, `Slopgate architecture`, `Slopgate Linux`, `Slopgate macOS`, `Slopgate Windows`, `Slopgate performance`, `Slopgate workflow lint`, and `slopgate/trusted-spec-drift`.

The approving-review count is intentionally zero. Code-owner/latest-push approvals are not mandatory under the owner's explicitly delegated self-review model. This does not waive CI. The branch-protection verifier returned `verified=true`, with no errors, after checking the actual hosted settings, ownership routing and active base workflow.

## Live negative and positive verification

Disposable PR #23 proved the deployed behavior, not just unit-test expectations. It was never merged.

At negative head `0caae1cae0db941f4982c68ceda887e57dfa1462`, a protected spec change had no ADR and the candidate copy of `scripts/spec_drift.py` exited successfully. The trusted-base workflow still rejected the change, listed both protected files, returned `status=blocked`, and published a failing required status. GitHub reported `mergeable_state=blocked`. This proves candidate review code did not decide the trusted status.

After restoring candidate review code and adding complete fixture ADR metadata, positive head `5bd70e725535205b37e1e07d564942f241d8319c` passed the trusted workflow and all normal CI jobs. GitHub reported `mergeable_state=clean`. The probe PR was closed without merging and its temporary branch was deleted.

Negative trusted run: https://github.com/alexcodeplace/slopgate/actions/runs/35098741728.
Positive trusted run: https://github.com/alexcodeplace/slopgate/actions/runs/35098940257.
Positive full CI: https://github.com/alexcodeplace/slopgate/actions/runs/35098943039.

## Release measurements on merged main

The following are actual merged-main CI fixture measurements, not arbitrary-project speed promises. They launch the real release executable without a daemon or Slopgate semantic-result cache. The repository workload is 1,001 files and 32,032 lines; the semantic row is a small actual TypeScript fixture.

| Workload | Median | p95 |
| --- | ---: | ---: |
| Native file, text policies | 5.087 ms | 5.200 ms |
| Native file including AST | 16.026 ms | 16.326 ms |
| 1,001-file text-policy repository | 17.146 ms | 18.375 ms |
| Small incremental TypeScript project | 333.051 ms | 352.778 ms |

All configured budgets passed. Source-input digest: `5573e4bf25a0b497b3f9a685888ef0116a53d876bac18ee5648fc05b7f417cd2`. Full machine/toolchain, binary identity, sample counts and bounds are in `evidence/performance-main-e798201.json`. Large project type checking remains more expensive than native single-file policy checks.

## Durable evidence and limits

`evidence/live-gate-activation-proof.json`, `evidence/hosting-activated-e798201.json` and `evidence/branch-protection-activated-e798201.json` record exact revisions, runs, observed settings and the live probe result. Previous historical reports remain as dated implementation checkpoints; their assistant-added requirement for another human is superseded by ADR 0004 and is not an outstanding blocker.

This is an enforced automated engineering workflow, not an adapter sandbox, a mathematical proof of arbitrary program correctness, or isolation from an administrator authorized to change protections. GitHub Actions App binding authenticates the App rather than an independent workflow principal. These limits are disclosed; no unrelated credential setup or human recruitment is required to use the completed delivery.

The original checkout's five unrelated modified files were preserved. No npm release was published and no existing user worktree was reset or stashed. The implementation is merged; release publication is a separate action.
