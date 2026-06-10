// src/audit/git-facts.mjs
/** All git plumbing for `slopgate audit` — extraction only, no scoring (that's measures.mjs).
 *  Every function fail-opens to empty data on any git error (not a repo, shallow history):
 *  audit sections skip-with-notice on empty inputs, they never crash. */
import { execFileSync } from 'node:child_process';

function git(repoRoot, args) {
  try {
    return execFileSync('git', args, { cwd: repoRoot, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
  } catch {
    return '';
  }
}

/** Commits-touching-file counts within the window. @returns Map<repoRelPath, count> */
export function churnByFile(repoRoot, sinceDays) {
  const raw = git(repoRoot, ['log', `--since=${sinceDays} days ago`, '--name-only', '--format=']);
  const map = new Map();
  for (const line of raw.split('\n')) {
    const f = line.trim();
    if (f) map.set(f, (map.get(f) ?? 0) + 1);
  }
  return map;
}

/** Per-commit file sets within the window (for co-change mining).
 *  Commits touching > maxFiles files are skipped — bulk refactors poison coupling stats. */
export function commitFileSets(repoRoot, sinceDays, { maxFiles = 20 } = {}) {
  const raw = git(repoRoot, ['log', `--since=${sinceDays} days ago`, '--name-only', '--format=%H']);
  const sets = [];
  let cur = null;
  for (const line of raw.split('\n')) {
    if (/^[0-9a-f]{40}$/.test(line)) { cur = []; sets.push(cur); }
    else if (line.trim() && cur) cur.push(line.trim());
  }
  return sets.filter((s) => s.length > 0 && s.length <= maxFiles);
}

/** Author commit shares for a path prefix. @returns [{author, commits, share}] sorted desc */
export function authorShares(repoRoot, sinceDays, dir) {
  const raw = git(repoRoot, ['log', `--since=${sinceDays} days ago`, '--format=%an', '--', dir]);
  const counts = new Map();
  let total = 0;
  for (const line of raw.split('\n')) {
    const a = line.trim();
    if (!a) continue;
    total++;
    counts.set(a, (counts.get(a) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([author, commits]) => ({ author, commits, share: commits / total }))
    .sort((a, b) => b.commits - a.commits);
}

/** File content at the last commit before `daysAgo`. null = no rev / file absent at that rev. */
export function fileAtDaysAgo(repoRoot, file, daysAgo) {
  const sha = git(repoRoot, ['rev-list', '-1', `--before=${daysAgo} days ago`, 'HEAD']).trim();
  if (!sha) return null;
  const out = git(repoRoot, ['show', `${sha}:${file}`]);
  return out || null;
}

/** Entry-count history of a committed { entries: {...} } JSON file, oldest → newest.
 *  Used for the ratchet burn-down curve — baseline.json is committed, history is free. */
export function jsonEntryHistory(repoRoot, relPath, entriesKey = 'entries') {
  const raw = git(repoRoot, ['log', '--format=%H %cI', '--', relPath]);
  const out = [];
  for (const line of raw.trim().split('\n')) {
    const [sha, ts] = line.split(' ');
    if (!sha) continue;
    const blob = git(repoRoot, ['show', `${sha}:${relPath}`]);
    try { out.push({ ts, count: Object.keys(JSON.parse(blob)[entriesKey] ?? {}).length }); }
    catch { /* malformed at that rev — skip point */ }
  }
  return out.reverse();
}
