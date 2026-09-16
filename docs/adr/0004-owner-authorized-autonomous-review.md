# ADR 0004: Owner-authorized autonomous review and merge

Status: authorized by the repository owner's explicit instruction in this conversation on 2026-09-16. Supersedes the independent-human-review activation prerequisite proposed in ADRs 0001 through 0003. Implementation correctness and architecture constraints are not relaxed.

## Decision

The owner explicitly instructed the assistant to review the implementation itself and merge it when ready, explaining that they do not read code and have no separate reviewer to provide. The earlier requirement for a distinct human approver and separate credentials was introduced by the assistant, not requested by the owner. Remove that prerequisite and finish the delegated work autonomously.

Preserve required pull requests, App-bound up-to-date CI, architecture and drift checks, resolved conversations, no force pushes/deletions and administrator enforcement. Zero required approving reviews, no mandatory code-owner approval and no latest-pusher approval are intentional. Self-review must be recorded as self-review, never as an independent human endorsement. Merge only the exact reviewed head after passing checks, using the normal merge endpoint without administrative bypass.

## Compatibility

This changes governance requirements SG-GOV-002 and SG-GOV-003 and their documentation/verifier tests. It does not change runtime code, adapter protocols, failure policy, baselines, scanner behavior or native performance targets. Historical ADRs/evidence remain as records of the earlier proposal and are explicitly superseded on this single point. Protected changes still require decision records, review and CI. The user has authorized the review/merge and the previously requested CI enforcement; this is not permission for future agents to weaken checks whenever implementation fails.

## Performance

No scan or compiler execution changes. The same release-performance jobs, native targets and measured reports remain required. Removing an unnecessary second-human approval does not trade runtime correctness for speed.

## Verification

Re-run governance mutations and the full PR CI suite. Review fail-closed execution, staged consistency, architecture dependency direction, packaged tooling, regression coverage and performance evidence. Activate ordinary requirements before bootstrap merge, then add the trusted status once the workflow exists on main. Prove live trusted-base negative/positive behavior using a disposable PR, never merge the probe, and verify main's final settings and CI. Record exact SHAs and distinguish source review, tests, hosted activation and administrative trust limits.

## Review authorization

The owner's latest instruction is the authorization for assistant review and merge when the implementation is good to go. No separate human code review or identity setup is required. The assistant remains responsible for checking actual repository state and passing CI. Administrator credentials can change protections and must not be represented as an adversarially isolated trust boundary; that limitation is disclosed rather than manufactured into a blocker.
