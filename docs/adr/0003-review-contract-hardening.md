# ADR 0003: Close verified configuration, coverage and governance gaps

Governance update: ADR 0004 supersedes this record's independent-human-review prerequisite. Its technical decisions and historical evidence remain applicable.

Status: implementation review corrections, pending independent human approval. The universal-gate specification is unchanged.

## Decision

Keep the coordinator language-neutral and strengthen the implementation where final review exposed gaps. An omitted gate tier retains its normal default instead of becoming an empty severity set. Invalid UX settings fail configuration validation. Shared engine identifiers are reserved so an external adapter cannot make coverage reporting ambiguous. Absolute editor paths are compared by filesystem identity when their representation differs from the configured canonical root.

Restore the existing commit-tier checker-health history as best-effort telemetry derived from explicit outcomes. It neither decides the gate result nor uses diagnostic-string guessing. Required failures remain incomplete and blocking. Bound health-cache reads and preserve atomic writes.

Strengthen architecture mutations against whole-standard-library aliases and conditional or qualified source inclusion, and resolve inherited workspace dependency paths according to Cargo's rules. Extend trusted drift coverage to embedded rule packs, launchers, hooks, provisioning/verification scripts and packaging configuration. The hosting verifier must require App-bound status sources, a verified wildcard ownership rule with independent human owners, and an active trusted-base workflow; merely finding a collaborator and matching status names is insufficient.

## Compatibility

Explicit empty gate severity lists remain intentional configuration. Only omitted tiers receive defaults. Invalid UX data, reserved adapter IDs, ambiguous ownership, and incomplete source identity are rejected instead of silently weakening policy. Normal relative paths remain unchanged. The guard permits generic code and real test fixtures while rejecting newly covered boundary violations. No rule or infrastructure failure is waived to obtain a passing result.

The hosting verifier supports the proposed single wildcard code-owner policy. Team membership, custom roles, complex ownership patterns and organization-level bypass powers require independent review rather than an unsubstantiated success result. All hosting operations remain read-only in this task.

## Performance

Keep fast paths free of heavyweight semantic checks. Health history is written only on staged commit-tier runs, not file feedback or immutable CI scans. Add a real native AST workload to the release benchmark, using the same sub-100 ms median and 300 ms p95 CI budget rather than relaxing the existing native targets. Record optional prior-report ratios with explicit machine comparability and load/cold-cache limitations. Full-project TypeScript cost remains measured separately.

## Verification

Add end-to-end regressions for absolute editor inputs, partial gate configuration, invalid UX settings, reserved coverage identities and historical checker-health behavior. Add offline hosting mutations for wrong/unbound status sources, privileged agent roles, code-owner self-approval, missing human access, disabled trusted workflows and review bypass. Retain all compiler, process-lifecycle, package, fixture, architecture and cross-platform tests. Rerun release benchmarks and all hosted CI jobs on the final revision before updating delivery evidence.

## Human approval

An independent human must approve these implementation corrections, policy coverage changes, test expectations and performance evidence. A new ADR does not grant its implementation author approval or permission to modify hosting protections. Dedicated non-admin agent credentials, human review and trusted-base bootstrap remain separate activation prerequisites.
