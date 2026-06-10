# slopgate

A global code-quality / anti-slop gate for Claude Code and git. Engine is shared, rules are per-project.

**What it does:** Catches code quality violations in two tiers — a fast post-edit scan (regex + AST rules, instant feedback) and a heavy commit-tier scan (static type checkers, dead-code analysis, architecture rules, copy-paste detection). A **ratchet baseline** lets legacy repos adopt without flooding — only NEW violations block commits; pre-existing ones are baselined and tracked for paydown.

---

## Features

- **Two-tier gate**
  - **Fast tier** (post-edit hook): regex patterns + AST rules, instant feedback as you code
  - **Commit tier** (pre-commit hook): includes heavy checkers (tsc, knip, jscpd, dependency-cruiser, type-coverage, diff-shape) + AST + regex, blocks commits
  
- **Ratchet baseline** — snapshot violations at adoption time; only NEW violations fail the gate. Track debt paydown over time.

- **Six commit-tier checkers**
  - **tsc** — TypeScript type errors (full-project scope)
  - **knip** — dead/unused code (exports, files, dependencies)
  - **jscpd** — copy-paste duplication (token-level)
  - **dependency-cruiser** — architecture rules (cycles, orphans, layer boundaries)
  - **type-coverage** — propagation of `any` type (per-expression tracking)
  - **diff-shape** — wide commits spanning too many directories (encourages focused changes)

- **Shared regex + AST rule packs** — fast-tier and commit-tier both run these
  - `no-stubs` — placeholder/TODO markers
  - `ts-suppress` — TypeScript suppression directives
  - `as-any` — unsafe `as any` casts
  - Semantic rules (empty catch, swallowed errors, console debug left)
  - Test-slop rules (unskipped/unawaited tests, missing assertions)
  - Security rules (eval, dangerously unsafe HTML)
  
- **Native git pre-commit hook** — no daemon, no CI coupling, just git
  
- **Claude Code integration** — hooks into PreToolUse (commit) and PostToolUse (edit) events
  
- **Suppressions** — per-file, per-line, with line-hash stability across edits
  
- **Self-test** — `npm run self-test` validates rule engines + baseline checker parsers

---

## Install

```bash
npm install slopgate
```

Then onboard a project:

```bash
npx slopgate init [path-to-repo]
```

This:
1. Detects TypeScript roots, file extensions, and package layout
2. Scaffolds `.slopgate/config.mjs` with detected checkers enabled
3. Writes `.slopgate/suppressions.json` and `.slopgate/depcruise.cjs` (starter)
4. Creates `.slopgate/convention-sources.json` (hints for authoring project rules from local skills/agents/docs)
5. Creates `.slopgate/rules/ast/` and `.slopgate/fixtures/src/` directories
6. Installs git pre-commit hook (or appends to existing)
7. Merges Claude Code hook settings into `.claude/settings.json`
8. Prints next steps (including: run `slopgate baseline --config .slopgate/config.mjs`)

---

## Quickstart

### Run the gate on staged changes (pre-commit):
```bash
slopgate --staged --config .slopgate/config.mjs
```

### Run on a single file (post-edit, fast tier):
```bash
slopgate --file src/app.ts --config .slopgate/config.mjs
```

### Create/update the baseline:
```bash
# Create baseline (refuses if it exists)
slopgate baseline --config .slopgate/config.mjs

# Update baseline (re-snapshot all current violations)
slopgate baseline --update --config .slopgate/config.mjs

# Prune baseline (remove entries no longer occurring)
slopgate baseline --prune --config .slopgate/config.mjs
```

### Run self-test:
```bash
npm run self-test
```

### Install or reinstall hooks:
```bash
slopgate install-hooks --config .slopgate/config.mjs
```

---

## Command Reference

### `slopgate init [dir]`
Onboard a new repository. Detects roots, extensions, installed checkers, and scaffolds project structure.

**Args:**
- `dir` (optional) — target directory; defaults to `process.cwd()`
- No `--config` required; generates config during init

**Creates:**
- `.slopgate/config.mjs` — project config (roots, extensions, rule packs, checkers, baseline/suppressions paths)
- `.slopgate/suppressions.json` — line-level violation suppressions (empty initially)
- `.slopgate/depcruise.cjs` — starter dependency-cruiser rules (if depcruise detected)
- `.slopgate/convention-sources.json` — hints for authoring project-specific rule packs
- `.slopgate/rules/ast/` and `.slopgate/fixtures/src/` — directories for custom rules and fixtures
- `.git/hooks/pre-commit` — native git pre-commit hook (creates new or appends to existing)
- `.claude/settings.json` — Claude Code hook entries (idempotent merge)

**Next step:** Run `slopgate baseline --config .slopgate/config.mjs` to create the initial ratchet baseline

---

### `slopgate --staged --config <path>`
Run commit-tier gate on staged files. Used by git pre-commit hook and Claude Code PreToolUse hook.

**Flags:**
- `--config <path>` (required) — path to `.slopgate/config.mjs`
- `--tier fast|commit` (optional) — override default tier (default: commit for `--staged`)

**Exit codes:**
- `0` — no violations (or all baselined/suppressed)
- `1` — violations block the commit
- `2` — config error or missing argument

**Output:**
- Violations grouped by source (regex, ast, checker:tsc, etc.)
- Baselined count footer
- Skipped checkers (if tool/config missing)

---

### `slopgate --file <path> --config <path>`
Run fast-tier gate on a single file (post-edit). Used by Claude Code PostToolUse hook.

**Flags:**
- `--file <path>` (required) — repo-relative path to check
- `--config <path>` (required) — path to `.slopgate/config.mjs`
- `--tier fast|commit` (optional) — override default tier (default: fast for `--file`)

**Exit codes:** same as `--staged`

**Output:** violations in the touched file only; no baseline filtering

---

### `slopgate baseline --config <path> [--update] [--prune]`
Manage the ratchet baseline.

**Flags:**
- `--config <path>` (required)
- `--update` — re-snapshot all current violations (overwrites baseline)
- `--prune` — remove entries whose fingerprint no longer occurs (dry-run only)
- Both flags can be combined; `--prune --update` prunes then updates

**Behavior:**
- No flags, file missing → create baseline with current violations
- No flags, file exists → error (refuses overwrite; use `--update`)
- `--update` → snapshot all violations in full commit tier scan
- `--prune` → drop resolved fingerprints (non-destructive; just removes old entries)

---

### `slopgate install-hooks --config <path>`
Install or upgrade the git pre-commit hook.

**Flags:**
- `--config <path>` (required)

**Behavior:**
- No hook exists → create new hook with slopgate check
- Hook exists with slopgate marker → upgrade (idempotent)
- Foreign hook exists → append slopgate block before final `exec` (preserves other hooks)

**Hook location:** `<git-dir>/hooks/pre-commit` (or respects `git config core.hooksPath`)

---


### `slopgate --self-test --config <path>`
Internal: validate regex + AST engines and checker parsers against fixtures.

**Flags:**
- `--config <path>` (required) — typically `rules/baseline/selftest.config.mjs`

Runs in-process tests; exit 0 = all pass, exit 1 = failure. Used by `npm run self-test`.

---

## How the Two Tiers Work

### Fast Tier (Post-Edit)
Runs on every Edit/Write to a `.ts`, `.tsx`, or `.astro` file.

**Scope:** Single file
**Engines:** Regex patterns + AST rules (baseline packs only)
**Baseline:** Not consulted (all violations shown)
**Latency:** < 1 second
**Feedback:** Instant, in-editor

**Rules applied:**
- All regex patterns in enabled baseline packs (`no-stubs`, `ts-suppress`, `as-any`, etc.)
- All AST rules from enabled baseline packs
- Project-owned AST rules (from `astRules` config)

---

### Commit Tier (Pre-Commit)
Runs before `git commit` or when `--staged` is called manually.

**Scope:** All staged files + full repo (for checkers like tsc, knip that need graph context)
**Engines:** Regex patterns + AST rules + six heavy checkers
**Baseline:** Consulted; only NEW violations block commit
**Latency:** 5–30 seconds (tsc + knip dominate)
**Feedback:** Commit blocked or passes

**Rules applied:**
- All regex patterns (same as fast tier)
- All AST rules (same as fast tier)
- **tsc** — TypeScript type errors (full-project compile)
- **knip** — unused exports/files/dependencies
- **jscpd** — copy-paste clones (staged files only are reported)
- **dependency-cruiser** — architecture violations
- **type-coverage** — NEW uncovered expressions
- **diff-shape** — staged files spanning > N top-level dirs

**Filtering:**
1. Run all sources (regex, ast, checkers)
2. Fingerprint violations (sha256 of source, rule, file, normalized message, line text)
3. Filter by ratchet baseline (drop fingerprints in baseline.json)
4. Filter by suppressions (line-level, per file + lineHash)
5. Filter by severity gate (only show `critical`/`high` by default, configurable)
6. Print report; exit 1 if violations remain

---

## Ratchet Baseline

The ratchet prevents violations from blocking adoption of new rules or onboarding legacy repos.

### How It Works

1. **At init:** `slopgate baseline --config ...` creates `.slopgate/baseline.json` with a snapshot of ALL current violations.

2. **On commit:** The gate compares the current full-repo commit-tier scan against the baseline. Violations whose fingerprint is in the baseline are ignored (baselined); NEW violations block the commit.

3. **Paydown:** As issues are fixed, their fingerprint disappears from the current scan. `slopgate baseline --prune` removes old entries from the baseline, lowering the bar.

4. **Re-snapshot:** `slopgate baseline --update` does a full re-scan and updates the baseline (use after intentionally widening rules or adding new checkers).

### Fingerprint Stability

Fingerprints include:
- Rule ID
- File path (repo-relative)
- Normalized message (digit runs replaced with `#`, kills line/col churn)
- First 60 chars of the source line (trimmed)

Fingerprints do NOT include the line number, so they survive unrelated edits shifting lines.

### Suppressions vs. Baseline

- **Baseline** — temporary allowlist; debt should be paid down over time. Track in version control. Entire project-wide snapshot.
- **Suppressions** — permanent per-file exemptions (e.g., "this pattern is correct in this context"). Sparse, line-level. Also tracked.

---

## Config Reference (`.slopgate/config.mjs`)

```javascript
export default {
  // Repository layout
  roots: ['src'],                          // source roots to scan
  exts: ['.ts', '.tsx', '.astro'],        // file extensions
  skipDirs: ['node_modules', 'dist'],      // dirs to skip

  // Rule packs
  baseline: ['no-stubs', 'ts-suppress', 'as-any'],  // baseline packs to enable (opt-in)
  rules: [],                                // project-owned rule .mjs files
  astRules: './rules/ast',                 // dir of .yml AST rules (optional)
  astDisable: [],                          // rule ids to disable (escape hatch)

  // Commit-tier checkers (detected at init; false/absent = off)
  checkers: {
    tsc:           true,                   // or { tsconfig: 'tsconfig.json', timeout: 120 }
    knip:          true,
    jscpd:         { minTokens: 50 },      // token threshold (optional)
    depcruise:     true,                   // uses .slopgate/depcruise.cjs
    typeCoverage:  true,
    diffShape:     { maxDirs: 5 },         // max root dirs per commit (optional)
    // false or absent = disabled
  },

  // Severity filtering (which violations show in reports)
  gate: {
    file: ['critical', 'high'],   // fast-tier report threshold
    staged: ['critical', 'high'], // commit-tier report threshold
  },

  // File paths (relative to repo root)
  suppressions: './suppressions.json',     // line-level exemptions
  fixtures: './fixtures',                  // test fixture canaries
  // baselinePath is auto-computed: .slopgate/baseline.json
};
```

**Auto-generated during `init`:**
- `roots` — detected from workspace packages and src/ dirs
- `exts` — detected from file walk
- `skipDirs` — detected from common exclusions (node_modules, dist, tests, .worktrees)
- `checkers` — detected from installed binaries and config files (all true initially)

---

## Rule Packs

### Baseline Packs (Shipped)

All are opt-in via the `baseline` array in config.

| Pack | Rules | Description |
|------|-------|-------------|
| `no-stubs` | placeholder, TODO markers, "not implemented" | Forbids stub/deferred-work comments |
| `ts-suppress` | @ts-ignore, @ts-expect-error | TypeScript suppression directives |
| `as-any` | `as any` casts | Unsafe type escapes |
| `raw-hex` | hardcoded #RGB hex colors | Use design tokens instead |
| `kv-ban` | Cloudflare KV usage | KV is eventually-consistent; use Durable Objects |

**Planned (v2+):**
- Semantic rules — empty-catch, swallowed-error, console-debug-left
- Depth rules — pass-through-fn, delegating-wrapper (Ousterhout symptoms)
- Test-slop rules — test-no-assertion, test-skip-only
- Extended security rules — eval, new Function

### Project-Owned Rules

Add custom rules by:

1. **Regex rule** — `.mjs` file exporting a `Pattern[]`:
   ```javascript
   export default [{
     id: 'my-rule',
     title: 'Description',
     category: 'category',
     severity: 'critical' | 'high' | 'low',
     pattern: 'regex',
     flags: 'i',
     description: '...',
     resolution: '...',
     canary: '...',
   }];
   ```

2. **AST rule** — `.yml` file (ast-grep syntax):
   ```yaml
   id: my-ast-rule
   pattern: |
     kind: function_declaration
     # ...
   message: Rule violation
   severity: high
   ```

Add to config:
```javascript
rules: ['./rules/my-regex-rule.mjs'],
astRules: './rules/ast',  // auto-loads all .yml files
```

---

## How Rules Are Authored

### Regex Rules

Patterns are JavaScript regex strings with flags (i, m, s, etc.). A pattern matches any line containing the regex.

**Example:**
```javascript
{
  id: 'no-stubs-placeholder',
  pattern: 'placeholder\\s+(?:for now|impl)',
  flags: 'i',
  canary: '// placeholder for now',
  negativeCanary: ['placeholder={t(\'x\')}'],  // should NOT match
}
```

**Advanced:**
- `minFiles: N` — pattern must match in ≥ N files to fire (catch widespread slop)
- `excludeGlobs: ['*.test.ts']` — skip matching in these paths
- `includeGlobs: ['src/**']` — only match in these paths
- Suppressions: per-file, per-line (lineHash = sha256 of line text)

### AST Rules

Written in ast-grep YAML syntax; scoped to source roots + extensions from config.

**Example:**
```yaml
id: empty-catch
pattern: |
  kind: catch_clause
  children:
    - kind: block
      children: []  # empty
message: Empty catch discards errors
severity: critical
```

**Fixtures:**
```yaml
id: empty-catch
rules:
  - id: empty-catch
    message: Empty catch discards errors
fixtures:
  - 'try { x(); } catch (e) {}'  # should match
```

Place fixtures in `rules/baseline/fixtures/` with `.case` and `.output` JSON files.

---

## Hooks Integration

### Claude Code Hooks

Init wires slopgate into `.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [{
      "matcher": "Bash",
      "hooks": [
        {
          "type": "command",
          "command": "/path/to/slopgate/hooks/commit-hook.sh"
        }
      ]
    }],
    "PostToolUse": [{
      "matcher": "Edit|Write",
      "hooks": [
        {
          "type": "command",
          "command": "/path/to/slopgate/hooks/edit-hook.sh"
        }
      ]
    }]
  }
}
```

- **PreToolUse** (commit-hook.sh) — fires before Bash tool use; checks for `git commit` in the command and runs `slopgate --staged`
- **PostToolUse** (edit-hook.sh) — fires after Edit/Write; runs `slopgate --file` on the touched file (fast tier, 5-second timeout)

### Git Pre-Commit Hook

`init` also installs `.git/hooks/pre-commit` (or appends to existing). This is the native git hook; it catches commits from any tool (terminal, IDE, other agents).

```bash
#!/usr/bin/env bash
ROOT=$(git rev-parse --show-toplevel) || exit 0
CONFIG="$ROOT/.slopgate/config.mjs"
[ -f "$CONFIG" ] || exit 0
exec node /path/to/slopgate/bin/slopgate --staged --config "$CONFIG"
```

The hook can be bypassed with `git commit --no-verify`, which is intentional (user-initiated escape hatch).

---

## Suppressions

Edit `.slopgate/suppressions.json`:

```json
{
  "version": 1,
  "entries": [
    {
      "ruleId": "no-stubs-placeholder",
      "file": "src/app.ts",
      "lineHash": "abc123def..."
    }
  ]
}
```

Line hash is auto-generated: `sha256(trimmedLine).slice(0, 16)`.

To suppress a violation, grab the line hash from the report and add an entry. The line text must match exactly (trimmed); unrelated edits shift line numbers but keep line text stable.

---

## Testing

### Run Self-Test

```bash
npm run self-test
```

Validates:
- Regex engine (patterns match canaries, skip negativeCanaries)
- AST engine (ast-grep rules parse + match fixtures)
- Checker parsers (tsc, knip, jscpd, depcruise, type-coverage outputs parse correctly)
- Ratchet fingerprints (stability under line shifts)
- Suppressions (line hashing, deduplication)

---

## Examples

### Example 1: Block Unsafe Type Casts

Config:
```javascript
baseline: ['as-any'],
gate: { staged: ['critical', 'high'] },
```

Commit a file with `const x = y as any;`:
```
slopgate: 1 violation(s)

ast › as-any-cast
  src/utils.ts:42
  Unsafe `as any` cast
  severity: high
  resolution: Use a precise type or a discriminated narrowing.

exit code: 1 (commit blocked)
```

Fix it to `const x = y as unknown;` or a proper type, then commit.

### Example 2: Allow Pre-Existing Copy-Paste, Block New Ones

Config:
```javascript
checkers: { jscpd: { minTokens: 50 } },
baseline: [],
```

Run `slopgate baseline --config .slopgate/config.mjs` to baseline existing clones. Now:
- Commits pass unless they introduce NEW duplications
- Track paydown via `slopgate baseline --prune` (drops resolved entries)

### Example 3: Custom Architecture Rules

Create `.slopgate/depcruise.cjs`:
```javascript
module.exports = {
  forbidden: [
    {
      name: 'no-ui-to-db',
      severity: 'error',
      from: { path: 'src/ui' },
      to: { path: 'src/db' },
    },
  ],
};
```

Now commits that import database code from UI layer are blocked.

### Example 4: Silence a Rule in One Project

Config:
```javascript
astDisable: ['console-debug-left'],  // CLI tools log on purpose
baseline: ['semantic'],
```

The `console-debug-left` rule fires everywhere except this project.

---

## Architecture

### Data Flow (Commit Tier)

```
git commit
  └─ .git/hooks/pre-commit
       └─ slopgate --staged --config <repo>/.slopgate/config.mjs
            ├─ Enumerate staged files
            ├─ Regex engine (patterns → violations)
            ├─ AST engine (ast-grep rules → violations)
            ├─ Checker adapters
            │  ├─ tsc (type errors)
            │  ├─ knip (dead code)
            │  ├─ jscpd (duplication)
            │  ├─ dependency-cruiser (architecture)
            │  ├─ type-coverage (any propagation)
            │  └─ diff-shape (mixed concerns)
            ├─ Ratchet baseline filter (drop pre-existing)
            ├─ Suppressions filter (per-file, per-line)
            ├─ Severity gate (critical/high)
            └─ Report + exit code (0 = pass, 1 = blocked)
```

### Checker Timeout and Errors

Each checker has a per-tool timeout (configurable):
- tsc: 120s
- knip: 90s
- jscpd: 60s
- depcruise: 60s
- type-coverage: 120s

Tool crash / timeout → `⚠ skipped: <id> (<reason>)` warning, gate continues (fail-open on infra). Violations still block; missing tools don't.

---

## Limitations & Future Work

- **Single-machine tool** — engine path is embedded absolute in hooks (assumed stable location)
- **Git-only** — no other VCS support
- **No auto-fix** — violations are reported, not automatically corrected
- **No CI integration yet** — hooks are local and Claude Code only; CI layer is future work
- **`slopgate audit` command** — planned for v2 (non-gating architecture-health report: hotspots, module shape, co-change coupling, ratchet progress tracking)
- **Embeddings-based semantic duplicate detection** — planned, not in v1
- **API-surface diff gate** — track breaking changes to public exports (future)
- **LLM-judge skill** — on-demand deep review of architectural debt (separate sub-project)
- **Rule harvesting** — auto-generate rules from repeated violations (separate sub-project)

---

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md).

---

## License

MIT — See [LICENSE](./LICENSE) for details.
