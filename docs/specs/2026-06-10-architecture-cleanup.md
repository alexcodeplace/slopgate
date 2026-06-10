# Spec: Architecture cleanup (changes 1–4)

Date: 2026-06-10
Status: proposed
Scope: structural cleanup only — no behavior change, no new rules. Every existing test must still pass; no new violations introduced.

Source: code-quality review of slopgate engine. These are the four "worth doing" structural items. Perf/exception-safety/determinism live in the separate deeper-dive audit and are out of scope here.

---

## Change 1 — Decompose `init.mjs` (depth outlier)

**Problem.** `src/init.mjs` is 370 lines — 4× the next-largest source file (`gate.mjs`, 92). One file, one export, four responsibilities: stack/workspace detection, workspace-glob expansion, config + depcruise-starter scaffolding, hook wiring. This is the single highest-cognitive-load spot in the codebase.

**Target.** Split by responsibility, keep `init.mjs` as a thin orchestrator that wires the pieces and owns the CLI-facing `runInit(dir) -> exitCode` contract.

- `src/init/detect-stack.mjs`
  - `readPackageJson(targetDir)`
  - `workspacePatterns(pkg)`
  - `expandWorkspaceGlobs(targetDir, patterns)`
  - any stack/root/ext detection helpers currently inline in init.mjs
  - exports pure functions; no writes.
- `src/init/scaffold.mjs`
  - `DEPCRUISE_STARTER` constant + config-file generation (the slopgate config + depcruise starter writers)
  - owns all `writeFileSync`/`mkdirSync`/`copyFileSync` for scaffolding.
- `src/init.mjs` (orchestrator)
  - imports the two modules + `installPreCommitHook`
  - sequences: detect → scaffold → wire hooks → return code
  - keeps the hook-config constants (`PRE_TOOL`, `POST_TOOL`, `COMMIT_HOOK`, `EDIT_HOOK`) OR move them next to the hook wiring — pick one home, document it.

**Constraints.**
- `runInit` signature and exit-code semantics unchanged (cli.mjs:28-31 calls it).
- No new directory if the team prefers flat `src/`: acceptable fallback is `src/init-detect.mjs` + `src/init-scaffold.mjs`. Decide once, be consistent.
- `init.test.mjs` must pass unchanged. If helper functions were previously module-private and the test reaches them, re-export from the new module and update the import path only.

**Acceptance.** `init.mjs` < ~120 lines; each new file one responsibility; `init.test.mjs` green.

**Anti-goal.** Do NOT introduce a plugin/registry abstraction for stack detection. Two-three plain exported functions, called directly.

---

## Change 2 — Extract `runJsonTool(bin, args, opts) -> { data, errors }`

**Problem.** `knip.mjs` and `depcruise.mjs` repeat the identical 5-line ceremony: `runTool` → `if (!res.ok) return {violations:[], errors:[...]}` → `JSON.parse` in try/catch → error-wrap. Verbatim duplication of logic (not just shape).

**Target.** Add to `src/checkers/shared.mjs`:

```js
/** Run a tool expected to emit JSON on stdout. Returns parsed data or a wrapped error.
 *  Never throws. { data:<parsed>|null, errors:string[] } */
export function runJsonTool(label, bin, args, opts) {
  const res = runTool(bin, args, opts);
  if (!res.ok) return { data: null, errors: [`${label} failed: ${res.error}`] };
  try { return { data: JSON.parse(res.stdout), errors: [] }; }
  catch (e) { return { data: null, errors: [`${label} JSON parse error: ${e}`] }; }
}
```

**Apply to:** `knip.mjs`, `depcruise.mjs` (both read JSON from stdout). Each `run()` becomes: call `runJsonTool` → if `data == null` return its errors → map `parse*Output(data)` to violations.

**Do NOT apply to:**
- `jscpd.mjs` — reads JSON from a temp-file report, not stdout (different I/O). Leave as-is (see Change 3 for its tmpdir).
- `tsc.mjs`, `type-coverage.mjs` — parse line-oriented text, not JSON. Untouched.
- `ast-engine.mjs` — distinct stderr-disambiguation + JSON path; leave (it is not a checker).

**Hard constraint — DO NOT over-DRY.** Do not extract a `defineToolChecker({parse, toViolations})` super-adapter. Each checker's `parse*` and violation-mapping are genuinely unique (jscpd staged-side selection, tsc multiline continuation, knip issue-type loop, depcruise severity map). The only shared logic is spawn+ok-guard+JSON.parse — that, and only that, is what `runJsonTool` captures. The `parse*Output` functions stay exported per-checker (tests call them directly).

**Acceptance.** `knip.test.mjs`, `depcruise.test.mjs`, `shared.test.mjs` green. Net line reduction in those two checkers. The `parse*Output` exports keep their current signatures so existing unit tests are untouched (note: `parseKnipOutput`/`parseDepcruiseOutput` currently take a JSON *string* and parse internally — to use `runJsonTool` they must accept already-parsed data, OR `runJsonTool` returns raw text. **Decision: keep `parse*Output` taking a string** and have them call nothing new; `runJsonTool` returns `{ data: <parsed object> }` AND we change the parsers to accept the object. Pick the variant that leaves test call-sites smallest — if tests pass JSON strings, instead have `runJsonTool` return `{ text }` and keep parsers string-taking. Resolve at implementation time; minimize test churn.)

---

## Change 3 — `withTempDir(fn)` helper

**Problem.** `ast-engine.mjs` (mkdtemp for sgconfig) and `jscpd.mjs` (mkdtemp for report output) both do `mkdtempSync(join(tmpdir(), 'slopgate-*-'))` + `try { ... } finally { rmSync(dir, {recursive:true, force:true}) }`. Two copies of the same lifecycle.

**Target.** Add a small helper (location: `src/checkers/shared.mjs` if both importers reach it cleanly; `ast-engine.mjs` is NOT under `checkers/`, so consider `src/temp.mjs` to avoid a checker→shared import from an engine). Decide based on import direction; prefer a neutral `src/temp.mjs`.

```js
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/** Make a temp dir, pass it to fn, always remove it. Returns fn's result. */
export function withTempDir(prefix, fn) {
  const dir = mkdtempSync(join(tmpdir(), prefix));
  try { return fn(dir); }
  finally { rmSync(dir, { recursive: true, force: true }); }
}
```

**Apply to:** `ast-engine.mjs` (`slopgate-sg-`), `jscpd.mjs` (`slopgate-jscpd-`).

**Constraint.** `fn` is synchronous (both call sites are sync — spawnSync). Do not make it async; that would force a wider refactor.

**Acceptance.** `jscpd.test.mjs` and any ast-engine test green; both call sites use the helper; cleanup still runs on the error path (verify with a forced-throw test or by inspection).

---

## Change 4 — Stop mutating `config._fileTarget`

**Problem.** `cli.mjs:75` stuffs transient per-invocation state onto the resolved-config object as a private `_fileTarget` field, which `gate.mjs:17` (`collectViolations`) then reads. The resolved config is otherwise an immutable description of the repo; this one field is request-scoped state riding on it.

**Target.** Thread the file target through the call path explicitly instead of via the config object.

- `runGate(mode, config, { tier } = {})` → `runGate(mode, config, { tier, fileTarget } = {})`.
- `runGate` passes it into `collectViolations(mode, config, tier, { fileTarget })` (add an opts param).
- `collectViolations` builds `opts` for `listSourceFiles` from the passed `fileTarget` instead of `config._fileTarget`.
- `cli.mjs:74-75`: `runGate('file', config, { tier: ..., fileTarget })` — drop the `config._fileTarget = ...` mutation.

**Constraint.** `runGate` and `collectViolations` are imported by `cli.mjs` and the tests. Update `gate.e2e.test.mjs` / `gate.tier.test.mjs` call sites if they pass `_fileTarget`. Keep `staged`/`full` modes unaffected (they ignore fileTarget).

**Acceptance.** No reference to `_fileTarget` anywhere; file-mode gate still targets the single file; all gate tests green.

---

## Out of scope (tracked elsewhere)
- Parallelizing the checker loop, checker `run()` try/catch wrapping, PATH-fallback determinism, orchestration-seam test gaps → deeper-dive audit (separate doc).
- diff-shape "shallowness", engine/checker dispatch asymmetry → intentional, no action.

## Suggested order
4 → 2 → 3 → 1. (4 is smallest and touches the orchestrator; 2 and 3 are local to checkers; 1 is the largest, do last when the rest is green.)
