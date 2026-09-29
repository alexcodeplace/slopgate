# ADR 0007: Measured regex performance without policy changes

Status: implementation decision for the owner's September 29, 2026 request to optimize Slopgate in a separate worktree, compare benchmarks, and avoid regressions. This does not change the universal-gate-v1 specification or its protected policy.

## Decision

Keep Rust and the existing release profile. Use the existing conservative necessary-literal proof to avoid repeated impossible regex matches. Case-sensitive literal checks use exact substring searches; Unicode-folded checks retain the original regex engine. For files with at least eight split lines, a missing mandatory literal can reject that rule's file candidate. Linear-engine auxiliary proofs are initialized lazily, and only when current line count times selected file count reaches 4,096 estimated line visits; this avoids duplicate proof/Unicode-regex setup on small scans. This heuristic affects optimization effort, not what is checked. All accepted candidates still use the original line-scoped matcher. Classify test-file names once per file rather than once per rule.

Add `scripts/compare_performance.py` and `scripts/test_compare_performance.py` as developer-only measurement tools. They are not scan-time dependencies, do not download tools, and do not replace the existing benchmark.py, budgets, acceptance or CI requirements. The harness runs preserved release binaries in alternating A/B order against identical inputs, recording raw samples, hashes and behavioral comparisons.

## Compatibility

No rule, configuration, API, adapter or protocol changes. Preserve source reads, UTF-8 validation, source and line bounds, diagnostic and excerpt limits, backtracking failures, rule-owned file scoping, minFiles thresholds, deterministic findings and staged consistency. No unsafe code, new dependency, daemon or result cache. A failed or unsupported literal proof simply provides no prefilter.

Benchmark comparisons ignore only coverage elapsed times and the random six-character AST scratch-directory nonce in the AST project-summary metadata. Findings, severity, order, locations, rule counts, coverage status, errors, selected files, exit codes and arbitrary diagnostic text remain checked. Unit tests protect that normalization boundary. TypeScript measurements use its existing incremental state, warmed for both variants.

## Performance

Compare independently built release binaries with the same toolchain and release settings on one assigned worker. Report medians, p95, raw samples and paired bootstrap intervals. Include small inputs, dense matches, unfilterable patterns, native source corpora and external checks. Shared worker load and CPU frequency are not controlled; these are not physical cold-cache tests or universal speed guarantees.

Three incremental trials isolate exact-literal checks, compatibility file prefilters and linear-engine file prefilters. On the initial 31-pair combined trial, the large synthetic repository improved from 49.466 to 33.382 ms and the Slopgate source corpus from 23.966 to 20.069 ms. Small and dense-positive cases changed little. A full-pack follow-up exposed about 5% extra single-file startup cost from eager linear proofs; the final selection therefore requires the lazy-proof trial to remove that penalty while retaining larger-scan gains. Final evidence and limitations belong in the review report. Existing absolute performance budgets remain unchanged and required.

## Verification

Run the existing workspace, strict lint, architecture, native and packaged acceptance, real ast-grep and TypeScript, and performance-budget checks. Add proof tests for shipped canaries, Unicode/scoped flags, anchors, alternatives/optional literals, line boundaries, input limits and backtracking errors. Differential CLI cases compare the preserved base binary with the candidate on the same paths and rules. Linear proof tests use the original linear matcher as oracle; fancy-regex is not a substitute where the existing engines differ on scoped flags.

No successful test or benchmark is claimed merely because it was scheduled. Exact-head CI and a substantive documented self-review remain prerequisites to ordinary protected merge. Raw measurements and validation outcomes are retained under docs/reviews/evidence/slopgate-performance-20260929/.

## Review authorization

The owner explicitly requested this performance implementation and comparison benchmarks, with no functional regressions. This decision records the two new benchmark scripts covered by the existing protected-script metadata policy. It does not grant authority to weaken tests, change rules, alter CI or budgets, or bypass hosting protections. Review and normal merge use the owner's autonomous-review authorization in ADR 0004. The implementation review is self-review, not an independent human approval, and must be recorded honestly.
