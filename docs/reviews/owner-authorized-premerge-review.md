# Owner-authorized pre-merge review

Authorization: the owner explicitly asked the assistant to review this code itself and merge when ready. ADR 0004 records the governance correction. This is a substantive self-review, not an independent human approval and not a GitHub self-approval event.

## Reviewed boundaries

The current source has one-way crate dependencies: concrete adapters depend on core, and CLI composes the registry. Core scheduling does not branch on concrete language IDs/extensions. Shared scanners use configured discovery; semantic adapters receive whole-project scope. Protocol output validates version, completion, exit status, finding IDs, severity, path, locations and gate-owned provenance. All required failures return incomplete before filtering/baselining. Staged input checks reject partial/untracked state and compare before/after proposed-tree identity. Process execution has deadlines, bounded capture, Unix retained-PID ownership and Windows Job Objects. Output and baseline ordering are deterministic and occurrence quotas prevent duplicated findings from inheriting unlimited exemptions.

Reviewed tests include real TypeScript, Cargo, Python protocol and AST execution; Linux/macOS/Windows packages; missing/malformed tool behavior; process descendants; config-only/deletion-only commits; user paths; exact fixture assertions; architecture mutations; and measured release benchmarks. The previous head's seven hosted checks were verified from the GitHub API, not inferred from documentation.

## Repairs from this review

The earlier governance requirement for an independent human was not requested by the owner. Spec, owner instructions, verifier, tests and documentation now implement owner-authorized self-review with mandatory PRs/checks and no external approval. Only that approval policy is relaxed; runtime correctness and architecture enforcement are not waived.

Response normalization could open a checker-nominated FIFO that was outside ordinary discovery. Added an explicit regular-file check before opening diagnostic source content and a timeout-bounded POSIX acceptance test that exercises the protocol path. This is a fail-closed robustness fix, not a new rule or an adapter sandbox guarantee. Deliberate concurrent filesystem replacement remains outside the threat model of trusted adapters.

## Merge decision conditions

The exact final head must pass the full hosted CI suite, including the new regression. Bootstrap protections require seven App-bound checks before merging normally with an expected head SHA. After the trusted workflow reaches main, require its eighth status and prove rejection/acceptance in a disposable live PR. Record the resulting main SHA and verifier output. Do not bypass failed checks, approve the PR as a fabricated second identity, or disturb the original checkout's five unrelated modified files.
