# ADR 0001: Universal policy coordinator with specialized adapters

Status: implementation proposal requiring independent human approval before merge. This record does not grant its author permission to approve a specification change.

## Decision

Keep a Rust coordinator and separate concrete checker implementations into `slopgate-adapters`. The CLI composes an injected registry. Add a versioned JSON executable adapter protocol so another language can integrate without changing or recompiling the core. Keep shared regex and structural engines, with project-owned rule packs and explicit capabilities. Use syntax-aware architecture checks and fixture-backed CI to make the documented boundaries executable.

Required checking failures become incomplete outcomes rather than a successful empty scan. The bounded process runner owns timeouts, output bounds and descendant cleanup. The staged gate refuses an inconsistent working tree instead of claiming it validated the index. Infrastructure failures are not suppressible or baselineable. Baseline occurrence quotas prevent newly copied identical findings from inheriting unlimited historical waivers.

## Compatibility

This is the pre-1.0 0.4.0 contract. Existing finding IDs remain stable, and old baseline entries represent one occurrence. Each suppression entry likewise authorizes one occurrence rather than unlimited identical copies. The reusable CI workflow now requires an explicit released version and uses a disposable hosted runner; an optional repository-owned setup script provisions project tools before checking. Unknown checker IDs, misspelled configuration keys, missing rule directories and invalid tool output now fail instead of being ignored. Intentional project rule overrides require an explicit reviewed `ruleOverrides` declaration. Project rule self-tests require exact positive and negative fixture evidence. Consumers must provision their selected tools explicitly and should review their configuration before upgrading.

TypeScript remains a specialized adapter, not a constraint on the core. Cargo and a Python protocol fixture exercise other language paths. The protocol does not promise arbitrary language coverage, sandboxing, or full behavioral verification: those capabilities must be supplied and tested by the selected adapters.

## Performance

Use the linear regex engine where compatible, reuse compiled globs across files, prefilter compatibility expressions only with proven necessary literals, batch work per adapter, and partition nested concurrency. Keep tool-owned incremental TypeScript state. Do not introduce unsafe semantic result caching or require a daemon. Release-mode benchmark budgets measure native single-file and repository work separately from real compiler cost. Failing a performance budget does not disable checking.

The native reference target is below 100 ms median for a small-file policy scan. External full-project semantic checks cannot honestly be promised to complete instantly. Actual timings and platform evidence belong in the delivery report and uploaded CI artifacts, not assumptions in this decision record.

## Verification

Acceptance covers real CLI file/full/staged paths, Python executable adapters, actual TypeScript and Cargo compilers, malformed output, missing tooling, contradictory statuses, bounded timeouts and output, inconsistent staging, config-only changes, deletions, Unicode filenames, baseline safety, concurrent execution and the actual npm package launcher. The Rust architecture guard validates dependency direction and source ownership, with mutation tests for forbidden imports, subprocess access, unsafe functions and module redirection. Trusted-base drift checking reads candidate Git objects as data and never executes candidate code with privileged permissions.

A passing implementation test is not evidence that hosting controls are already active. The activation checklist and read-only hosting verifier cover that separate trust boundary. The delivery report must explicitly identify any unexecuted platform test or unresolved activation prerequisite.

## Human approval

A reviewer distinct from the implementation/push identity must inspect this architecture, the protected policy surfaces, compatibility corrections, test results and performance evidence before merge. Code-owner review, latest-push approval, stale-approval dismissal and required checks must be enforced by GitHub. The current shared administrator identity cannot provide an independent approval. An administrator must arrange a separate reviewer and non-admin agent credentials without weakening these requirements.

## Upstream integration

The implementation began from c145921 and was reconciled with ee59c2e (upstream 0.3.4). Existing fail-closed AST and baseline fixes are retained by the stronger completion contract. Socket-safe hook input, durable hook anchoring, package-owned ast-grep resolution and the published AST dependency are preserved. The proposed universal release is 0.4.0, not a downgrade to 0.3.0. No package has been published by this task.
