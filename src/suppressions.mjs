/**
 * False-positive suppression registry.
 * Match key = (id, file, sha1-of-trimmed-line). Content hash survives line drift;
 * a file move invalidates the entry (deliberate: forces re-review).
 * Malformed JSON → treated as empty with error surfaced (fail toward blocking).
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

export function lineHash(line) {
  return createHash('sha1').update(String(line).trim()).digest('hex');
}

export function loadSuppressions(path) {
  if (!path || !existsSync(path)) return { entries: [], error: null };
  try {
    const j = JSON.parse(readFileSync(path, 'utf8'));
    if (!Array.isArray(j.entries)) throw new Error('"entries" is not an array');
    return { entries: j.entries, error: null };
  } catch (err) {
    return { entries: [], error: String(err) };
  }
}

/** violation must carry { id, file, lineHash } */
export function isSuppressed(entries, v) {
  return entries.some((e) => e.id === v.id && e.file === v.file && e.lineHash === v.lineHash);
}

/**
 * Stale detection: entry whose (file missing) or (no line in file hashes to lineHash).
 * Default prunes (writes file when something was removed); { dryRun: true } only reports.
 * @param {string} repoRoot absolute repo root that `entry.file` is relative to
 * @param {string} path absolute path to suppressions.json
 * @param {{ dryRun?: boolean }} [opts]
 */
export function pruneStale(repoRoot, path, { dryRun = false } = {}) {
  const { entries, error } = loadSuppressions(path);
  if (error) return { pruned: [], kept: entries, error };
  const kept = [];
  const pruned = [];
  for (const e of entries) {
    const abs = join(repoRoot, e.file);
    if (!existsSync(abs)) { pruned.push(e); continue; }
    const lines = readFileSync(abs, 'utf8').split('\n');
    if (lines.some((l) => lineHash(l) === e.lineHash)) kept.push(e);
    else pruned.push(e);
  }
  if (pruned.length && !dryRun) writeFileSync(path, JSON.stringify({ version: 1, entries: kept }, null, 2) + '\n');
  return { pruned, kept, error: null };
}