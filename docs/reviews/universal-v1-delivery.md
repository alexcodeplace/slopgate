# Universal gate v1 delivery evidence

Status: implementation and verification in progress. This file is not an approval or a claim that hosted branch protection is active.

Source branch: `feat/universal-gate-20260915`, based on `c145921d6304220c8cd6fbae338f97dc51aa0c4e`. Work is isolated from the five unrelated modifications in the main checkout.

## Verified checkpoint

The current implementation compiled on the dedicated Linux build tree with `cargo check --workspace --all-targets --locked`. Before the latest exact-fixture additions, core tests passed 186/186, adapter tests passed 97/97, architecture mutations passed 4/4, golden finding output passed, and two CLI assertions exposed intentionally changed error messages. Those assertions were updated to the stricter position-aware argument contract, not removed. The real compiler acceptance suite exposed a missing structured `infraFailed` output field; the field was added and is awaiting a fresh full run.

## Review findings already repaired

- Concrete checkers, stack discovery and checker-specific reporting were separated from the language-neutral coordinator.
- Required checker failures, missing rule directories, parsing failures and timeouts now prevent a clean gate and baseline mutation.
- Process deadlines and descendant cleanup replaced unbounded analysis subprocess calls. Unix cleanup retains the leader PID until cleanup to avoid a PID-reuse race.
- Regex evaluation failures no longer mean no match; compatible patterns use the linear engine, and compatibility prefilters are based on necessary literals rather than rule IDs.
- File discovery and AST routing no longer hard-code TypeScript extensions. Config-only and deletion-only commits retain applicable project checks.
- Partial staging and untracked analysis inputs are rejected instead of scanning different code from the proposed commit.
- Baselines use occurrence quotas, so adding a second identical finding cannot inherit unlimited historical waivers.
- Project fixtures now support exact positive/negative evidence through the normal file pipeline.
- Architecture mutations cover compound Rust imports, inherited dependencies, unsafe functions and module redirection.

## Remaining verification at this checkpoint

Complete Linux reruns, release benchmarks, packaged-launcher tests, native Windows/macOS CI, final specification drift review and hosted workflow execution are pending. No unrun result is reported as passing. Full result logs, timings and revision provenance will replace this checkpoint before delivery.

## External activation boundary

Independent approval and least-privileged credentials are still required. At inspection, the sole collaborator and available agent identity were both `alexcodeplace`, with administrator privileges. The hosting controls described in `docs/architecture/activation.md` must be independently configured and verified before claiming that agents cannot override their own review gate. No implementation agent may approve its own specification or bypass that prerequisite.

## Hosted cross-platform review findings

First hosted run 35016554644 on c95f354 passed quality, architecture, workflow lint and performance checks. Native-platform workspace jobs correctly blocked on three integration defects rather than reporting a false pass: GitHub PATH ordering selected the npm AST wrapper instead of the provisioned binary; Darwin reported EPERM when only the retained zombie group leader remained; Windows unit fixtures supplied forbidden command shims instead of native executables. Repairs keep failure policies intact: expose the native CI directory explicitly, accept Darwin's zombie-only case only after a bounded native membership query, and compile genuine Windows fixture executables. A new cross-platform acceptance case proves descendants cannot continue after adapter completion. The repaired run must pass before this checkpoint is promoted to final delivery evidence.
