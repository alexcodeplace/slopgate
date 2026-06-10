# Slopgate v2 — Two-Tier Gate, Checker Adapters, Ratchet Baseline, Native Pre-Commit

Date: 2026-06-10
Status: Draft → for plan
Scope: sub-project 1 of 3. Sub-project 2 (`/slopgate` LLM-judge review skill, on-demand only) and sub-project 3 (rule harvesting from past mistakes) get separate specs after this ships.

## 1. Problem

Slopgate v1 catches only what regex + ast-grep can express, and only at two Claude Code hook points (post-edit, pre-commit-command). Gaps:

1. **Detection** — type errors hidden behind the bans it enforces (`as any` is banned, but a structurally wrong type passes), dead/orphaned code, reimplemented-instead-of-imported duplicates, swallowed errors.
2. **Enforcement** — commits from a terminal, another agent, or an IDE bypass the gate entirely (Claude PreToolUse hook only sees Claude's own Bash calls).
3. **Adoption** — heavy checks on a legacy repo flood with pre-existing violations; v1's only answer is "drive to zero first", which blocks adoption.

## 2. Decisions (user-confirmed)

- Two tiers: **fast** (post-edit, regex + ast only, unchanged) and **commit** (staged: regex + ast + heavy checkers).
- Heavy checkers: **tsc --noEmit**, **knip** (dead/unused), **jscpd** (copy-paste), **dependency-cruiser** (architecture: layers, cycles, orphans), **type-coverage** (any-propagation), **diff-shape** (mixed-concern commits), plus new **semantic / test-slop / comment-slop ast+regex packs** (cheap, run in both tiers).
- New enforcement point: **native git pre-commit hook** installed by init. No pre-push, no CI (future option).
- Pre-existing violations: **ratchet baseline** — snapshot at init, only NEW violations fail, counts only go down.
- **`slopgate audit`** — non-gating periodic architecture-health report (churn×size hotspots, fan-out, barrel files, ratchet progress).
- LLM-judge review = separate on-demand skill, never hooked (sub-project 2).

## 3. Architecture

### 3.1 Data flow (commit tier)

```
git commit
  └─ .git/hooks/pre-commit            (native, installed by init)
       └─ slopgate --staged --config <repo>/.slopgate/config.mjs
            ├─ resolveConfig            (+ checkers section, tier resolution)
            ├─ enumerate staged files
            ├─ regex engine             (existing)
            ├─ ast-grep engine          (existing + semantic pack)
            ├─ checker adapters         (tsc, knip, jscpd, depcruise, type-coverage, diff-shape — commit tier only)
            ├─ fingerprint every violation
            ├─ ratchet filter           (drop fingerprints present in baseline.json)
            ├─ suppressions filter      (existing, applies to all sources)
            ├─ severity gate            (existing config.gate.staged)
            └─ report + exit code       (1 = commit blocked)
```

Claude Code commit-hook (`hooks/commit-hook.sh`) keeps calling `--staged` and therefore gets the commit tier too — both chokepoints run identical checks. Post-edit hook (`--file`) stays fast tier.

Tier selection: `--file` ⇒ fast; `--staged` ⇒ commit; `--tier fast|commit` overrides (lets a user run a fast staged scan).

### 3.2 Checker adapter interface

New `src/checkers/` directory. Each checker is one module default-exporting:

```js
export default {
  id: 'tsc',                       // stable, used in fingerprints + report
  // Is the tool usable in this repo? Never throws.
  detect(config) {/* -> { available: boolean, reason?: string } */},
  // Run and parse. Never throws; tool crash/timeout -> errors[], violations untouched.
  run(config, { files }) {/* -> { violations: Violation[], errors: string[] } */},
};
```

`src/checkers/index.mjs` exports the static array `[tsc, knip, jscpd, depcruise, typeCoverage, diffShape]` — the registry.

Checker `Violation` matches the existing shape used by regex/ast paths:
`{ ruleId, title, severity, category, file (repo-relative), line, fullLine, excerpt, resolution }` plus `source: 'checker:<id>'`. Suppressions (`file + lineHash(fullLine)`) work unchanged for line-bearing violations; file-level violations (knip unused-file) use `fullLine: ''`.

Tool resolution: `<repoRoot>/node_modules/.bin/<tool>` only — no global/npx fallback (deterministic versions). Missing binary ⇒ `detect()` returns `available: false`; the gate prints one `⚠ skipped: <id> (<reason>)` line and continues. A skipped checker never fails the gate.

Per-checker timeout (config-overridable defaults: tsc 120s, knip 90s, jscpd 60s, depcruise 60s, type-coverage 120s, diff-shape n/a). Timeout/crash ⇒ stderr warning, gate continues (fail-open on infra, same philosophy as edit-hook), exit code unaffected by infra errors.

### 3.3 Individual checkers

**tsc** (`src/checkers/tsc.mjs`)
- `detect`: tsconfig present (config `checkers.tsc.tsconfig` override, default `<repoRoot>/tsconfig.json`) AND local tsc binary exists.
- `run`: `tsc --noEmit -p <tsconfig> --pretty false`. Always full-project — type errors are not file-scoped; a staged change can break a non-staged file, and that MUST fail. Pre-existing errors are absorbed by the baseline, not by scoping.
- Parse lines `relPath(line,col): error TSnnnn: message`. Multi-line continuation lines are appended to the previous error's message. `ruleId: 'tsc-TSnnnn'`, severity `high`, category `types`.

**knip** (`src/checkers/knip.mjs`)
- `detect`: local knip binary + knip config (any of `knip.json`, `knip.jsonc`, `"knip"` key in package.json). No config ⇒ skipped (knip without config produces noise).
- `run`: `knip --reporter json --no-exit-code`. Parse `issues` → violations per unused export / unused file / unused dependency. `ruleId: 'knip-<issueType>'`, severity `high`, category `dead-code`. Unused-file violations are file-level (`line: 1`, `fullLine: ''`).
- Always full-repo (dead code is a whole-graph property); baseline absorbs pre-existing.

**jscpd** (`src/checkers/jscpd.mjs`)
- `detect`: local jscpd binary.
- `run`: `jscpd <roots> --min-tokens <N> --reporters json --output <tmpdir> --silent` (N default 50, config `checkers.jscpd.minTokens`). Read JSON from tmpdir, delete tmpdir after.
- A clone produces ONE violation only if at least one of its two sides overlaps a staged file (commit tier passes the staged list); report points at the staged side and names the other side in the excerpt ("duplicates src/x.ts:10-42"). Severity `high`, category `duplication`, `ruleId: 'jscpd-clone'`.

**dependency-cruiser** (`src/checkers/depcruise.mjs`)
- `detect`: local depcruise binary + rules file (`.slopgate/depcruise.cjs`, falling back to project `.dependency-cruiser.{js,cjs,json}`).
- `run`: `depcruise --config <rules> --output-type json <rootsRel>` from repoRoot. One violation per rule transgression: `ruleId: 'depcruise-<ruleName>'`, category `architecture`, severity from rule severity (`error`→critical, `warn`→high, `info`→dropped). File = `from` module, `line: 1`, excerpt names the offending edge (`src/ui/x.ts → src/db/y.ts violates no-ui-to-db`).
- Init scaffolds a starter `.slopgate/depcruise.cjs` with universal rules (`no-circular`, `no-orphans`); layer-boundary rules are per-project and authored during `slopgate-init` convention mining.
- This is the primary "catch bad architecture programmatically" surface: intended layering is encoded as rules, violations gate commits.

**type-coverage** (`src/checkers/type-coverage.mjs`)
- `detect`: local type-coverage binary + tsconfig.
- `run`: `type-coverage --detail` (project tsconfig). Each uncovered expression → violation `ruleId: 'type-coverage-uncovered'`, severity `high`, category `types`, with file/line/identifier text as `fullLine`.
- No separate watermark mechanism: the standard ratchet baseline absorbs all pre-existing uncovered expressions; any NEW `any`-typed expression fails. Coverage can only rise.

**diff-shape** (`src/checkers/diff-shape.mjs`)
- No external tool (`detect` always available; commit tier only — needs a staged set).
- Staged files spanning > N distinct top-level dirs under the configured roots (default 5, `checkers.diffShape.maxDirs`) → ONE violation, `ruleId: 'diff-shape-mixed-concerns'`, severity `high`, category `hygiene`, resolution "split into focused commits". Suppressible like any violation for legit wide refactors.

### 3.4 Ratchet baseline (`src/ratchet.mjs`)

Fingerprint (internal to this module): `sha256(source | ruleId | relPath | normalizedMessage | trimmedFullLine)` truncated to 16 hex chars.
- `normalizedMessage` = violation title/message with all digit runs replaced by `#` (kills line/col churn inside tsc messages).
- No line number in the fingerprint ⇒ survives unrelated edits shifting lines. Trimmed source-line text disambiguates repeated identical violations in one file; N identical (same file, same rule, same line text) violations collapse to one fingerprint — acceptable: ratchet stays sound for "did something NEW appear".

`baseline.json` (lives in `.slopgate/`, committed):

```json
{ "version": 1, "generated": "2026-06-10T...", "entries": { "<fp>": { "ruleId": "tsc-TS2322", "file": "src/x.ts" } } }
```

API:
- `loadBaseline(path)` → `{ entries, error? }` (malformed ⇒ warn + treat as empty, same posture as suppressions).
- `filterNew(violations, baseline)` → `{ fresh, baselinedCount }`.
- `writeBaseline(path, violations)` — full snapshot.

Gate behavior (commit tier only; fast tier never consults baseline — post-edit feedback should show everything in the touched file):
- Baseline missing ⇒ empty baseline (everything is new) + one-line hint: `run: slopgate baseline --config …`.
- Report prints `N pre-existing (baselined) ignored` when N > 0.

CLI:
- `slopgate baseline --config <c>` — create if missing; refuse to overwrite without `--update` (prevents accidentally re-baselining fresh slop).
- `slopgate baseline --update --config <c>` — full re-snapshot (runs a full-repo commit-tier scan, all checkers).
- `--prune` (combinable) — drop entries whose fingerprint no longer occurs.

### 3.5 New rule packs (rules/baseline/)

All ast rules ship with canary + negative fixtures in `rules/baseline/fixtures/`; regex rules use the existing canary/negativeCanary mechanism.

**Semantic ast pack** (always shipped):

| rule id | catches | severity |
|---------|---------|----------|
| `empty-catch` | `catch` with empty block | critical |
| `swallowed-error` | `catch (e)` where `e` is never referenced and block doesn't rethrow | high |
| `console-debug-left` | `console.log` / `debugger` statement in source roots | high |

**Depth pack** (always shipped — Ousterhout shallow-module symptoms expressible in ast):

| rule id | catches | severity |
|---------|---------|----------|
| `pass-through-fn` | exported function whose body is a single call forwarding its own params verbatim (`export const f = (a,b) => g(a,b)`) — decorative seam | high |
| `delegating-wrapper` | exported class where every method body is a single delegation to one wrapped field | high |

(Deeper depth analysis — export-ratio, single-consumer modules, co-change coupling — is graph-level, not per-file: lives in `slopgate audit` §3.9. Deletion-test/seam-audit judgment calls live in the LLM-judge skill, sub-project 2.)

**Test-slop pack** (opt-in baseline pack `test-slop`; rules carry `includeGlobs: ['**/*.test.*', '**/tests/**']`):

| rule id | catches | severity | engine |
|---------|---------|----------|--------|
| `test-no-assertion` | `it`/`test` block containing no `expect`/`assert` call | critical | ast |
| `test-skip-only` | `.skip(` / `.only(` / `xit(` / `xdescribe(` | critical | regex |

Engine change required: pattern-level `includeGlobs` support (mirror of existing `excludeGlobs`) for regex rules; the ast rule (`test-no-assertion`) scopes itself via ast-grep's native `files:` field in its yml. The edit-hook stops skipping `*.test.ts(x)` so test files get fast-tier feedback.

**Comment-slop additions** (appended to existing `no-stubs` baseline pack): `in production( you would| code)?`, `for (the sake of )?simplicity`, `simplified (version|implementation)` — AI-deferral markers, severity critical, case-insensitive, with negative canaries.

**Security ast additions** (always shipped): `eval(`/`new Function(` (critical), `dangerouslySetInnerHTML` (high — complements existing `inner-html`).

New config key `astDisable: ['rule-id', ...]` — filters ast violations by id post-scan (escape hatch for projects where e.g. `console-debug-left` is wrong; CLI tools log on purpose).

### 3.6 Native pre-commit installer

New CLI `slopgate install-hooks --config <c>`; `slopgate init` calls it.

- Resolve hooks dir: `git config core.hooksPath` if set, else `<gitdir>/hooks`.
- No existing `pre-commit` ⇒ write ours (marker line `# slopgate-hook v1`), `chmod +x`:

```bash
#!/usr/bin/env bash
# slopgate-hook v1
ROOT=$(git rev-parse --show-toplevel) || exit 0
CONFIG="$ROOT/.slopgate/config.mjs"
[ -f "$CONFIG" ] || exit 0
exec node /home/user/Projects/slopgate/bin/slopgate --staged --config "$CONFIG"
```

- Existing hook containing our marker ⇒ rewrite (idempotent upgrade).
- Existing foreign hook ⇒ append our block (guarded by marker) before any `exec` line; if the foreign hook ends in `exec`, insert before it; otherwise append at end. Never delete foreign content.
- Engine path is embedded absolute (this is a single-machine personal tool; engine repo location is stable).
- `--no-verify` bypass remains possible — documented, acceptable (user-intentional override; CI is the future answer).

### 3.7 Config additions (`.slopgate/config.mjs`)

```js
export default {
  // ...existing keys (baseline, rules, astRules, roots, exts, skipDirs, gate, suppressions, fixtures)
  astDisable: [],
  checkers: {
    tsc:          true,          // or { tsconfig: 'tsconfig.app.json', timeout: 120 }
    knip:         true,
    jscpd:        { minTokens: 50 },
    depcruise:    true,          // rules: .slopgate/depcruise.cjs
    typeCoverage: true,
    diffShape:    { maxDirs: 5 },
    // false / absent => disabled even if detected
  },
};
```

Resolver: absent `checkers` key ⇒ all disabled (explicit opt-in; init scaffolder writes the block with detected tools enabled). `true` normalizes to `{}`. Gate runs only checkers that are both enabled and `detect() === available`.

### 3.8 Report (`src/report.mjs`)

- Group violations by source: `regex`, `ast`, `checker:<id>`.
- Footer lines: skipped checkers (`⚠ skipped: knip (no knip config)`), baselined count, infra errors.

### 3.9 `slopgate audit` command (`src/audit.mjs`)

Non-gating architecture-health report. Always exits 0; meant to run periodically (manually or via wrap-up skill), not on commit. Sections:

1. **Hotspots** — top 10 files by `churn × size`: churn = commit count touching the file in last 90 days (`git log --since`), size proxy = LOC × function count (function count via one ast-grep count query). These files are where architecture is rotting; candidates for the LLM-judge deep review.
2. **Module shape** (from the depcruise JSON graph, reusing the checker's invocation):
   - fan-out top offenders (module importing the most internal modules),
   - single-consumer modules (fan-in == 1, < 60 LOC) — failed-deletion-test candidates: inline into the caller,
   - barrel files (re-export-only modules) — pure indirection inventory.
   - export-ratio per module (exported top-level decls ÷ total top-level decls, via ast-grep counts; ratio ≥ 0.9 over ≥ 5 decls = shallow-module flag, no information hiding).
3. **Co-change coupling** — file pairs committed together ≥ 70% of the time (last 90 days, min 5 shared commits) that live in DIFFERENT top-level dirs: boundary is in the wrong place.
4. **Ratchet progress** — baseline entries at creation vs still-occurring now (reuses `--prune` dry-run logic): "debt burned down X → Y".

Skips any section whose inputs are unavailable (no depcruise ⇒ no module-shape; shallow git history ⇒ shorter windows) with a notice.

## 4. Error handling summary

| failure | behavior |
|---------|----------|
| checker binary missing / no tool config | skipped, one ⚠ line, gate continues |
| checker crash / timeout / unparseable output | ⚠ with first stderr line, gate continues (fail-open infra) |
| baseline.json malformed | warn, treat as empty (everything new) — fail-closed on violations |
| suppressions malformed | existing behavior (warn, empty) |
| violations present | exit 1, commit blocked — fail-closed |
| foreign pre-commit hook | preserved, ours appended with marker |

Infra errors never flip the exit code by themselves; violations always do.

## 5. Testing

- **Self-test extension** (`src/selftest.mjs`): new "parser fixtures" stage — recorded real tool outputs in `rules/baseline/fixtures/checker-outputs/{tsc.txt,knip.json,jscpd.json,depcruise.json,type-coverage.txt}` + expected violation arrays; self-test feeds each through the checker's parser and asserts match. Catches tool-output-format drift without invoking tools. diff-shape (no external tool) gets a plain unit test instead.
- **Canaries**: each new ast rule gets canary + negativeCanary fixture entries (existing mechanism).
- **Ratchet unit tests** (extend `init.test.mjs` pattern, plain node): fingerprint stability under line shifts, `filterNew` semantics, `--update` refusal without flag, malformed baseline posture.
- **Installer tests**: temp git repo — fresh install, idempotent re-install, foreign-hook append, core.hooksPath respect.
- **End-to-end smoke**: temp repo with tsconfig + a type error; baseline it; verify clean commit passes; add a new type error; verify exit 1.

## 6. Files changed

| file | change |
|------|--------|
| `src/checkers/index.mjs` | new — registry array |
| `src/checkers/{tsc,knip,jscpd,depcruise,type-coverage,diff-shape}.mjs` | new — adapters |
| `src/ratchet.mjs` | new — fingerprint + baseline load/filter/write/prune |
| `src/audit.mjs` | new — hotspots, module shape, co-change, ratchet progress |
| `src/gate.mjs` | tier param; run checkers in commit tier; ratchet filter; pass staged file list |
| `src/config.mjs` | `checkers`, `astDisable` resolution; pattern `includeGlobs` validation |
| `src/regex-engine.mjs` | pattern-level `includeGlobs` support |
| `src/cli.mjs` | `--tier`, `baseline`, `install-hooks`, `audit` subcommands |
| `src/init.mjs` | scaffold `checkers` block + starter depcruise.cjs, call install-hooks, generate baseline |
| `src/report.mjs` | source grouping, skipped/baselined footers |
| `src/selftest.mjs` | parser-fixture stage |
| `rules/baseline/ast/*.yml` | semantic + depth + test + security ast rules + fixtures |
| `rules/baseline/index.mjs` | comment-slop patterns in `no-stubs`; new `test-slop` pack |
| `hooks/edit-hook.sh` | stop skipping `*.test.ts(x)` (test-slop pack needs fast-tier feedback) |
| `hooks/commit-hook.sh` | none — `--staged` implies commit tier automatically |

## 7. Out of scope (explicit) + backlog

Sub-projects (own spec each, after v2 ships):
- **Sub-project 2: `/slopgate` LLM-judge skill** — on-demand staged-diff review; the only layer for true deletion-test/seam-audit judgment, "semantically duplicates existing util", "this abstraction is decorative". Never hook-triggered.
- **Sub-project 3: rule harvesting loop** — mine learn-from-mistakes outputs + repeated violations into new rule-pack candidates.

Backlog (recorded, deliberately not in v2):
- Embeddings-based semantic duplicate detection (function-level similarity index).
- API-surface diff gate (api-extractor style: new public exports need acknowledgment).
- Mutation testing (Stryker) — audit/nightly, catches fake tests jscpd/test-slop can't.
- Coverage-on-changed-lines ratchet.
- Runtime smoke gate (build succeeds / app boots) — `verify` skill territory.
- Bundle-size ratchet (size-limit).
- Dependency-hygiene gate on package.json changes (size, maintenance, capability-duplicate of existing dep).
- Convention-drift stats (new file vs sibling structure outlier).
- Commented-out-code detection (regex too false-positive-prone; needs smarter heuristic).
- gitleaks secret-scan checker.
- Stale-TODO aging (audit).
- Destructive-migration detection.
- CI job / pre-push hook (revisit if `--no-verify` abuse becomes real).
- Auto-fix of violations.
- Non-TS languages in checkers (regex/ast packs stay language-agnostic as today).

## Architecture Decisions

- **fingerprint as internal function of `ratchet.mjs`, not own module** — single consumer; separate file failed the deletion test. (collapsed pre-emptively)
- **checker registry = static array export, not plugin discovery** — six known checkers, one machine; dynamic loading failed single-adapter/YAGNI. (collapsed pre-emptively)
- **checker adapter interface kept** — six real implementations on day one; hides per-tool invocation + parsing behind one shape. Deep module. (kept)
- **type-coverage reuses ratchet baseline instead of a percent watermark** — one debt mechanism, not two; fingerprints give per-expression precision a percentage can't. (collapsed pre-emptively)
- **tier logic inline in gate.mjs, not a tiers module** — two tiers, one branch point; a module would be a decorative seam. (collapsed pre-emptively)
- **`install-hooks` as separate CLI command (not init-only)** — needed standalone for re-install/upgrade on already-initialized repos. (kept)
- **`audit` as own module, not gate flag** — different lifecycle (periodic vs commit), different output (report vs block), never gates; sharing gate's pipeline would couple unrelated concerns. (kept)
- **depth detection split three ways** — per-file symptoms (pass-through, wrappers) → ast gate rules; graph metrics (export-ratio, fan-in==1, co-change) → audit; judgment (deletion test proper) → LLM-judge sub-project. Each landed in the cheapest layer that can express it. (kept)
