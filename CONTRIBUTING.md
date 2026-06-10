# Contributing to slop-gate

Thank you for contributing to slop-gate! This guide covers development setup, testing, and how to add new features.

## Development Setup

```bash
git clone https://github.com/alexcodeplace/slop-gate.git
cd slop-gate
npm install
npm run self-test
```

The project uses Node.js native `node --test` runner (no external test framework). All tests are colocated with source files using the `*.test.mjs` naming convention.

## Project Structure

```
slop-gate/
├── bin/
│   └── slop-gate                       # CLI entry point
├── src/
│   ├── cli.mjs                         # Command dispatcher
│   ├── config.mjs                      # Config loader + resolver
│   ├── gate.mjs                        # Main gate logic (fast/commit tiers)
│   ├── regex-engine.mjs                # Pattern matcher
│   ├── ast-engine.mjs                  # AST-grep runner
│   ├── checkers/
│   │   ├── index.mjs                   # Checker registry
│   │   ├── tsc.mjs                     # TypeScript checker adapter
│   │   ├── knip.mjs                    # Dead-code checker adapter
│   │   ├── jscpd.mjs                   # Copy-paste checker adapter
│   │   ├── depcruise.mjs               # Architecture checker adapter
│   │   ├── type-coverage.mjs           # Type-coverage checker adapter
│   │   ├── diff-shape.mjs              # Mixed-concerns checker adapter
│   │   ├── shared.mjs                  # Shared checker utilities
│   │   └── *.test.mjs                  # Checker tests
│   ├── ratchet.mjs                     # Baseline fingerprinting + filtering
│   ├── audit.mjs                       # Architecture health report
│   ├── report.mjs                      # Violation output formatter
│   ├── suppressions.mjs                # Suppression line-hash logic
│   ├── install-hooks.mjs               # Git hook installer
│   ├── init.mjs                        # Repository onboarding
│   ├── selftest.mjs                    # Self-test runner
│   └── *.test.mjs                      # Unit tests
├── hooks/
│   ├── commit-hook.sh                  # Claude Code PreToolUse hook
│   └── edit-hook.sh                    # Claude Code PostToolUse hook
├── rules/
│   └── baseline/
│       ├── index.mjs                   # Baseline rule packs
│       ├── ast/
│       │   ├── *.yml                   # AST rules (ast-grep syntax)
│       │   └── fixtures/               # Test fixtures
│       └── selftest.config.mjs         # Config for self-test
├── package.json
├── README.md
├── CONTRIBUTING.md
└── LICENSE
```

## Running Tests

### Self-Test (Comprehensive)

```bash
npm run self-test
```

Runs:
1. **Parser fixtures** — validates checker output parsers against recorded real tool outputs
2. **Canary tests** — verifies regex/AST rules match their canaries and skip negativeCanaries
3. **Unit tests** — ratchet fingerprints, suppressions, config resolution, hook installation
4. **Diff-shape logic** — internal unit test (no external tool)

All test files are named `*.test.mjs` and use Node.js native `assert` module.

### Run a Single Test File

```bash
node --test src/gate.test.mjs
```

### Watch Mode (Manual)

No watch runner is configured. Edit → run tests manually or integrate with your editor.

## Adding a New Checker Adapter

Commit-tier checkers live in `src/checkers/`. Each is a module exporting a default object:

```javascript
// src/checkers/my-tool.mjs
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';

export default {
  id: 'my-tool',  // used in fingerprints + report, stable identifier

  /**
   * Is this tool usable in this repo?
   * @param {import('../config.mjs').ResolvedConfig} config
   * @returns {{ available: boolean, reason?: string }}
   */
  detect(config) {
    // Check if tool is installed
    const bin = join(config.repoRoot, 'node_modules/.bin/my-tool');
    if (!existsSync(bin)) {
      return { available: false, reason: 'my-tool not installed' };
    }
    // Check for tool config if needed
    if (!hasMyToolConfig(config.repoRoot)) {
      return { available: false, reason: 'no my-tool config' };
    }
    return { available: true };
  },

  /**
   * Run the tool and parse its output.
   * Never throw; tool crash/timeout → errors[] only.
   * @param {import('../config.mjs').ResolvedConfig} config
   * @param {{ files: string[] }} ctx - staged file list (commit tier only)
   * @returns {{ violations: import('../gate.mjs').Violation[], errors: string[] }}
   */
  run(config, ctx) {
    const violations = [];
    const errors = [];

    try {
      const timeout = config.checkers?.['my-tool']?.timeout ?? 60_000;
      const result = spawnSync('my-tool', ['--json'], {
        cwd: config.repoRoot,
        timeout,
        encoding: 'utf8',
      });

      if (result.error) {
        errors.push(result.error.message);
        return { violations, errors };
      }

      if (result.status !== 0 && result.status !== 1) {
        errors.push(`my-tool exited ${result.status}: ${result.stderr?.slice(0, 100)}`);
        return { violations, errors };
      }

      // Parse JSON output
      const output = JSON.parse(result.stdout);
      for (const issue of output.issues) {
        violations.push({
          id: `my-tool-${issue.code}`,          // ruleId for this violation
          text: issue.message,                  // short message (≤90 chars)
          severity: 'high',
          category: 'code-quality',
          file: issue.file,                     // repo-relative path
          line: issue.line,
          fullLine: issue.source || '',         // source line text
          resolution: 'Fix the issue.',
        });
      }
    } catch (e) {
      errors.push(`my-tool parse error: ${e.message}`);
    }

    return { violations, errors };
  },
};
```

### Add to Checker Registry

Update `src/checkers/index.mjs`:

```javascript
import myTool from './my-tool.mjs';

export const CHECKERS = [tsc, knip, jscpd, depcruise, typeCoverage, diffShape, myTool];
```

### Add Tests

Create `src/checkers/my-tool.test.mjs`:

```javascript
import { test, describe } from 'node:test';
import assert from 'node:assert';
import myTool from './my-tool.mjs';
import { BASELINE_FIXTURES_DIR } from '../../rules/baseline/index.mjs';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

describe('my-tool checker', () => {
  test('detect returns available=true when tool + config present', () => {
    const config = { repoRoot: '/fake' }; // mock
    const result = myTool.detect(config);
    assert.ok(result);
  });

  test('parses output correctly', () => {
    const fixtureOutput = readFileSync(
      join(BASELINE_FIXTURES_DIR, 'checker-outputs/my-tool.json'),
      'utf8'
    );
    const mockResult = {
      status: 0,
      stdout: fixtureOutput,
      stderr: '',
    };
    // Mock spawnSync; call run(); assert violations match expected
  });

  test('handles timeout gracefully', () => {
    // Verify errors[] captures timeout message
  });
});
```

### Add Fixture Data

Record real tool output and expected violations:

```bash
# Run your tool and capture output
my-tool --json > rules/baseline/fixtures/checker-outputs/my-tool.json

# Create expected violations
# rules/baseline/fixtures/checker-outputs/my-tool.expected.json
[
  {
    "ruleId": "my-tool-E001",
    "file": "src/app.ts",
    "line": 10
  }
]
```

## Adding a New Regex Rule Pack

Create a rule pack module in `rules/baseline/` or project `rules/`:

```javascript
// rules/baseline/my-pack.mjs
export const MY_PACK = [{
  id: 'my-rule-1',
  title: 'Description of issue',
  category: 'code-quality',
  severity: 'high',
  pattern: 'regex-pattern',
  flags: 'i',  // optional: i, m, s, g (g/y removed for line scanning)
  description: 'Full explanation of why this is bad.',
  resolution: 'How to fix it.',
  excludeGlobs: ['*.test.ts'],  // optional
  includeGlobs: ['src/**'],      // optional
  minFiles: 1,  // optional: require pattern in ≥ N files
  canary: 'Example code that SHOULD match',
  negativeCanary: [  // Code that SHOULD NOT match
    'false positive example 1',
    'false positive example 2',
  ],
}];
```

Add to baseline pack registry in `rules/baseline/index.mjs`:

```javascript
export const BASELINE_PACKS = {
  'no-stubs': [...],
  'my-pack': MY_PACK,
  // ...
};
```

### Test It

The self-test runner automatically verifies:
- `canary` string matches the pattern
- All `negativeCanary` strings do NOT match

Run `npm run self-test` to validate.

## Adding a New AST Rule

Create a rule in `.yml` (ast-grep syntax):

```yaml
# rules/baseline/ast/my-ast-rule.yml
id: my-ast-rule
pattern: |
  kind: call_expression
  function:
    text: dangerous_function
message: Calling dangerous function without safeguards
severity: high
fix: |
  wrap_with_guard($0)
```

Add test fixtures in `rules/baseline/fixtures/` (follow ast-grep fixture format):

```yaml
# rules/baseline/fixtures/my-ast-rule.case
id: my-ast-rule
rules:
  - id: my-ast-rule
    message: Calling dangerous function without safeguards
fixtures:
  - dangerous_function();  # should match
  - safe_function();       # should not match
```

Self-test will validate it.

## Code Style

- **Node.js modules** — ES modules (`.mjs`), no CommonJS
- **Imports** — Prefer `node:` builtins (e.g., `import fs from 'node:fs'`)
- **Error handling** — Checkers never throw; capture errors in `errors[]`; fail-open on infra
- **Testing** — Node.js native `assert` + `test()`; no external frameworks
- **Comments** — JSDoc for public functions; inline comments for subtle logic
- **Naming** — Clear, searchable names (e.g., `fingerprintViolation`, not `fp`)
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

Set `DEBUG=slop-gate:*` (if logging is added):

```bash
DEBUG=slop-gate:* npm run self-test
```

### Inspect Violations

Use `slop-gate --file` on a sample file:

```bash
node bin/slop-gate --file src/app.ts --config rules/baseline/selftest.config.mjs
```

### Check Config Resolution

Create a test config, then inspect output:

```bash
node -e "
import { resolveConfig } from './src/config.mjs';
const cfg = await resolveConfig('.slop-gate/config.mjs');
console.log(JSON.stringify(cfg, null, 2));
"
```

## Submitting a PR

- Run `npm run self-test` and ensure all tests pass
- Include a clear PR description explaining the change
- Link to any related issues
- If adding a new checker or rule, include test fixtures

## Questions?

Open an issue or discussion on the repo.

---

Happy contributing!
