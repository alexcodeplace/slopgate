// src/ratchet.mjs
/**
 * Ratchet baseline: snapshot existing violations; gate fails only on NEW ones.
 * Fingerprint = sha256(engine|id|file|digit-normalized message|trimmed line text), 16 hex.
 * Line numbers excluded → survives unrelated line shifts. Identical (file,rule,line-text)
 * duplicates collapse to one fingerprint — acceptable for "did something NEW appear".
 * Editing the violating LINE invalidates the fingerprint — intentional (boy-scout rule:
 * touch the line, fix the debt). Renaming the FILE does not: filterNew follows staged
 * renames so pure moves don't re-flag baselined debt.
 * Malformed baseline → treated as empty with error surfaced (fail toward blocking).
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { execSync } from 'node:child_process';

export function fingerprintViolation(v, fileOverride) {
  const msg = String(v.text ?? '').replace(/\d+/g, '#');
  const line = String(v.fullLine ?? '').trim();
  return createHash('sha256')
    .update([v.engine ?? '', v.id, fileOverride ?? v.file, msg, line].join('|'))
    .digest('hex').slice(0, 16);
}

/** Staged renames as { newPath: oldPath }. Fail-open: any git error → {}. */
export function stagedRenames(repoRoot) {
  try {
    const raw = execSync('git diff --cached -M --name-status --diff-filter=R', { cwd: repoRoot, encoding: 'utf8' });
    const map = {};
    for (const line of raw.split('\n')) {
      const m = /^R\d*\t([^\t]+)\t([^\t]+)$/.exec(line);
      if (m) map[m[2]] = m[1];
    }
    return map;
  } catch { return {}; }
}

export function loadBaseline(path) {
  if (!path || !existsSync(path)) return { entries: {}, missing: true, error: null };
  try {
    const j = JSON.parse(readFileSync(path, 'utf8'));
    if (!j.entries || typeof j.entries !== 'object' || Array.isArray(j.entries)) {
      throw new Error('"entries" is not an object');
    }
    return { entries: j.entries, missing: false, error: null };
  } catch (err) {
    return { entries: {}, missing: false, error: String(err) };
  }
}

export function filterNew(violations, entries, { renames = {} } = {}) {
  const fresh = [];
  let baselinedCount = 0;
  for (const v of violations) {
    const hit = entries[fingerprintViolation(v)]
      || (renames[v.file] && entries[fingerprintViolation(v, renames[v.file])]);
    if (hit) baselinedCount++;
    else fresh.push(v);
  }
  return { fresh, baselinedCount };
}

export function writeBaselineRaw(path, entries, generated) {
  // sorted keys → deterministic file → reviewable diffs, mergeable branches
  const sorted = Object.fromEntries(Object.keys(entries).sort().map((k) => [k, entries[k]]));
  writeFileSync(path, `${JSON.stringify({ version: 1, generated, entries: sorted }, null, 2)}\n`);
  return Object.keys(sorted).length;
}

export function writeBaseline(path, violations, generated) {
  const entries = {};
  for (const v of violations) entries[fingerprintViolation(v)] = { ruleId: v.id, file: v.file };
  return writeBaselineRaw(path, entries, generated);
}
