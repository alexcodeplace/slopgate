// src/stats/store.mjs
// JSONL stats store: location resolution + line-atomic append + tolerant read.
import { appendFileSync, readFileSync, mkdirSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { homedir } from 'node:os';

/** Cross-project global store. */
export function globalStatsPath() {
  return join(homedir(), '.slopgate', 'stats.jsonl');
}

/** Per-project mirror, next to the project's config. */
export function projectStatsPath(config) {
  return join(config.configDir, 'stats.jsonl');
}

/**
 * Append one row as a single JSON line. One appendFileSync call per row ->
 * under O_APPEND the write offset is atomic, so concurrent sessions' rows
 * never interleave.
 */
export function appendRow(path, obj) {
  mkdirSync(dirname(path), { recursive: true });
  appendFileSync(path, JSON.stringify(obj) + '\n');
}

/** Read all rows. Missing file -> []. Malformed lines skipped silently. */
export function readRows(path) {
  if (!existsSync(path)) return [];
  const rows = [];
  for (const line of readFileSync(path, 'utf8').split('\n')) {
    if (!line.trim()) continue;
    try { rows.push(JSON.parse(line)); } catch { /* skip malformed / partial write */ }
  }
  return rows;
}
