import { existsSync, statSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { BASELINE_PACKS, BASELINE_AST_DIR, BASELINE_FIXTURES_DIR } from '../rules/baseline/index.mjs';
import { STACK_PACKS } from '../rules/stack/index.mjs';
import { UX_PACKS, UX_AST_DIR, UX_ALL_AST_IDS, resolveUxSeverity } from '../rules/ux/index.mjs';

/** @typedef {import('../rules/baseline/index.mjs')} _b */

function gitRoot(fromDir) {
  try { return execSync('git rev-parse --show-toplevel', { cwd: fromDir, encoding: 'utf8' }).trim(); }
  catch { return null; }
}

function validatePattern(p, src) {
  for (const k of ['id', 'severity', 'pattern', 'resolution']) {
    if (!p[k]) throw new Error(`slopgate: rule from ${src} missing "${k}" (id=${p.id ?? '?'})`);
  }
  try { new RegExp(p.pattern, p.flags || undefined); } catch (e) { throw new Error(`slopgate: rule ${p.id} bad regex: ${e}`); }
  return p;
}

export async function resolveConfig(configPath) {
  const absConfig = isAbsolute(configPath) ? configPath : resolve(process.cwd(), configPath);
  if (!existsSync(absConfig) || !statSync(absConfig).isFile()) throw new Error(`slopgate: config not found: ${absConfig}`);
  const configDir = dirname(absConfig);
  const repoRoot = gitRoot(configDir) || dirname(configDir);
  const raw = (await import(pathToFileURL(absConfig).href)).default;

  // baseline packs (opt-in by name)
  const patterns = [];
  for (const name of raw.baseline ?? []) {
    if (!BASELINE_PACKS[name]) throw new Error(`slopgate: unknown baseline pack "${name}" (known: ${Object.keys(BASELINE_PACKS).join(', ')})`);
    for (const p of BASELINE_PACKS[name]) patterns.push(validatePattern(p, `baseline:${name}`));
  }
  // stack packs (opt-in by name — e.g. stack: ['cloudflare'])
  for (const name of raw.stack ?? []) {
    if (!STACK_PACKS[name]) throw new Error(`slopgate: unknown stack pack "${name}" (known: ${Object.keys(STACK_PACKS).join(', ')})`);
    for (const p of STACK_PACKS[name]) patterns.push(validatePattern(p, `stack:${name}`));
  }
  // project rule packs
  for (const relPath of raw.rules ?? []) {
    const abs = isAbsolute(relPath) ? relPath : resolve(configDir, relPath);
    const mod = (await import(pathToFileURL(abs).href)).default;
    if (!Array.isArray(mod)) throw new Error(`slopgate: rule pack ${relPath} must default-export an array`);
    for (const p of mod) patterns.push(validatePattern(p, relPath));
  }

  // UX module (optional): opt-in per sub-module via ux:{ <name>: severity }.
  // A value of 'advisory' aliases to medium (non-gating); omit a key to disable.
  // Regex rules join `patterns` (severity overridden to the chosen value); ast
  // rule ids are collected so the engine only keeps/gates enabled ux ast rules.
  const uxAstSeverity = new Map();
  let uxEnabledAst = false;
  for (const [key, value] of Object.entries(raw.ux ?? {})) {
    const pack = UX_PACKS[key];
    if (!pack) throw new Error(`slopgate: unknown ux sub-module "${key}" (known: ${Object.keys(UX_PACKS).join(', ')})`);
    const sev = resolveUxSeverity(value, pack);
    if (!sev) continue; // disabled
    for (const p of pack.regex) patterns.push(validatePattern({ ...p, severity: sev }, `ux:${key}`));
    for (const id of pack.astIds) { uxAstSeverity.set(id, sev); uxEnabledAst = true; }
  }

  // dedupe by id (last-wins value, first-occurrence order — both guaranteed by Map)
  const byId = new Map();
  for (const p of patterns) byId.set(p.id, p);
  const dedupedPatterns = [...byId.values()];

  // ast rule dirs: baseline ast + project ast (if present)
  const astRuleDirs = [BASELINE_AST_DIR];
  if (raw.astRules) {
    const abs = isAbsolute(raw.astRules) ? raw.astRules : resolve(configDir, raw.astRules);
    if (existsSync(abs) && statSync(abs).isDirectory()) astRuleDirs.push(abs);
  }
  // UX ast rules only scanned when a ux sub-module that owns ast rules is enabled.
  if (uxEnabledAst) astRuleDirs.push(UX_AST_DIR);

  // commit-tier checkers: absent/false => disabled; true => {}; object => options
  const checkers = {};
  for (const [name, v] of Object.entries(raw.checkers ?? {})) {
    if (v === false || v == null) continue;
    checkers[name] = v === true ? {} : v;
  }

  const rootsRel = (raw.roots ?? ['src']);
  return {
    repoRoot, configDir,
    roots: rootsRel.map((r) => join(repoRoot, r)),
    rootsRel,
    exts: new Set(raw.exts ?? ['.ts', '.tsx', '.astro']),
    skipDirs: new Set(raw.skipDirs ?? ['node_modules', 'dist', 'tests']),
    patterns: dedupedPatterns,
    astRuleDirs,
    checkers,
    astDisable: new Set(raw.astDisable ?? []),
    baselinePath: join(configDir, 'baseline.json'),
    gate: { file: raw.gate?.file ?? ['critical', 'high'], staged: raw.gate?.staged ?? ['critical', 'high'] },
    suppressionsPath: raw.suppressions
      ? (isAbsolute(raw.suppressions) ? raw.suppressions : resolve(configDir, raw.suppressions))
      : join(configDir, 'suppressions.json'),
    fixturesDirs: [BASELINE_FIXTURES_DIR, raw.fixtures ? resolve(configDir, raw.fixtures) : null].filter(Boolean),
    checkerConcurrency: raw.checkerConcurrency ?? 3,
    // UX module ast gating: which ux ast ids are enabled (id → severity), and the
    // full set of ux ast ids so the engine can DROP any that aren't enabled.
    uxAstSeverity,
    uxAstAll: new Set(UX_ALL_AST_IDS),
  };
}