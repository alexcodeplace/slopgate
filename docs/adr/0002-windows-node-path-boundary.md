# ADR 0002: Keep canonical ownership checks separate from Node path representation

Governance update: ADR 0004 supersedes this record's independent-human-review prerequisite. Its technical decisions and historical evidence remain applicable.

Status: proposed implementation correction requiring independent human approval with the universal-gate pull request. This does not change or weaken the architecture specification.

## Decision

Keep Windows path canonicalization and package-entrypoint containment checks in the adapter before execution. Convert only safely equivalent namespace-prefixed paths to the conventional Windows spelling at the Node subprocess boundary. Use the pinned Windows-only `dunce` 1.0.5 helper rather than implement another partial Windows filename parser. The core remains language-neutral and does not learn about Node, npm, or TypeScript.

The Windows CI run for revision 9a5c2e5 demonstrated that the real TypeScript compiler was never starting: Node's CommonJS entrypoint loader rejected the canonical `\\?\C:\...` entrypoint with `EISDIR` while resolving `C:`. The gate correctly returned incomplete, but ordinary Windows projects were unusable. The upstream Node issue is https://github.com/nodejs/node/issues/60435; Rust documents this interoperability concern at https://doc.rust-lang.org/std/fs/fn.canonicalize.html.

## Compatibility

No rule, gate severity, failure classification, subprocess deadline, or package containment check is relaxed. The adapter still invokes Node with an argument array and never evaluates a command shim as shell text. Compiler arguments representing safely simplifiable paths are converted along with the validated entrypoint. Paths that cannot be simplified without changing Windows filename semantics remain an explicit adapter error instead of being redirected to a different object. Other operating systems do not acquire this runtime dependency.

## Performance

The helper performs lexical path simplification at invocation time. It does not launch another process, scan source files, or install tools. Existing batching, checker concurrency, and deadline behavior are unchanged. Full release performance CI remains required; this correction does not justify increasing a budget.

## Verification

Windows unit tests assert that the chosen entrypoint retains the same canonical filesystem identity, ordinary and Unicode/space-containing paths are represented correctly, argument boundaries remain intact, and reserved or ambiguous namespace paths are refused. The existing real-TypeScript project and project-reference acceptance tests must pass on native Windows. All Linux, macOS, packaged-launcher, architecture, drift, and performance checks remain required. Unexecuted tests must not be described as passing.

## Human approval

The new dependency allowlist entry and this boundary correction must be reviewed by a human distinct from the implementation/push identity. This record accompanies a protected-surface change; it does not approve itself, activate branch protections, or override the separate hosting prerequisites recorded in `docs/architecture/activation.md`.
