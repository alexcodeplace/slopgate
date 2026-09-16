# Contributing to Slopgate

Read `AGENTS.md` and `docs/specs/universal-gate-v1.md` before changing code. The implementation must follow that architecture; changing the specification to excuse an implementation shortcut is not an acceptable fix.

## Ownership boundaries

`slopgate-core` owns neutral contracts, configuration, file discovery, scheduling, shared scanners, bounded processes, gate decisions and policy state. It must not depend on `slopgate-adapters` or choose behavior by a concrete checker ID or language extension.

`slopgate-adapters` owns concrete checkers, their diagnostic parsers, project discovery and language-aware helpers. `slopgate-rs` is the CLI/composition root and injects the registry. An external adapter implements the versioned JSON protocol rather than introducing another parser or compiler into the core.

All production analysis subprocesses and probes go through the bounded process module. Do not add raw `Command::output`, shell strings, unbounded reader threads, ignored subprocess failures, automatic tool installation, or results that silently treat a required failure as clean code. Unsafe platform lifecycle operations belong only to the reviewed process implementation.

## Development checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -p slopgate-architecture -- --root .
python3 scripts/test_spec_drift.py
python3 scripts/test_verify_hosting.py
```

CI provisions its fixture tools explicitly with `node scripts/provision-test-tools.mjs`. For local real-compiler acceptance, set `SLOPGATE_TEST_TOOLS` to that provisioned directory and add its native ast-grep and `node_modules/.bin` directories to PATH. Set `SLOPGATE_REQUIRE_COMPILERS=1` so absent tools fail acceptance rather than skip it.

```bash
python3 tests/acceptance/test_universal.py --binary target/debug/slopgate-rs
cargo build --release --locked -p slopgate-rs
python3 scripts/benchmark.py --binary target/release/slopgate-rs \
  --semantic --structural --output artifacts/performance.json
```

The packaged acceptance test requires a matching binary staged into `vendor/<host>` using `scripts/build-npm-packages.mjs --stage <host>`. It packs the actual npm tarball, extracts it safely, exercises the real launcher, verifies source identity, runs bundled self-tests and checks a non-TypeScript consumer's project rule.

## Adding an adapter

A built-in adapter implements `detect` and `run` from the neutral checker contract and declares its execution scope in the adapter registry. Errors represent infrastructure/completion failures; warnings are separate. A semantic checker defaults to complete configured project scope. Adding a new language must not require a coordinator branch.

Prefer an external executable integration when no native in-process adapter is needed. Its `complete` response must be valid JSON, exit zero and contain no infrastructure errors. Invalid output, missing tools, timeouts and inconsistent exit/status must fail required checks. See `docs/adapter-protocol.md` for examples and resource limits.

Keep parsers tested against real diagnostic fixtures, and test the actual executable path separately. A fake checker output test is not evidence that a real compiler was invoked. Cross-project changes, missing configuration, malformed output and nonzero exits without diagnostics need explicit coverage.

## Adding or changing rules

Project JSON regex packs are supported. Keep textual, structural and semantic policies separate. Do not add rule-ID-specific scanner behavior to optimize a single pattern. A generic optimization must preserve semantics and be protected by positive, negative and performance regression cases.

Text rules need canaries. Project policy also needs an exact fixture contract: expected rule IDs, engines and locations are compared as a complete multiset through the production file pipeline. Rules must have separate positive and negative witnesses. Preserve the configured rule scope when creating fixtures; a rule that works only through a relaxed self-test path is not proven to work in the gate.

Baseline or suppression growth is a policy change requiring owner-authorized substantive review, not a way to make CI green. Incomplete checks must never write, update or prune baselines. Performance failures are fixed by reducing valid work or improving execution, not by disabling rules or broadening exclusions.

## Architecture and specification changes

The architecture guard checks dependency manifests and Rust syntax, including compound imports, source redirection and unsafe boundaries. Its mutation tests must remain meaningful. Adding a dependency or changing a protected policy requires a new ADR explaining the decision, compatibility, performance, verification and review authorization. Under ADR 0004, the assistant may perform that review and merge after all required checks pass; do not invent an independent-human prerequisite.

The trusted-base drift workflow reads candidate Git blobs as data. Never change it to execute candidate code under `pull_request_target`, consume candidate caches, or accept a self-authored checksum as approval. Repository protection is part of deployment, not something `CODEOWNERS` alone establishes. Shared administrator credentials are a disclosed authority limit, not a requirement for the owner to arrange another reviewer.

Record exact commands, results, binary provenance and measured performance in the delivery evidence. Do not label unrun checks as passing. See `docs/architecture/activation.md` for the hosted bootstrap and verification process.
