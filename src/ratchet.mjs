// src/ratchet.mjs
/**
 * Ratchet baseline: snapshot existing violations; gate fails only on NEW ones.
 * Fingerprint = sha256(engine|id|file|digit-normalized message|trimmed line text), 16 hex.
 * Line numbers excluded → survives unrelated line shifts. Identical (file,rule,line-text)
 * duplicates collapse to one fingerprint — acceptable for "did something NEW appear".
 * Malformed baseline → treated as empty with error surfaced (fail toward blocking).
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';

export function fingerprintViolation(v) {
  const msg = String(v.text ?? '').replace(/\d+/g, '#');
  const line = String(v.fullLine ?? '').trim();
  return createHash('sha256')
    .update([v.engine ?? '', v.id, v.file, msg, line].join('|'))
    .digest('hex').slice(0, 16);
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

export function filterNew(violations, entries) {
  const fresh = [];
  let baselinedCount = 0;
  for (const v of violations) {
    if (entries[fingerprintViolation(v)]) baselinedCount++;
    else fresh.push(v);
  }
  return { fresh, baselinedCount };
}

export function writeBaselineRaw(path, entries, generated) {
  writeFileSync(path, `${JSON.stringify({ version: 1, generated, entries }, null, 2)}\n`);
  return Object.keys(entries).length;
}

export function writeBaseline(path, violations, generated) {
  const entries = {};
  for (const v of violations) entries[fingerprintViolation(v)] = { ruleId: v.id, file: v.file };
  return writeBaselineRaw(path, entries, generated);
}
