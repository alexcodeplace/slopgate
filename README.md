# slopgate

A global code-quality / anti-slop gate for hook-capable agent CLIs (Claude Code, Codex, Grok, Gemini) and git. Engine is shared, rules are per-project.

**What it does:** Catches code quality violations in two tiers — a fast post-edit scan (regex + AST rules, instant feedback) and a heavy commit-tier scan (static type checkers, dead-code analysis, architecture rules, copy-paste detection). A **ratchet baseline** lets legacy repos adopt without flooding — only NEW violations block commits; pre-existing ones are baselined and tracked for paydown.

---

## Features

- **Two-tier gate**
  - **Fast tier** (post-edit hook): regex patterns + AST rules, instant feedback as you code
  - **Commit tier** (pre-commit hook): includes heavy checkers (tsc, knip, jscpd, dependency-cruiser, leakscan, type-coverage, diff-shape) + AST + regex, blocks commits
  
- **Ratchet baseline** — snapshot violations at adoption time; only NEW violations fail the gate. Track debt paydown over time.

- **Seven commit-tier checkers**
  - **tsc** — TypeScript type errors (full-project scope)
  - **knip** — dead/unused code (exports, files, dependencies)
  - **jscpd** — copy-paste duplication (token-level)
  - **dependency-cruiser** — architecture rules (cycles, orphans, layer boundaries)
  - **leakscan** — presentation-layer files importing/calling raw data or transport APIs
  - **type-coverage** — propagation of `any` type (per-expression tracking)
  - **diff-shape** — wide commits spanning too many directories (encourages focused changes)

- **Shared regex + AST rule packs** — fast-tier and commit-tier both run these
  - Convention: `no-stubs`, `ts-suppress`, `as-any`, `raw-hex` (design tokens), `sql-safety`
  - Security: `live-secrets`, `eval-ban`, `pii-logs`, `weak-hash`
  - Cloudflare boundary: `kv-ban` (plus the opt-in `stack = ["cloudflare"]` pack)
  - Built-in AST rules: empty-catch, unsafe `innerHTML`/`dangerouslySetInnerHTML`, `target="_blank"` without `rel`, `window` access during render
  
- **Native git pre-commit hook** — no daemon, no CI coupling, just git
  
- **Agent hook integration** — installs PreToolUse/PostToolUse/SessionStart hooks for Claude Code, Codex, Grok, and Gemini settings files when detected or requested
  
- **Suppressions** — per-file, per-line, with line-hash stability across edits
  
- **Self-test** — `slopgate --self-test` validates rule engines + baseline checker parsers against bundled fixtures

---

## Install

```bash
npm install -g slopgate
```

The matching prebuilt native engine for your platform (linux / macOS / Windows × x64 / arm64) is pulled in automatically as an optional dependency — no toolchain or build step required.

Then onboard a project:

```bash
slopgate init [path-to-repo]
```

This:
1. Detects source roots, file extensions, and package layout
2. Scaffolds `.slopgate/config.toml` with detected checkers enabled
3. Writes `.slopgate/suppressions.json` and `.slopgate/depcruise.cjs` (starter)
4. Creates `.slopgate/convention-sources.json` (hints for authoring project rules from local skills/agents/docs)
5. Creates `.slopgate/rules/ast/` and `.slopgate/fixtures/src/` directories
6. Installs git pre-commit hook (or appends to existing)
7. Merges repo-local Claude Code hook settings into `.claude/settings.json` and installs detected user-level agent hook files for Claude/Codex/Grok/Gemini
8. Prints next steps (including: run `slopgate baseline --config .slopgate/config.toml`)

---

## Quickstart

### Run the gate on staged changes (pre-commit):
```bash
slopgate --staged --config .slopgate/config.toml
```

### Run on a single file (post-edit, fast tier):
```bash
slopgate --file src/app.ts --config .slopgate/config.toml
```

### Create/update the baseline:
```bash
# Create baseline (refuses if it exists)
slopgate baseline --config .slopgate/config.toml

# Update baseline (re-snapshot all current violations)
slopgate baseline --update --config .slopgate/config.toml

# Prune baseline (remove entries no longer occurring)
slopgate baseline --prune --config .slopgate/config.toml
```

### Run self-test (validate the engine against bundled fixtures):
```bash
slopgate --self-test --config "$(npm root -g)/slopgate/rules/baseline/selftest.config.toml"
```

### Install or reinstall hooks:
```bash
slopgate install-hooks --config .slopgate/config.toml
```

---

## Command Reference

### `slopgate init [dir]`
Onboard a new repository. Detects roots, extensions, installed checkers, and scaffolds project structure.

**Args:**
- `dir` (optional) — target directory; defaults to `process.cwd()`
- No `--config` required; generates config during init

**Creates:**
- `.slopgate/config.toml` — project config (roots, extensions, rule packs, checkers, baseline/suppressions paths)
- `.slopgate/suppressions.json` — line-level violation suppressions (empty initially)
- `.slopgate/depcruise.cjs` — starter dependency-cruiser rules (if depcruise detected)
- `.slopgate/convention-sources.json` — hints for authoring project-specific rule packs
- `.slopgate/rules/ast/` and `.slopgate/fixtures/src/` — directories for custom rules and fixtures
- `.git/hooks/pre-commit` — native git pre-commit hook (creates new or appends to existing)
- `.claude/settings.json` — repo-local Claude Code hook entries (idempotent merge)

When an agent CLI is detected, `init` also updates that user's hook settings file through the same installer used by `slopgate agent-hooks`.

**Next step:** Run `slopgate baseline --config .slopgate/config.toml` to create the initial ratchet baseline

---

### `slopgate --staged --config <path>`
Run commit-tier gate on staged files. Used by the git pre-commit hook and agent PreToolUse commit hook.

**Flags:**
- `--config <path>` (required) — path to `.slopgate/config.toml`
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
Run fast-tier gate on a single file (post-edit). Used by the agent PostToolUse edit hook.

**Flags:**
- `--file <path>` (required) — repo-relative path to check
- `--config <path>` (required) — path to `.slopgate/config.toml`
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
- `--config <path>` (required) — typically `rules/baseline/selftest.config.toml`

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
**Engines:** Regex patterns + AST rules + seven heavy checkers
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

## UX Module (optional)

The UX module provides opinionated static analysis rules for common UX anti-patterns. It is **off by default** since UX preferences vary across teams and projects. Enable selectively via the `ux:{}` config namespace.

**Why optional?** Many teams have different UX preferences, and enabling UX rules on existing projects would flag pre-existing markup. These are good-enough defaults for NEW projects where you want opinionated UX guidance but have no specific opinion yourself.

### Configuration

```toml
# .slopgate/config.toml
# ... other config

# UX module (optional) — off by default, opt-in per sub-module
[ux]
a11y = "high"        # Accessibility violations (gate commits)
cls = "high"         # Cumulative Layout Shift violations (gate commits)
feedback = "high"    # Silent async / double-submit (gate commits)
taste = "advisory"   # Design taste violations (report only, don't gate)
advisory = "advisory" # Heuristic nudges (report only, higher false-positive)
# taste = "medium"   # equivalent to 'advisory'
# taste = true       # use sub-module default severity
# omit key = that sub-module OFF
# delete whole [ux] table = entire module OFF
```

### Sub-modules

| Key | Catches | Default Severity | Framework § |
|-----|---------|------------------|-------------|
| `a11y` | onClick on `<div>`/`<span>` without role; `<a onClick>` without href; `<img>` without alt; `<button>` without type; positive `tabIndex` | `high` | §11 |
| `cls` | `<img>`/`<video>`/`<iframe>` without width/height | `high` | §13 |
| `feedback` | async `onClick` on a `<button>` with no `disabled` state (double-submit, silent wait) | `high` | §3/§12 |
| `taste` | emoji in UI, "trusted by" clichés, Lorem ipsum, robotic microcopy, heavy drop shadows, linear/long (>300ms) motion | `medium` | §0/§6/§26 |
| `advisory` | modal without `onClose`; array index as React `key`; view state (tab/page/filter) in `useState` instead of the URL | `medium` | §10/§14 |

Magic hardcoded colors/spacing (`#hex`, `rgb()`/`hsl()`, multi-digit `px`) are caught by the baseline `raw-hex` pack (§15), independent of the UX module.

### Severity Levels

- **`'critical'`/`'high'`**: Gates commits (blocks by default, since default gate is `['critical','high']`)
- **`'medium'`/`'advisory'`**: Reports but doesn't block commits (useful for gradual adoption)
- **`true`**: Use the sub-module's default severity
- **Omit key**: That sub-module is OFF
- **Delete `ux:{}` block**: Entire UX module is OFF

### Opt-out

Symmetric and trivial:
- Delete a key to disable one sub-module: `ux: { a11y: 'high' }` (cls and taste OFF)
- Delete the whole `ux:{}` block to disable the entire module

### Companion Skill

Pair the static UX module with the `/slopgate-ux` skill for semantic UX directives that static analysis can't enforce (four-states, button hierarchy, focus-trap, optimistic UI, etc.).

---

## Config Reference (`.slopgate/config.toml`)

```toml
# Repository layout
roots = ["src"]                          # source roots to scan
exts = [".ts", ".tsx", ".astro"]         # file extensions; Rust repos may detect [".rs"]
skipDirs = ["node_modules", "dist"]      # dirs to skip

# Rule packs
baseline = ["no-stubs", "ts-suppress", "as-any"]  # built-in baseline packs to enable (opt-in)
rules = []                               # project regex rule packs — must be [] (PHASE-2, not yet supported)
astRules = "./rules/ast"                 # dir of .yml AST rules (optional)
astDisable = []                          # rule ids to disable (escape hatch)

# Custom file paths (relative to repo root)
suppressions = "./suppressions.json"     # line-level exemptions
fixtures = "./fixtures"                  # test fixture canaries
# baselinePath is auto-computed: .slopgate/baseline.json

# Commit-tier checkers (detected at init; absent = off)
# Per-checker options as key = value under each [checkers.<name>] table.
[checkers.tsc]
# e.g. timeout = 60

[checkers.leakscan]
# enabled by init only when a bundled/dev leakscan binary exists and frontend TSX/JSX roots are present

# UX module (optional) — off by default, opt-in per sub-module
[ux]
a11y = "high"        # accessibility violations
cls = "high"         # cumulative layout shift
taste = "advisory"   # design taste (reports, doesn't gate)

# Severity filtering (which violations show in reports)
[gate]
file = ["critical", "high"]    # fast-tier report threshold
staged = ["critical", "high"]  # commit-tier report threshold
```

**Auto-generated during `init`:**
- `roots` — detected from workspace packages and src/ dirs
- `exts` — detected from file walk
- `skipDirs` — detected from common exclusions (node_modules, dist, tests, .worktrees)
- `checkers` — detected from installed binaries, config files, and conservative checker-specific signals

---

## Rule Packs

### Baseline Regex Packs (Shipped)

All are opt-in via the `baseline` array in config. Severity drives the gate threshold (`critical`/`high` block by default).

| Pack | Severity | Category | Catches |
|------|----------|----------|---------|
| `no-stubs` | critical | convention | Stub / placeholder / "not implemented" / deferred-work markers |
| `ts-suppress` | high | convention | `@ts-ignore` / `@ts-expect-error` — suppressing tsc instead of fixing the cause |
| `as-any` | high | convention | `as any`, `: any`, `Array<any>`, `Promise<any>`, `Record<string, any>` escape hatches that disable type safety |
| `raw-hex` | high | convention | Hardcoded hex / `rgb()` colors + raw multi-digit `px` — use design tokens |
| `sql-safety` | critical | convention | `SELECT … FOR UPDATE` with an aggregate (Postgres rejects this at runtime) |
| `kv-ban` | critical | boundary | Cloudflare KV in read-after-write paths (eventually-consistent) |
| `live-secrets` | critical | security | Hardcoded Stripe / webhook / Google live credentials |
| `eval-ban` | critical | security | `eval` / dynamic code execution (injection surface) |
| `pii-logs` | high | security | PII fields written to logs / error trackers |
| `weak-hash` | high | security | MD5 / SHA-1 for integrity checks or passwords (cryptographically broken) |

### Baseline AST Rules (Shipped, Always Active)

Loaded automatically alongside the regex packs (the resolver always adds `rules/baseline/ast`); disable any by id via `astDisable = [...]`.

| Rule id | Catches |
|---------|---------|
| `empty-catch` (ts + tsx) | Empty `catch` block silently swallowing an error |
| `inner-html` | Unsafe `innerHTML` / `dangerouslySetInnerHTML` assignment |
| `focused-test` | Focused tests committed via `test.only` / `it.only` / `describe.only` / `fit` / `fdescribe` |
| `target-blank-norel` | `target="_blank"` anchor missing `rel="noopener"` |
| `window-in-render` | `window`/`document` access during render (SSR hazard) |

### Stack Packs (Shipped)

Opt-in via `stack = ["cloudflare"]`:

| Pack | Rule ids |
|------|----------|
| `cloudflare` | `cf-env-spread-secrets`, `process-env-access`, `waituntil-bare-method-ref`, `cf-getCloudflareContext-banned`, `hono-env-direct-access` |

**Planned (v2+):**
- Depth rules — pass-through-fn, delegating-wrapper (Ousterhout symptoms)
- Test-slop rules — test-no-assertion, test-skip
- Custom project **regex** rule packs (the `rules = [...]` field — see [Project-Owned Rules](#project-owned-rules))

### Project-Owned Rules

Add custom rules as **AST rules** — `.yml` files in ast-grep syntax:

```yaml
id: my-ast-rule
language: tsx
severity: error          # ast-grep level (error|warning|info)
message: Rule violation
note: '{"severity":"high","category":"convention","resolution":"…"}'  # slopgate metadata
rule:
  pattern: 'someBadCall($$$ARGS)'   # code-snippet matcher; or structural kind/has/inside/all/any/not
```

Point `astRules` at the directory holding them:

```toml
astRules = "./rules/ast"  # auto-loads all .yml files in this dir
```

> **Note:** Custom **project regex rule packs** (the `rules = [...]` field) are **not yet supported** by the native engine. `rules` must currently be `[]`; a non-empty value errors with `slopgate: project rule pack "<path>" cannot be loaded by the native TOML resolver (PHASE-2: project rule packs)`. Project regex packs are planned (PHASE-2). For now, use ast-grep YAML for custom rules, or one of the built-in baseline/stack packs.

---

## How Rules Are Authored

### Regex Rules

> **Note:** Authoring *custom project* regex rule packs is **not yet supported** by the native engine (PHASE-2 — see [Project-Owned Rules](#project-owned-rules)). The shape below describes how the **built-in** regex packs are defined (compiled into the engine); it is reference, not a workflow you can wire in via `rules` today. Use ast-grep YAML for custom rules.

Patterns are regex strings with flags (i, m, s, etc.). A pattern matches any line containing the regex.

**Example:**
```
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

**Example** (modeled on the shipped `rules/baseline/ast/empty-catch-block-tsx.yml`):
```yaml
id: empty-catch
language: tsx
severity: error          # ast-grep level (error|warning|info)
message: Empty catch block swallows error silently
note: '{"severity":"high","category":"convention","resolution":"Handle or rethrow; log with context."}'
rule:
  pattern: 'try { $A } catch ($E) {}'   # code-snippet matcher; or structural kind/has/inside/all/any/not
```

The top-level `severity` is ast-grep's own level; slopgate's gating severity/category/resolution live in the
JSON `note` field.

**Fixtures:** add a source canary that *triggers* the rule to `.slopgate/fixtures/src/` (built-in rules use
`rules/baseline/fixtures/src/`). A `.ts`/`.tsx` file containing the violating code is enough:

```tsx
// .slopgate/fixtures/src/empty-catch.tsx
export function f() { try { risky(); } catch (e) {} }  // should fire empty-catch
```

`slopgate --self-test --config .slopgate/config.toml` scans the fixtures and asserts every rule fires at
least once.

---

## Hooks Integration

### Agent Hooks

`init` wires repo-local Claude Code hooks into `.claude/settings.json`. The same hook set is managed for user-level Claude, Codex, Grok, and Gemini settings with:

```bash
slopgate agent-hooks status
slopgate agent-hooks install --agent claude,codex,grok,gemini
slopgate agent-hooks remove --agent codex
```

The hook JSON uses this shape:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "/path/to/slopgate/hooks/commit-hook.sh"
          }
        ]
      },
      {
        "matcher": "Bash|Edit|Write",
        "hooks": [
          {
            "type": "command",
            "command": "/path/to/slopgate/hooks/baseline-guard.sh"
          }
        ]
      }
    ],
    "PostToolUse": [{
      "matcher": "Edit|Write",
      "hooks": [
        {
          "type": "command",
          "command": "/path/to/slopgate/hooks/edit-hook.sh"
        }
      ]
    }],
    "SessionStart": [{
      "hooks": [
        {
          "type": "command",
          "command": "/path/to/slopgate/hooks/session-start.sh"
        }
      ]
    }]
  }
}
```

- **PreToolUse** (commit-hook.sh) — fires before Bash tool use; checks for `git commit` in the command and runs `slopgate --staged`
- **PreToolUse** (baseline-guard.sh) — blocks direct agent edits/removals of `.slopgate/baseline.json` and `.slopgate/suppressions.json`, plus agent-run `slopgate baseline`
- **PostToolUse** (edit-hook.sh) — fires after Edit/Write; runs `slopgate --file` on the touched file (fast tier, 5-second timeout)
- **SessionStart** (session-start.sh) — records the active model/session for stats attribution

### Git Pre-Commit Hook

`init` also installs `.git/hooks/pre-commit` (or appends to existing). This is the native git hook; it catches commits from any tool (terminal, IDE, other agents).

```bash
#!/usr/bin/env bash
ROOT=$(git rev-parse --show-toplevel) || exit 0
CONFIG="$ROOT/.slopgate/config.toml"
[ -f "$CONFIG" ] || exit 0
exec slopgate --staged --config "$CONFIG"
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

### CI and Package Smoke Checks

CI keeps the Rust workspace and npm package path honest with:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --locked -p slopgate-rs
SLOPGATE_BIN=target/debug/slopgate-rs npm run self-test
npm pack --dry-run --json
```

GitHub Actions workflows are linted with `actionlint`. `zizmor` runs as a
non-blocking advisory security check for workflow hardening.

`cargo-deny` / `cargo audit` are deferred until the project has an agreed
advisory/license policy and a validated low-noise config.

---

## Examples

### Example 1: Block Unsafe Type Casts

Config:
```toml
baseline = ["as-any"]
[gate]
staged = ["critical", "high"]
```

Commit a file with `const x = y as any;`:
```
slopgate: 1 violation(s)

regex › as-any-cast
  src/utils.ts:42
  Unsafe `as any` cast
  severity: high
  resolution: Use a precise type or a discriminated narrowing.

exit code: 1 (commit blocked)
```

Fix it to `const x = y as unknown;` or a proper type, then commit.

### Example 2: Allow Pre-Existing Copy-Paste, Block New Ones

Config:
```toml
baseline = []
[checkers.jscpd]
minTokens = 50
```

Run `slopgate baseline --config .slopgate/config.toml` to baseline existing clones. Now:
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

### Example 4: Silence a Built-in Rule in One Project

Config:
```toml
baseline = ["no-stubs", "as-any"]
astDisable = ["target-blank-norel"]  # this app links only to vetted internal routes
```

`astDisable` lists built-in AST rule ids to turn off for this repo; every other rule stays active.

---

## Architecture

### Data Flow (Commit Tier)

```
git commit
  └─ .git/hooks/pre-commit
       └─ slopgate --staged --config <repo>/.slopgate/config.toml
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
- leakscan: 60s
- type-coverage: 120s

Tool crash / timeout → `⚠ skipped: <id> (<reason>)` warning, gate continues (fail-open on infra). Violations still block; missing tools don't.

---

## Limitations & Future Work

- **Git-only** — no other VCS support
- **No auto-fix** — violations are reported, not automatically corrected
- **Local gate integration only** — hooks are local and Claude Code/git based; CI covers project/package quality but does not replace per-repo slopgate adoption
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
