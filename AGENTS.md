# Slopgate engineering contract

Read `docs/specs/universal-gate-v1.md` before changing code. It is authoritative. Do not invent a different architecture, weaken checks/tests, update baselines to absorb regressions, or alter the spec to match a shortcut. Report conflicts to the orchestrator.

Core is language-neutral. Concrete language/framework/checker knowledge belongs in `slopgate-adapters`; `slopgate-rs` is the composition root. Analysis subprocesses use the shared bounded process runner. Required infrastructure failures are not passes. No language-specific extension whitelist in the core coordinator or hooks. Preserve generic extensibility, existing rule semantics, and deterministic behavior.

Specification, CI, architecture policy/guard, manifests/lockfiles, rules and baseline/suppression policy are orchestrator-owned. Change those only with explicit orchestrator authorization for the exact files. Implementations may not approve their own contract changes. Do not use `--no-verify`, force-push, stash/reset another person's work, modify hosting protections, or touch sibling worktrees. Do not inspect Fabro or Factory.

Work only in the assigned isolated worktree. Source builds/tests run on an explicitly assigned build host or CI, not by overwriting another project's build directory. Record real test and benchmark evidence. Do not claim mocked tooling is a real compiler run or that external semantic checking is instantaneous. No automatic tool installation in scan execution. No background work may outlive its supervised task without explicit approval.
