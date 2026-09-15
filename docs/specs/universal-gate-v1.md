# Universal Slopgate architecture, version 1

Status: authoritative implementation contract. Changes require an architecture decision and human code-owner approval. Implementation agents may report gaps but MUST NOT weaken this specification, acceptance tests, or enforcement policy to obtain a passing result.

## Purpose and boundary

Slopgate is a deterministic, language-neutral policy gate. A Rust coordinator schedules shared scanning engines and replaceable specialized adapters, combines normalized findings, applies reviewed project policy, and returns one trustworthy result. Universal means adding a language or framework does not require changing the coordinator. It does not claim one parser, syntax rule, or type system can analyze every language, or that static analysis can prove all behavioral properties or stop repository administrators.

This is the complete v1 delivery contract. Adapter coverage is explicit, not inferred from discovering an extension. Expensive external semantic checking cannot be promised to finish instantly. Performance applies to measured native work and safe reuse, never skipping required checks.

## Architecture and dependency direction

SG-ARCH-001: Separate `slopgate-core` (neutral coordination, contracts, configuration, discovery, shared regex/AST scanning, process execution, baselines, suppression, reporting), `slopgate-adapters` (concrete checkers/parsers, stack discovery and language-aware integration helpers), and `slopgate-rs` (CLI/composition root). Adapters MAY depend on core; core MUST NOT depend on adapters. The executable composes the registry and passes it to the coordinator. Core MUST NOT select behavior by concrete checker name or hard-coded programming-language extension. A neutral contract module owns checker request/result/capability types. Do not embed JavaScript or a compiler per language in core.

SG-ARCH-002: Shared scanners use the same explicitly enumerated files in file, staged, and full modes. Structural scanning delegates parsing to ast-grep. Configuration MUST NOT present unsupported parser coverage as checked. Extension policy comes from project configuration, not hook/coordinator whitelists. Test files are not globally excluded; rule-owned inclusion/exclusion and explicit project exclusions determine coverage.

SG-ARCH-003: Built-in checkers retain existing identifiers and findings except documented, tested corrections. Adapter capabilities declare ID and execution scope. Semantic/uncertain cross-file work defaults to full configured project scope; do not add changed-file-only type checking. Retain TypeScript incremental/multi-project support; reference/build mode belongs in its adapter. JSON regex and ast-grep YAML remain project-owned data. Protocol adapters provide extensibility without rebuilding core.

## Adapter protocol

SG-PROTO-001: External adapters use a versioned JSON stdin/stdout protocol, one bounded process per batch. Configure an executable and argument array, never an implicit shell string. Requests contain protocolVersion=1, adapterId, repository root, selected files or explicit full-project scope, mode/tier and settings. Responses contain protocolVersion=1, explicit complete/error status, normalized findings and errors. Complete responses cannot contain errors. Empty/truncated/malformed/unsupported-version output, invalid findings, contradictory exit/status, signals, overflow and timeout are infrastructure errors, never zero findings. Document the protocol and test real subprocesses. Validate nonempty IDs, recognized severity, repository-relative paths without traversal, and positive locations. Do not trust adapter-provided engine identity.

SG-PROTO-002: Unknown checker IDs and unknown top-level config keys fail validation. Required checks default to required; optional behavior is explicit and visible. No automatic tool downloads/installations, implicit shell evaluation, or network-dependent LLM decisions during scans. Any language may implement an executable adapter. Protocol execution is not a sandbox: untrusted project code runs only on disposable least-privileged CI workers.

## Correctness and reliability

SG-GATE-001: Outcomes are complete/pass (exit 0), complete/blocking policy findings (exit 1), and incomplete/configuration/infrastructure failure (exit 2). Required checks that cannot complete prevent passing. Infrastructure failures cannot be baselined/suppressed as code findings. Incomplete scans MUST NOT create/update/prune baselines. Warnings (including explicitly allowed unpinned-tool notices) are distinct from infrastructure errors. Optional skips are reported.

SG-GATE-002: Detection, execution, parsing, regex evaluation and enumeration errors propagate. Explicit nonexistent AST rule directories are invalid. Empty scope is visible and distinct from discovery failure. Retain parser/CLI regression tests; document changes correcting unsafe fail-open behavior.

SG-PROC-001: Analysis subprocesses and probes use one bounded runner. Enforce wall-clock deadlines, bounded stdout/stderr, null stdin except protocol input, exit status and cleanup. Drain output without deadlocks. A parent exit with descendants holding pipes cannot hang the gate. Native CI tests Unix process groups and Windows process-tree handling. No ambient shell for ordinary commands. Unsupported strong cleanup guarantees fail explicitly rather than silently claiming enforcement.

SG-SCOPE-001: Staged scans MUST NOT validate a different working tree while claiming commit correctness. V1 rejects inconsistent index/working-tree state (partial staging, removed files and untracked analysis inputs) before semantic execution; never stash/reset the user's tree. Index-overlay snapshots are outside v1. Config-only, lockfile-only and deletion-only staged changes still trigger applicable project checks. Git path lists are NUL-delimited. Git failures are errors, not empty selections. Test linked worktrees.

SG-RULE-001: Support/document project JSON regex packs. Keep text, structural and semantic rules separate. Regex execution errors cannot become non-matches. Prefer the linear engine for compatible expressions, with bounded fallback for compatibility. No rule-ID-specific matching implementations. V1 regex remains line-scoped with explicit input/backtracking bounds. Project rule overrides must be visible and reviewed, not silently launder a mandatory rule.

SG-TEST-001: Positive/negative fixtures assert exact expected IDs and locations/counts, not some rule firing somewhere. Exercise real CLI file/full/staged entry points and packaged launcher selection. Include a non-JavaScript protocol adapter. Cover missing tooling, malformed output, invalid config, timeouts, output floods, inconsistent staging, deletion/config-only changes, Unicode paths and deterministic output. Multi-language shared-scanner fixtures must not require coordinator branches.

## Performance

SG-PERF-001: Bound concurrency globally; nested scheduling cannot multiply the limit. Batch work per adapter/project. Output is deterministic regardless of completion order. No daemon is required. Preserve prebuilt binaries. Native fast scans never invoke heavy checkers.

SG-PERF-002: Reuse results only with complete input identity. Retain tool-owned incremental state. New Slopgate result caches must include source/config/rule content, adapter/protocol/tool versions and executable identity, scope, dependencies and relevant environment. Corrupt/untrusted cache entries are misses; incomplete results are never cached. Disable external caching for incomplete dependency declarations and fall back to full-project checks. V1 needs neither remote cache nor daemon; do not add unsafe semantic caching for apparent speed.

SG-PERF-003: Commit reproducible release-binary benchmarks/CI budgets with machine/toolchain/workload metadata. Report wall-clock p50/p95 separately for native file/repository scans and external checker work. Include long-line regex regressions. Reference target: sub-100 ms median native small-file scan; CI has documented hardware-noise headroom with hard upper bounds. Measure throughput and regression ratios, not universal guarantees. Budget failures never disable rules. Record actual baselines/limitations.

## Enforcement and governance

SG-GOV-001: CI checks dependency direction, checker isolation, shared analysis-process runner, requirement/test traceability, formatting, strict lint, workspace tests, packaged-CLI acceptance and release performance. Architecture checks inspect manifests/modules or syntax, not just AGENTS.md. Mutation tests demonstrate rejection of forbidden architecture and drift.

SG-GOV-002: Protect spec, architecture policy/guard, workflows, manifests/lockfile, baseline/suppression policy, rules and CODEOWNERS. CODEOWNERS alone is not enforcement. Hosting must require PRs, named passing checks, code-owner review, stale-review dismissal, and approval of latest push by someone other than its pusher; forbid force pushes/deletion and enforce for admins. Agent credentials must not approve themselves, alter hosting protections or bypass checks. Trusted-base metadata-only drift checking must never execute candidate code or load candidate policy with privileged permissions.

SG-GOV-003: Spec changes require an ADR with compatibility/performance impact, acceptance evidence and independent human approval, not a self-approved checksum. Administrators controlling code/checks/protections can bypass repository safeguards; state this honestly. Report feature-branch implementation separately from hosted protection bootstrap/activation. Do not claim hosted enforcement active before verification.

## Evidence

SG-EVIDENCE-001: Map requirements to implementation/tests, record build/commit provenance, commands/results, benchmarks, review findings/resolutions and external activation steps. Review correctness, architecture drift, trust boundaries and performance. No unrun test is passing; mocked checker tests are not real compiler runs. Preserve unrelated working-tree changes.

## Design sources

- Rust processes: https://doc.rust-lang.org/std/process/struct.Child.html and https://doc.rust-lang.org/std/process/struct.Command.html
- TypeScript references: https://www.typescriptlang.org/docs/handbook/project-references.html
- GitHub protection: https://docs.github.com/repositories/configuring-branches-and-merges-in-your-repository/defining-the-mergeability-of-pull-requests/about-protected-branches
- Code owners: https://docs.github.com/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners
