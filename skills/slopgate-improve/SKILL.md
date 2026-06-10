---
name: slopgate-improve
description: Mine a project's institutional memory (skills, agents, commands, CLAUDE.md, editor rules) for conventions not yet caught by slopgate, and add them to the right rule tier. Invoke on /slopgate-improve or when a recurring agent mistake is documented but undetected.
---

# /slopgate-improve

**Goal:** Find real agent mistakes documented in the project's institutional memory that slopgate doesn't yet catch, and add them to the right engine tier.

Scope / focus area (optional): $ARGUMENTS — blank = all sources.

## Rule tier model

| Tier | Pack location | Who opts in |
|------|--------------|-------------|
| **baseline** | `slopgate/rules/baseline/index.mjs` | Any project — universal TypeScript/web rules |
| **stack** | `slopgate/rules/stack/index.mjs` | Projects using that runtime (e.g. `stack: ['cloudflare']`) |
| **project** | `<repo>/.slopgate/rules/<name>.mjs` | That repo only |

When proposing a new rule, assign the lowest tier where it applies without FPs:
- Applies to any TypeScript/web project → baseline
- Applies to all projects using a specific runtime/framework → stack
- Project-specific convention or business logic → project

## Phase 1 — Inventory current coverage

```bash
# List all enabled rule IDs (baseline + stack + project)
grep -o "id: '[^']*'" <repo>/.slopgate/rules/*.mjs 2>/dev/null
# Baseline packs enabled:
grep 'baseline:' <repo>/.slopgate/config.mjs
# Stack packs enabled:
grep 'stack:' <repo>/.slopgate/config.mjs
```

Build a set of already-covered rule IDs. Do NOT propose rules with IDs already present.

## Phase 2 — Extract rule candidates from ALL convention sources

Read `.slopgate/convention-sources.json` and open every file it lists:
- `claudeMd` roots and subtrees
- `skills/` — every SKILL.md
- `agents/` — every agent definition
- `commands/` — every command file
- `editorRules` (`.cursorrules`, `.windsurfrules`)
- `knowledgeDocs`

Also read: `git log --oneline -50` for recent commit messages that describe mistakes.

For each convention/mistake found, record:
- **source**: which file + section stated it
- **what**: the mistake or convention in one sentence
- **already covered**: yes/no (check Phase 1 set)

## Phase 3 — Bucket triage

For each uncovered convention:

| Bucket | Description | Action |
|--------|-------------|--------|
| A — regex | Token/pattern/import detectable by line scan | Author regex rule |
| B — ast | Structural pattern (call shape, attribute presence) | Author ast-grep rule |
| C — semantic | Judgment/intent — regex/AST would FP constantly | Add to judge-rules.md only |
| skip | Too noisy, too specific, or already enforced by other tooling | Document why, discard |

**Detectable (A/B):** banned token, hardcoded value where a utility is required, required attribute absence, import boundary violation, file-shape constraint.

**NOT detectable (C/skip):** semantic conventions ("use the knowledge graph first"), runtime behavior, data-shape requirements, anything needing type information a regex can't see.

## Phase 4A — Author regex candidates

For each bucket-A rule, draft:
```js
{
  id: '<kebab-id>',
  title: '<short title>',
  category: 'security|convention|duplication|boundary|api|i18n',
  severity: 'critical|high|medium|low',
  pattern: '<regex>',
  description: '<why this is wrong>',
  resolution: '<what to do instead>',
  excludeGlobs: ['<legit exception globs>'],
  canary: '<string the pattern must match>',
}
```

**Confidence check:** test the pattern against 3 real examples from the codebase:
- Does it match the violation? ✓
- Does it match anything it shouldn't? If yes → add excludeGlobs or raise threshold to medium/low.
- Low confidence → defer to C (judge-only), do not ship noise.

**Tier decision:** assign to baseline / stack / project per the tier model above.

## Phase 4B — Author ast-grep candidates

For each bucket-B rule, draft a YAML rule in ast-grep syntax:
- `language: typescript|tsx`
- `severity: error|warning`
- `note:` JSON with `{"severity":"high","category":"...","resolution":"..."}`
- `rule:` structural pattern
- `files:` / `ignores:` scope guards if needed

Add to correct dir:
- Baseline-quality → `slopgate/rules/baseline/ast/<id>.yml`
- Project-specific → `<repo>/.slopgate/rules/ast/<id>.yml`

Always add a fixture file with a canary hit.

## Phase 4C — Add judge-only candidates

For bucket-C rules: append to `<repo>/scripts/code-quality/judge-rules.md` (or equivalent):
```
| <id> | <one-line description of what to flag> |
```

## Phase 5 — Verify

For each new regex rule:
```bash
# Canary must match
node -e "console.log(new RegExp('<pattern>').test('<canary>'))"
# Run self-test
node /home/user/Projects/slopgate/bin/slopgate --self-test --config <repo>/.slopgate/config.mjs
# Dry-run on repo (count hits; review for FPs)
node /home/user/Projects/slopgate/bin/slopgate --config <repo>/.slopgate/config.mjs 2>&1 | grep '<id>'
```

If FP count > 5% of hits → add excludeGlobs or demote to medium/low. If still noisy → demote to bucket C.

## Phase 6 — Apply

- Baseline/stack rules: write to slopgate repo, commit (`feat(rules): add <id> to baseline|stack/cloudflare`)
- Project rules: write to `<repo>/.slopgate/rules/`, commit to project repo
- Run self-test one final time; must exit 0.

## Phase 7 — Report (conversation text only, no files)

```
## /slopgate-improve results
### Added — regex (N): `<id>` (tier:sev) — desc
### Added — ast (N): `<id>` (tier:sev) — desc
### Added — judge-only (N): `<id>` — desc
### Deferred (N): `<id>`: <why — too noisy / not statically expressible / already covered>
### Self-test: pass|fail
```

## Constraints

- NEVER author a rule that fires on > ~5% false positives without excluding the FP sources.
- NEVER duplicate an existing rule ID — Phase 1 set is authoritative.
- NEVER add a baseline rule for a project-specific business concept (e.g. "agorot math").
- NEVER skip the canary check — a rule without a matching canary is untestable and will rot.
- implementer reports candidates; orchestrator + user decide on tier and enable. Do NOT self-approve critical+ rules without user confirmation.
