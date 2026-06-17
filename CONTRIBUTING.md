# Contributing to slopgate

Thank you for contributing to slopgate! This guide covers development setup, testing, and how to add new features.

## Development Setup

```bash
git clone https://github.com/alexcodeplace/slopgate.git
cd slopgate
npm install
cargo build --release    # builds the native engine (target/release/slopgate-rs)
npm run self-test
```

The engine is a native Rust workspace. `bin/slopgate` is a thin Node launcher that
locates and execs the compiled binary (`target/release/slopgate-rs`), so the engine
must be built before `bin/slopgate` (and therefore `npm run self-test`) will run.

Tests use Cargo's built-in runner (`cargo test --workspace`). Unit tests are
colocated with source as `#[cfg(test)]` modules; integration tests live under
`crates/*/tests/`.

## Project Structure

```
slopgate/
├── bin/
│   └── slopgate                       # Thin Node launcher → execs the native binary
├── crates/
│   ├── slopgate-core/                 # Engine library crate
│   │   └── src/
│   │       ├── lib.rs                  # Crate root / module wiring
│   │       ├── config.rs               # Config loader + resolver (TOML)
│   │       ├── gate.rs                 # Main gate logic (fast/commit tiers)
│   │       ├── regex_engine.rs         # Pattern matcher
│   │       ├── ast_engine.rs           # AST-grep runner
│   │       ├── enumerate.rs            # File enumeration
│   │       ├── glob.rs                 # Glob matching
│   │       ├── hash.rs                 # Hashing
│   │       ├── ratchet.rs              # Baseline fingerprinting + filtering
│   │       ├── report.rs               # Violation output formatter
│   │       ├── suppressions.rs         # Suppression line-hash logic
│   │       ├── selftest.rs             # Self-test runner
│   │       ├── severity.rs             # Severity model
│   │       ├── error.rs                # Error types
│   │       ├── help.rs                 # CLI help text
│   │       ├── temp.rs                 # Temp-file helpers
│   │       ├── checkers/               # Commit-tier checker adapters
│   │       │   ├── index.rs            # Checker registry
│   │       │   ├── tsc.rs              # TypeScript checker adapter
│   │       │   ├── knip.rs             # Dead-code checker adapter
│   │       │   ├── jscpd.rs            # Copy-paste checker adapter
│   │       │   ├── depcruise.rs        # Architecture checker adapter
│   │       │   ├── type_coverage.rs    # Type-coverage checker adapter
│   │       │   ├── diff_shape.rs       # Mixed-concerns checker adapter
│   │       │   ├── leakscan.rs         # Secret-leak checker adapter
│   │       │   ├── health.rs           # Health checker
│   │       │   └── shared.rs           # Shared checker utilities
│   │       ├── rules/                  # Embedded rule packs (baseline/stack/ux .json)
│   │       ├── audit/                  # Audit subcommand
│   │       ├── stats/                  # Stats store/query/record
│   │       ├── init/                   # Repository onboarding
│   │       └── install/               # Git/agent hook + skill installers
│   └── slopgate-rs/                    # Binary crate
│       └── src/
│           └── main.rs                 # CLI dispatcher → target/release/slopgate-rs
├── tools/
│   └── leakscan/                       # Native secret-scanner helper crate
├── hooks/
│   ├── commit-hook.sh                  # Claude Code PreToolUse hook
│   └── edit-hook.sh                    # Claude Code PostToolUse hook
├── rules/
│   └── baseline/
│       ├── ast/
│       │   └── *.yml                   # AST rules (ast-grep syntax)
│       └── selftest.config.toml        # Config for self-test
├── package.json
├── README.md
├── CONTRIBUTING.md
└── LICENSE
```

## Running Tests

### Local CI Parity

Before opening a PR, run the same checks that CI gates:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --locked -p slopgate-rs
SLOPGATE_BIN=target/debug/slopgate-rs npm run self-test
npm pack --dry-run --json
```

CI also lints GitHub Actions workflows with `actionlint`. It runs `zizmor` for
workflow security findings as a non-blocking advisory check so dependency and
policy updates do not stop unrelated PRs.

`cargo-deny` / `cargo audit` are intentionally deferred for now. They are good
future additions, but need an agreed advisory/license policy and a validated
low-noise config before becoming CI gates.

### Cargo Test Suite

```bash
cargo test --workspace    # also available as: npm test
```

Unit tests are colocated with source as `#[cfg(test)]` modules. Integration
tests live under `crates/*/tests/` — e.g. `crates/slopgate-rs/tests/parity_golden.rs`
with golden vectors under `crates/slopgate-core/tests/parity_vectors/`.

### Self-Test (Comprehensive)

```bash
npm run self-test
```

Runs `slopgate --self-test` against `rules/baseline/selftest.config.toml`, which:
1. **Regex canary tests** — verifies each pattern matches its `canary` and rejects its `negativeCanary` strings
2. **Config validation** — checks configured roots and fixtures dirs exist
3. **AST canary scan** — runs ast-grep over the fixtures and validates the AST rules fire

Engine logic with no canary (ratchet fingerprints, suppressions, config resolution,
diff-shape, etc.) is covered by the Cargo test suite above, not by `--self-test`.

### Run a Single Test

```bash
cargo test -p slopgate-core gate    # run tests whose name matches "gate"
```

## Adding a New Checker Adapter

Commit-tier checkers live in `crates/slopgate-core/src/checkers/`. Each adapter is a
module exposing a `Checker` (see the `Checker` struct in `checkers/index.rs`) with:

- `id` — stable identifier used in fingerprints + report
- `detect(config, opts)` — returns a `DetectResult { available, reason }`; reports
  whether the tool is usable in this repo (installed, configured)
- `run(config, opts)` — runs the tool and parses its output into a
  `CheckerRunResult { violations, errors }`. Never panic; a tool crash/timeout must
  surface in `errors` only (fail-open).

Model a new adapter on an existing one such as `checkers/tsc.rs` or `checkers/knip.rs`.

### Add to Checker Registry

Register the adapter in `crates/slopgate-core/src/checkers/index.rs` (the registry that
wires together `tsc`, `knip`, `jscpd`, `depcruise`, `type_coverage`, `diff_shape`,
`leakscan`).

### Add Tests

Add a colocated `#[cfg(test)]` module to your adapter source, modeled on the test
modules in the existing checker files. For end-to-end coverage, extend the integration
tests under `crates/slopgate-rs/tests/` (e.g. `parity_golden.rs`) and, where relevant,
the parity vectors under `crates/slopgate-core/tests/parity_vectors/`.

## Adding a New Regex Rule Pack

Regex rule packs are embedded in the engine as JSON, deserialized into the `Pattern`
struct in `crates/slopgate-core/src/rules/packs.rs` from
`crates/slopgate-core/src/rules/{baseline,stack,ux}.json`. A pattern object has the
shape:

```json
{
  "id": "my-rule-1",
  "title": "Description of issue",
  "category": "code-quality",
  "severity": "high",
  "pattern": "regex-pattern",
  "flags": "i",
  "description": "Full explanation of why this is bad.",
  "resolution": "How to fix it.",
  "excludeGlobs": ["*.test.ts"],
  "includeGlobs": ["src/**"],
  "minFiles": 1,
  "canary": "Example code that SHOULD match",
  "negativeCanary": ["false positive example 1", "false positive example 2"]
}
```

Add the pattern to the appropriate pack key in the relevant JSON file
(`baseline.json`, `stack.json`, or `ux.json`).

### Test It

The self-test runner automatically verifies:
- `canary` string matches the pattern
- All `negativeCanary` strings do NOT match

Run `npm run self-test` to validate.

## Adding a New AST Rule

Create a rule in `rules/baseline/ast/*.yml` using ast-grep rule syntax. Model it on an
existing rule such as `rules/baseline/ast/inner-html.yml`:

```yaml
# rules/baseline/ast/my-ast-rule.yml
id: my-ast-rule
language: tsx
severity: error
message: Calling dangerous function without safeguards
note: '{"severity":"high","category":"security","resolution":"How to fix it."}'
rule:
  any:
    - pattern: dangerous_function($$$)
```

Self-test (`npm run self-test`) runs the AST rules and validates them.

## Code Style

- **Rust** — keep `cargo fmt` clean and `cargo clippy` warning-free
- **Error handling** — Checkers never panic; capture errors in the result's `errors`; fail-open on infra
- **Testing** — Cargo's built-in test harness; colocated `#[cfg(test)]` modules + `crates/*/tests/`; no external frameworks
- **Comments** — doc comments (`///`) for public items; inline comments for subtle logic
- **Naming** — Clear, searchable names (e.g., `fingerprint_violation`, not `fp`)
- **No breaking changes** — Public APIs (CLI flags, config keys) are stable

## Git Workflow

1. Create a feature branch: `git checkout -b feature/my-feature`
2. Make changes, test locally: `npm run self-test`
3. Commit with clear messages: `fix(checkers): handle tsc timeout gracefully`
4. Push and open a PR

Commit messages:
- `feat(...)` — new feature
- `fix(...)` — bug fix
- `perf(...)` — performance improvement
- `refactor(...)` — code restructuring (no behavior change)
- `docs(...)` — documentation
- `test(...)` — test additions/fixes

## Debugging

### Enable Verbose Output

Set `DEBUG=slopgate:*` (if logging is added):

```bash
DEBUG=slopgate:* npm run self-test
```

### Inspect Violations

Use `slopgate --file` on a sample file:

```bash
node bin/slopgate --file src/app.ts --config rules/baseline/selftest.config.toml
```

### Check Config Resolution

Config is loaded from `.slopgate/config.toml`. Config resolution is covered by tests
in `crates/slopgate-core/src/config.rs` and the parity vector
`crates/slopgate-core/tests/parity_vectors/resolved_config.json`; run them with:

```bash
cargo test -p slopgate-core config
```

## Submitting a PR

- Run the local CI parity commands above and ensure all tests pass
- Include a clear PR description explaining the change
- Link to any related issues
- If adding a new checker or rule, include test fixtures

## Questions?

Open an issue or discussion on the repo.

---

Happy contributing!
