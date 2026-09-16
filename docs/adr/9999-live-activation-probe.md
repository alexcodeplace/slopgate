# ADR 9999: Disposable live activation probe

## Decision

Exercise the deployed trusted-base metadata review without merging this synthetic specification comment. Candidate review code is restored exactly to the protected base before this positive case.

## Compatibility

There is no runtime or product behavior change. This branch is a temporary verification fixture and must be closed without merging. No actual checker or source policy is removed.

## Performance

All normal CI and performance requirements remain enabled. The temporary decision record is metadata only and cannot exempt a failed checker or alter an execution budget.

## Verification

The prior head was rejected by the trusted-base workflow with protectedChanges identifying the specification and candidate review script, and GitHub reported the PR blocked. This case restores candidate code and adds the required decision metadata; its trusted status and normal CI must pass.

## Review authorization

The owner authorized end-to-end implementation review and CI enforcement verification. This record is a disposable positive test of that metadata contract, not approval to merge a probe or to bypass production checks.
