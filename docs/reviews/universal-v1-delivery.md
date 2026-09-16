# Universal gate v1 delivery evidence

Status: implementation and automated verification completed for the source revision below. Independent human approval, protected-base bootstrap and least-privileged agent access are not active. This document is evidence, not approval to merge or a claim that repository administrators cannot bypass their own controls.

## Provenance and delivery location

- Repository: `alexcodeplace/slopgate`.
- Pull request: https://github.com/alexcodeplace/slopgate/pull/22, open and not merged by this task.
- Source branch: `feat/universal-gate-20260915`.
- Verified implementation revision: `8e515f1e3ba029818c03d45d809097dcb082a1d7`.
- Hosted CI run: https://github.com/alexcodeplace/slopgate/actions/runs/35043860320, all seven jobs succeeded.
- CI performance build revision: `d05a1b3c5f0e71b5fe526977453c9772c92a303b`, the GitHub pull-request merge ref, not a different implementation claim.
- Native source-input digest: `2b8e53aed6e838edd19b68c322b97b4f0127191c9d52ab0ceb846487b01d49fa`. The dedicated Linux build and hosted performance build agree on this digest. This identifies the inputs hashed by `crates/slopgate-rs/build.rs`, not every file in the repository.
- Proposed package contract: 0.4.0. No package or release was published.

The work began at c145921 and incorporated upstream ee59c2e/0.3.4 safeguards. Source changes live only in the isolated worktree. The five unrelated changes in the original main checkout were not edited, reset, stashed or committed by this task.

## Automated correctness and integration evidence

| Verification | Observed result |
| --- | --- |
| Formatting and strict lint | `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed locally and in CI |
| Dedicated Linux workspace | 321 non-documentation tests passed: core 189, adapters 97, architecture 5, CLI 23, leakscan integration 4, golden 1, workflow contract 2 |
| Executable architecture review | Workspace membership/dependencies, neutral-core boundaries, process ownership and requirement traceability passed |
| Real CLI/compiler acceptance | 27 tests passed on dedicated Linux, including actual TypeScript 5.9.3, Cargo/rustc, native AST, and Python executable adapters |
| Metadata drift mutations | 6 tests passed, including protected embedded rules, hooks, launchers and candidate scripts treated only as data |
| Hosting-verifier mutations | 9 tests passed offline; these prove refusal/validation logic, not that hosting protections are enabled |
| Native Linux CI | Workspace, real compiler acceptance, release build, platform package smoke and actual npm-tarball consumer tests passed |
| Native macOS CI | The same full chain passed on macOS 15 ARM64 |
| Native Windows CI | The same full chain passed on Windows 2025 x64; the newline-filename case is intentionally inapplicable to Windows |
| Workflow lint | Actionlint passed |
| Release performance | Every median and p95 budget passed, including actual AST execution and real incremental TypeScript |

CI job names are `Slopgate quality`, `Slopgate architecture`, `Slopgate Linux`, `Slopgate macOS`, `Slopgate Windows`, `Slopgate performance`, and `Slopgate workflow lint`. The separate `slopgate/trusted-spec-drift` status requires the workflow to exist on the protected base branch. It is not presented as active merely because its proposed code passes tests in this PR.

Primary reproduction commands are in `.github/workflows/ci.yml`. The explicit test-tool provisioning script pins versions and exposes the native AST executable; no scan installs dependencies or relies on a daemon. The package tests unpack the actual `npm pack` tarball, verify selected native source identity, run bundled self-tests, enforce a non-TypeScript project rule, and reject invalid or recursive binary overrides. Other CPU targets are not represented as natively tested by this three-platform matrix.

## Measured performance

Hosted measurements below came from an Ubuntu runner with four logical CPUs, Python 3.12.3, Rust 1.95.0, ast-grep 0.45.3 and TypeScript 5.9.3. All measurements launch the real release CLI. There is no cross-run Slopgate result cache or warm daemon.

| Workload | Median | p95 | Samples |
| --- | ---: | ---: | ---: |
| Small native text-policy scan | 7.135 ms | 7.715 ms | 31 |
| Small native scan including real AST | 18.523 ms | 20.655 ms | 21 |
| 150 KB negative long-line regression | 7.197 ms | 7.465 ms | 15 |
| Positive long-line policy finding | 7.740 ms | 12.361 ms | 15 |
| 1,001-file native text-policy repository | 34.271 ms | 36.085 ms | 9 |
| Incremental TypeScript project check | 514.352 ms | 524.661 ms | 5 |

The repository workload contains 32,032 lines and 1,101,100 source bytes. Its measured throughput is about 29,209 files/second for this synthetic workload; it is not a guarantee for arbitrary repositories or semantic checking. The first TypeScript check was 1,250.462 ms. The native AST row is a single-file workload, not whole-repository structural performance.

The dedicated eight-CPU Linux host measured 6.347 ms median for native file scans, 17.445 ms including AST, 21.612 ms for the same repository fixture, and 369.154 ms for the incremental TypeScript fixture. These are different machines and are not interchangeable benchmarks.

Raw results, exact binary hashes, configured budgets and tool versions are committed in:

- `docs/reviews/evidence/performance-ci-8e515f1.json`
- `docs/reviews/evidence/performance-linux-reference-8e515f1.json`

The reference report also includes observed ratios against a pre-review release executable on matching machine metadata. Several small-file/semantic samples are slower while repository samples are faster; no improvement is claimed merely from noisy ratios. The benchmark explicitly does not control concurrent host load or physical cold-disk state. Absolute required budgets remain authoritative. Expensive full-project checkers cannot honestly be promised to complete instantly.

## Correctness review and repairs

These were implementation self-review and test-driven repair passes, not an independent human review.

**Architecture and coverage.** Concrete checkers, stack discovery and checker-specific reporting moved to the adapters crate, with a registry injected by the CLI. Source selection no longer hard-codes TypeScript extensions. Tests cover TS/TSX/MTS/CTS, Python, Rust, Astro text policy and arbitrary extensions without coordinator branches. Python structural matching and a Python executable protocol prove independent integration paths. Discovering an extension does not imply a parser or semantic checker exists for it. Reserved shared-engine IDs prevent misleading duplicate coverage identities.

**Required completion and policy safety.** Missing tools/directories, invalid configuration, malformed or contradictory output, regex work-limit failures and process failures return incomplete rather than clean. Incomplete scans cannot mutate baselines. Omitted severity tiers retain defaults; explicit empty tiers remain deliberate configuration. Invalid UX data cannot silently disable policy. Project rule overrides require explicit reviewed declarations. Baseline and suppression occurrence quotas prevent new identical copies inheriting unlimited waivers.

**Proposed-commit correctness.** Partial staging, missing working-tree inputs and untracked analysis inputs are rejected without stashing or rewriting files. NUL-delimited path handling supports spaces/Unicode/newlines where the platform permits them. Before/after proposed-tree identities catch changes that an adapter also stages. Config-only and deletion-only changes still execute project checks. Linked worktrees and absolute editor paths have regression coverage.

**Process and cross-platform behavior.** Required analysis uses one bounded runner. Deadlines, captured-output limits and process ownership are enforced; child processes cannot keep pipe-reader threads hung because capture does not depend on blocking pipes. Unix retains the leader PID until cleanup. Darwin's zombie-only EPERM case is accepted only after bounded native membership inspection. Windows uses Job Objects. Tests demonstrate an exited adapter cannot leave its ordinary descendant running. This is lifecycle management, not a sandbox against a malicious program deliberately escaping OS containment.

**Windows TypeScript repair.** An actual compiler acceptance failure exposed Node rejecting namespace-prefixed canonical entrypoint paths. The adapter now preserves canonical ownership checks and only simplifies equivalent paths for Node, using pinned Windows-only `dunce`. Reserved or ambiguous namespace paths are refused. Neither shell-string evaluation nor containment weakening was introduced. Real compiler, reference-project and packaged Windows tests passed after the repair.

**Performance and compatibility.** Compatible regexes use a linear engine; bounded compatibility matching uses necessary-literal proofs, not rule-ID-specific shortcuts. Globs are compiled once per scan. Diagnostics and source reads are bounded. Nested concurrency is partitioned and deterministic reporting is tested. Tool-owned incremental state remains supported, with no unsound external result cache. Historical checker-health telemetry was restored from typed outcomes, with atomic bounded cache handling; it cannot authorize a pass or suppress a required error.

**Governance and drift.** Mutation tests reject forbidden imports, compound imports, whole-standard-library aliases, conditional/qualified source inclusion, unsafe code outside its reviewed owner and dependency-boundary violations. Workspace inheritance follows Cargo path semantics. Protected drift coverage includes embedded policy, packaging, hooks and verification/provisioning scripts. Candidate metadata is read from Git blobs by the trusted base script, never executed with elevated workflow permissions. The hosting verifier rejects arbitrary status producers, missing/inactive trusted workflow, self-owned policy, custom roles and unverified owner access.

## Remaining human-controlled activation blocker

Read-only inspection at this checkpoint returned `Branch not protected` (HTTP 404) for main, no repository rulesets, and one collaborator: `alexcodeplace`, with admin/maintain permissions. The same identity is exposed to the implementation tools. PR 22 has no independent approving review. Running `scripts/verify-hosting.py --repo alexcodeplace/slopgate --agent-login alexcodeplace` correctly returned exit 2 with `verified=false`.

A separate human reviewer/code owner with write access and dedicated non-admin, non-owner agent credentials are required. The human must review and merge the bootstrap, activate App-bound required checks and review protections, and verify the trusted-base drift status using the documented disposable-PR test. See `docs/architecture/activation.md` and `docs/architecture/hosting-protection.json`.

No agent has self-approved, merged this PR, modified repository protections or published a release. Automated code correctness is not a substitute for that independent authority boundary. The overall user goal is therefore not marked DONE.
