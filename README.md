# Slopgate

A deterministic, language-neutral code-policy gate. One Rust coordinator runs shared text/structural checks and specialized adapters, applies reviewed project policy, and returns one trustworthy result.

This tree implements the pre-1.0 **0.3.0 universal-gate contract**. Do not assume a globally installed older binary has these capabilities. Use `slopgate capabilities` to inspect the selected executable, source digest and adapter contract.

## Architecture

```text
slopgate-rs: CLI and composition root
    ├── slopgate-core: configuration, discovery, scheduling, shared scanners,
    │                  bounded processes, baselines, suppressions and reporting
    └── slopgate-adapters: concrete tools and language-aware integration
              └── depends on slopgate-core, never the reverse
```

External executable adapters use a versioned JSON protocol and can be implemented in any language. Adding a language does not require inventing a new architecture or putting its compiler into the core. File discovery is not proof of parser or semantic coverage.

The authoritative contract is [universal-gate-v1](docs/specs/universal-gate-v1.md). Dependency and source boundaries are checked by the syntax-aware architecture guard. Policy changes require an ADR and independent human review. [Hosting activation](docs/architecture/activation.md) explains the external controls needed to make those checks mandatory.

## Build and verify

```bash
cargo build --release --locked --workspace
./target/release/slopgate-rs capabilities
cargo test --workspace --locked
cargo run --locked -p slopgate-architecture -- --root .
```

CI uses Rust 1.95.0, provisioned TypeScript 5.9.3 and ast-grep 0.45.2 for its fixture tests. Native platform tests and real compiler acceptance are separate from mocked-output parser tests. Tool installation is an explicit setup action; a scan never downloads a missing checker.

The npm distribution uses `bin/slopgate` to select a prebuilt executable under `vendor/<platform>-<arch>`. The native executable can also run directly without Node. Node is required only for the npm launcher and adapters that use Node tools. An explicit `SLOPGATE_BIN` is authoritative: a missing or recursive override is an error, not permission to use a stale fallback binary.

## Configure a project

```toml
roots = ["src"]
exts = []
skipDirs = [".git", "node_modules", "dist", "target"]
astEnabled = false
rules = ["./rules/project.json"]
fixtures = "./fixtures"
checkerConcurrency = 3

[gate]
file = ["critical", "high"]
staged = ["critical", "high"]
```

Place this at `.slopgate/config.toml`. Scan roots are repository-relative. Rule, fixture and suppression paths are configuration-directory-relative. Empty `exts` selects all files under the roots; narrow roots/extensions or exclusions when a tree contains generated/binary assets. Tests are not globally excluded. Rule-owned `scanTestFiles`, include globs and exclude globs determine each text rule's scope.

A project-owned JSON regex pack is supported:

```json
{
  "project": [{
    "id": "project/no-placeholder",
    "severity": "high",
    "pattern": "FORBIDDEN_PLACEHOLDER",
    "resolution": "Implement the required behavior.",
    "canary": "FORBIDDEN_PLACEHOLDER",
    "negativeCanary": ["implemented behavior"],
    "scanTestFiles": true
  }]
}
```

Regex rules are line-scoped text policies, not type checking. Compatible expressions use a linear engine; advanced expressions use a bounded compatibility engine. Evaluation/resource errors make the scan incomplete. Pattern semantics, path globs and positive/negative cases must be tested. Intentional ID collisions require a reviewed `ruleOverrides = ["rule-id"]` declaration; accidental overrides fail configuration.

For structural rules, provision ast-grep and set `astEnabled = true` plus `astRules = "./rules/ast"`. The same selected files reach the structural engine regardless of language extension. Its inspection details show what was actually scanned; discovered files are not automatically analyzed by every rule. A configured missing rule directory is an error.

## Specialized checks

Built-ins include `tsc`, `cargo-check`, Knip, dependency-cruiser, jscpd, type-coverage, leakscan, ShellCheck, actionlint, typos and diff-shape. The exact configured IDs/scopes are listed by `capabilities` and `doctor`.

```toml
[checkers.tsc]
tsconfig = ["packages/api/tsconfig.json", "packages/ui/tsconfig.json"]
incremental = true
required = true
timeout = 120

[checkers.cargo-check]
required = true
timeout = 120
```

TypeScript resolves project scope through the selected compiler and checks its configured projects, not just changed files. Empty or unresolved scope cannot silently pass. Reference-containing projects require explicit `build = true`, selecting TypeScript reference/build semantics; build output and build-info topology remain TypeScript-owned in that mode. Cargo checks the configured workspace offline and locked; provision dependencies first. Required checks default to required. An explicit optional check (`required = false`) may fail without blocking, but that outcome remains visible.

The [adapter protocol](docs/adapter-protocol.md) documents executable integrations, scopes, tiers, validation, resource limits and exact fixture contracts.

## Run the gate

```bash
# Native fast tier on one edited file.
slopgate --file src/example.py --config .slopgate/config.toml

# Commit tier, including applicable full-project checks.
slopgate --staged --config .slopgate/config.toml

# Immutable CI snapshot, human / JSON / GitHub annotation output.
slopgate scan --scope repo --tier commit --format json --config .slopgate/config.toml

# Inspect configuration, executable provenance and available checks.
slopgate doctor --config .slopgate/config.toml

# Verify rule canaries, exact project fixtures and checker parser contracts.
slopgate --self-test --config .slopgate/config.toml
```

Exit **0** means complete with no blocking findings. Exit **1** means complete with blocking policy findings. Exit **2** means configuration or required analysis was incomplete. Infrastructure failures cannot be suppressed or absorbed into a baseline. Structured output includes findings, errors and per-check coverage; an optional failure is not disguised as a completed check.

The staged gate conservatively refuses partial staging, relevant missing working-tree files and untracked inputs. It does not stash or reset user work. Config-only, lockfile-only and deletion-only changes still run applicable project checks. This v1 policy is intentionally stricter than pretending a working-tree scan validates a different Git index.

## Baselines and rule proof

```bash
slopgate baseline --config .slopgate/config.toml
slopgate baseline --prune --config .slopgate/config.toml
slopgate baseline --update --config .slopgate/config.toml
```

Creation refuses an existing baseline. Pruning removes resolved allowances without absorbing new findings. Updating explicitly resnapshots current debt and requires review. Baseline occurrence counts prevent additional identical findings from inheriting an unlimited waiver. No baseline mutation may proceed from an incomplete scan. CI does not update baselines automatically.

Project rules require positive/negative canaries and an exact `expectations.json` fixture contract. Each case declares its input and complete expected finding multiset, including rule ID, engine and one-based location. Required rules have separate positive and negative witnesses. Fixture execution uses the normal file pipeline and respects the rule's own file globs. `defect record` and `harvest --check` retain the repeated-defect workflow; harvesting no longer restricts fixtures to TypeScript extensions.

## Correctness before apparent speed

The coordinator batches work, bounds concurrency, amortizes pattern/glob compilation and keeps native fast scans separate from heavy semantic checks. It needs no daemon. V1 intentionally has no external-result cache: tool-owned incremental state is safer than an unsound cache keyed only by a changed file.

```bash
python3 scripts/benchmark.py --binary target/release/slopgate-rs \
  --semantic --output artifacts/performance.json
```

The benchmark records binary provenance, hardware, sample counts, medians and p95s. Native file/repository work and actual TypeScript checking are measured separately. Reviewed CI budgets fail on regressions rather than disabling checks. See [delivery evidence](docs/reviews/universal-v1-delivery.md) for executed results and remaining activation prerequisites; do not treat a target latency as a measured guarantee.

## Trust boundary

Slopgate is not an adapter sandbox or proof of arbitrary behavioral correctness. Run untrusted project tools on disposable CI workers without deployment secrets. Code-owner rules and required checks need server-side enforcement, independent human review and non-admin agent credentials. An agent using the administrator's identity can change the protections themselves; repository code cannot remove that authority.
