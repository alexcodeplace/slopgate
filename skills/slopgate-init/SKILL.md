---
name: slopgate-init
description: Onboard any repo to the global slopgate engine. Detect stack, mine the project's OWN local conventions (.claude/skills|agents|commands + CLAUDE.md subtree + editor rules), evaluate which are statically-detectable rule candidates, author the project rule pack, drive violations to zero, wire + verify hooks. Use when adopting slopgate in a new project or re-evaluating an existing project's rules.
---

# slopgate-init — Project Initialization

Bootstraps a repo onto the global slopgate engine (`/home/user/Projects/slopgate`). The engine
carries zero project knowledge; everything project-specific lives in `<repo>/.slopgate/`. This skill
produces that directory **from the project's own stated conventions**, not from a generic template.

Core idea: a project already documents what it cares about — in its `.claude/skills`, `.claude/agents`,
`.claude/commands`, its `CLAUDE.md` (+ subtree guides), and editor rule files (`.cursorrules`,
`.windsurfrules`). Mine those, keep only the conventions a static scanner can enforce, and mechanize them.

---

## Rule tier model

Before authoring any rule, decide which tier it belongs to:

| Tier | Lives in | Config key | Who benefits |
|------|----------|-----------|--------------|
| **baseline** | `slopgate/rules/baseline/index.mjs` | `baseline: ['pack-name']` | Any TypeScript/web project |
| **stack** | `slopgate/rules/stack/index.mjs` | `stack: ['cloudflare']` | Projects using that runtime/framework |
| **project** | `<repo>/.slopgate/rules/<name>.mjs` | `rules: ['./rules/<name>.mjs']` | This repo only |

Assign the **lowest tier** where the rule applies without false positives. Only project-specific business logic or tuned overrides belong in project tier.

---

## Step 0 — Already initialized?

Check if `.slopgate/config.mjs` exists in the target repo:

```bash
ls <repo>/.slopgate/config.mjs 2>/dev/null
```

If it exists → **stop, invoke `/slopgate-improve` instead.** This skill is for greenfield only.

---

## Step 1 — Scaffold (deterministic, the CLI does it)

```bash
node /home/user/Projects/slopgate/bin/slopgate init <repo-abs-path>
```

This auto-detects source roots (monorepo workspace-aware), exts, and skipDirs; writes a populated
`.slopgate/config.mjs`; emits `.slopgate/convention-sources.json` (the manifest of convention inputs
to read); and **safe-merges** the edit/commit hooks into the repo's existing `.claude/settings.json`
(appends, never clobbers — a `.bak` is written). Idempotent: re-running preserves an existing config.

Read the printed summary. **Sanity-check the detected `roots`** — fix `config.mjs` by hand if the repo
has an unusual layout the detector missed.

## Step 2 — Read the convention sources

Open `.slopgate/convention-sources.json`. Read every file it lists: `claudeMd` (root + subtrees),
`skills`, `agents`, `commands`, `editorRules`, `knowledgeDocs`. These are the project's own rules in
prose. (For large repos, push the reading to a cursor-agent that returns a candidate table, not file
dumps — see Step 4 / cursor-orchestrator return-size discipline.)

## Step 3 — Evaluate rule candidates (the heart of this skill)

For each convention you find, decide if a static scanner can enforce it. Build a candidate table:

| field | meaning |
|-------|---------|
| `id` | kebab rule id |
| `source` | which convention file + line stated it |
| `tier` | `baseline` \| `stack/<name>` \| `project` |
| `detect` | `regex` \| `ast` \| `none` |
| `confidence` | high \| med \| low (= false-positive risk, inverted) |
| `pattern` | the regex / ast pattern (draft) |
| `exceptGlobs` | legit exceptions (e.g. `**/tokens/**`, PDF/print, generated files) |
| `severity` | critical \| high \| (lower severities don't gate) |

**Detectable (good candidates):**
- Banned token / element / import — `<table>`, `as any`, `@ts-ignore`, a deprecated primitive import.
- Hardcoded value where a token is required — hex/rgb/hsl, raw px radius/shadow outside the token file.
- Required attribute presence — `<img>` without `width`/`height`, missing `alt`.
- Path-scoped import boundaries — ORM/db import inside `routes/**`, server-only import in client code.
- File-shape — non-`.webp` image refs, a stub/placeholder/TODO marker.

**NOT detectable (skip — do not force a brittle regex):**
- Semantic / judgment conventions — "use the knowledge-graph tool first", "add delight", "check the
  package before building a new atom", "never duplicate the nav".
- Runtime behavior, data-shape, or anything needing type information a regex can't see.

**Confidence rubric:** high = the pattern matches the violation and almost nothing else; med = some
false positives expected, needs `exceptGlobs` tuning; low = high FP risk → defer, don't ship noise.

**Authoring gotcha (mechanize correctly):** for import-membership / import-shape checks use a **regex
rule**, NOT an ast-grep `constraints` regex on a spread metavar (`$$$A`) — ast-grep constraints do not
filter spreads and the rule fires on every import. Reserve ast rules for structural patterns
(`$X.query($$$)` etc.). Line-oriented regex misses multi-line imports — accept that limit or use a
multiline scan; never trust a single-line grep's "0 hits" to prove a symbol is absent.

## Step 4 — Triage (high-reasoning; the implementer does NOT self-approve)

The candidate table is a set of *proposals*. Deciding which to enable — and whether a convention is
worth a rule at all — is a product-intent call. Per project discipline (cursor-orchestrator / zc-orchestrate),
an implementing/audit agent **reports** candidates; the orchestrator (+ user for genuine intent calls)
**decides**. Pick the high-confidence, low-FP candidates to ship now; defer low-confidence ones with a
one-line reason (never let them silently vanish). Enable baseline packs (`raw-hex`, `kv-ban`, …) only
when the candidate review shows the project actually wants them.

## Step 4b — Offer the UX module (greenfield only)

The UX module (`ux:{}` in config) is **optional and off by default** — UX taste varies, so it is
never auto-enabled. The scaffold writes it as a commented template. Decide whether to offer it:

- **Existing project with substantial UI already written** → do NOT push it. Mention one line
  ("UX module available — uncomment `ux:` in config to enable") and move on. Turning it on now would
  flag a pile of pre-existing markup; ratchet absorbs gating violations, but the advisory noise annoys.
- **Greenfield / "just vibing a new project"** → offer it. These are good-enough defaults for someone
  with no strong UI opinion. Ask the user which sub-modules to enable (don't assume):

  Sub-modules (`ux:` keys, value = severity; `'advisory'` reports but never blocks, `'high'` gates):
  | key | catches | default |
  |-----|---------|---------|
  | `a11y` | `<div onClick>`→`<button>`, anchor-no-href, img-no-alt, button-no-type, positive tabIndex (§11) | `high` |
  | `cls` | `<img>`/`<video>`/`<iframe>` without width/height → layout shift (§13) | `high` |
  | `feedback` | async `onClick` button with no `disabled` state → double-submit (§3/§12) | `high` |
  | `taste` | emoji-as-icon, "Trusted by", Lorem ipsum, robotic microcopy, heavy drop-shadow, linear/long motion (§0/§6/§26) | `advisory` |
  | `advisory` | modal-no-close, index-as-key, view-state-not-in-URL — higher false-positive nudges (§10/§14) | `advisory` |

  Use AskUserQuestion (multi-select sub-modules + a severity choice). On consent, uncomment/author the
  `ux:` block in `.slopgate/config.mjs`. Opt-out UX is symmetric: deleting a key disables one sub-module,
  deleting the block disables the module. Pair with the `/slopgate-ux` skill (prompt-time design
  directives) for the semantic UX rules a static scanner can't enforce.

## Step 5 — Author the approved rules

- **Baseline/stack rules** (generic enough): add to `slopgate/rules/baseline/index.mjs` or `slopgate/rules/stack/index.mjs`. Add fixture canary. Run slopgate self-test.
- **Project regex rules** → `.slopgate/rules/<project>-patterns.mjs` (array of `{id,title,category,severity,pattern,description,resolution,excludeGlobs?,canary}`). Every rule MUST have a `canary` string it matches.
- **Project AST rules** → `.slopgate/rules/ast/<id>.yml` (only for genuine structural patterns).
- Add `.slopgate/fixtures/src/` canary files so `--self-test` proves each rule fires.
- Wire enabled rule files into `config.mjs` `rules` / keep `astRules: './rules/ast'`.
- Override a baseline rule with the same `id` in project rules to tune pattern/excludeGlobs — last-wins dedup applies.

## Step 6 — Drive to zero (zero-tolerance before enabling)

```bash
node /home/user/Projects/slopgate/bin/slopgate --self-test --config .slopgate/config.mjs   # expect 0
# full dry-run count per rule id → must reach {}
```

**Exit 0 is NOT enough — confirm the self-test actually exercised every path.** A self-test that
*structurally cannot fail* is worse than none (it hid two real engine bugs behind a green adoption).
Read the lines, not just the code: every regex rule must print `OK <id>`, and the ast line must read
`OK ast-grep canary (N fixture violations)` with **N ≥ 1** — a `0`-violation canary, a `WARN ast-grep
unavailable`, or any `FAIL ast: …` line means the ast path didn't truly run (broken project ast rule,
missing binary, or wrong scan target). Treat those as a red self-test even if a later `exit=0` slips by.

For each non-zero id: fix the offending source (preferred) or, only with **user approval**, add a
`suppressions.json` entry. Re-run until counts are `{}`. Do not rely on the hooks firing until the
existing tree is clean — otherwise every later edit trips legacy debt.

## Step 7 — Verify hooks live

```bash
# self-test already green. Prove the PostToolUse wiring end-to-end:
echo 'export const c = "#ff0044";' > <a-scanned-root>/__slopgate_probe.ts
echo "{\"tool_input\":{\"file_path\":\"$PWD/<a-scanned-root>/__slopgate_probe.ts\"}}" | /home/user/Projects/slopgate/hooks/edit-hook.sh; echo "edit_hook=$?"
rm <a-scanned-root>/__slopgate_probe.ts
```

Expect `edit_hook=2` with the violation printed (only if a hex/hardcoded-value rule is enabled; else use
any enabled rule's canary). If not 2, the wiring is broken — fix before committing.

## Step 8 — Commit runtime config only

```bash
git add .slopgate/config.mjs .slopgate/rules .slopgate/suppressions.json .slopgate/convention-sources.json .claude/settings.json
git commit -m "feat: adopt slopgate (<project> rule pack + edit/commit hooks)"
```

Commit `.slopgate/**` + `.claude/settings.json` only — the pinned-rules design requires rules to live
in project git. Do NOT add fixtures-only or `.bak` files unless the repo wants them. If the repo's git
allow-list rejects `.slopgate/`, STOP and ask the user.

---

## Red flags

- Forcing a regex for a semantic convention → noise; if `detect: none`, skip it.
- ast-grep `constraints` on a `$$$` spread for import checks → mass false positives; use a regex rule.
- Wiring hooks before the tree is at zero → every edit trips legacy debt.
- Overwriting an existing `.claude/settings.json` → the CLI safe-merges; never hand-replace it.
- An implementing agent self-approving which conventions become rules → that's the orchestrator's call.
- Enabling a baseline pack the candidate review didn't justify (e.g. `kv-ban` in a non-CF repo).
- Authoring a project rule that belongs in baseline/stack — check existing packs first.
