// src/stats/record.mjs
// Build one row per blocked violation and write to the global + project stores.
// Session/model resolution lives here (single caller — no separate module).
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { homedir } from 'node:os';
import { basename, join } from 'node:path';
import { appendRow, globalStatsPath, projectStatsPath } from './store.mjs';

function sessionKey(repoRoot) {
  return createHash('sha256').update(repoRoot).digest('hex').slice(0, 16);
}

/**
 * Resolve the recording model. Precedence: env SLOPGATE_MODEL -> SessionStart
 * file (~/.slopgate/sessions/<key>.json) -> 'unknown'. Never throws.
 * @returns {{ model:string, sessionId:string|null, startedAt:string|null }}
 */
export function resolveSession(repoRoot) {
  if (process.env.SLOPGATE_MODEL) {
    return { model: process.env.SLOPGATE_MODEL, sessionId: null, startedAt: null };
  }
  try {
    const p = join(homedir(), '.slopgate', 'sessions', `${sessionKey(repoRoot)}.json`);
    const s = JSON.parse(readFileSync(p, 'utf8'));
    return { model: s.model || 'unknown', sessionId: s.sessionId ?? null, startedAt: s.startedAt ?? null };
  } catch {
    return { model: 'unknown', sessionId: null, startedAt: null };
  }
}

/**
 * Append one row per blocked violation to the global + project stores.
 * Caller MUST wrap in try/catch (fail-open) — stats must never break a commit.
 * @param {object[]} violations  blocked-only set (post severity+suppression+ratchet)
 * @param {{ repoRoot:string, configDir:string }} config
 * @param {{ mode:string }} ctx
 * @returns {number} rows written
 */
export function recordIncidents(violations, config, { mode }) {
  if (!violations?.length) return 0;
  const { model, sessionId } = resolveSession(config.repoRoot);
  const ts = new Date().toISOString();
  const project = basename(config.repoRoot);
  const gp = globalStatsPath();
  const pp = projectStatsPath(config);
  for (const v of violations) {
    const row = {
      ts,
      project,
      projectPath: config.repoRoot,
      model,
      sessionId,
      mode,
      ruleId: v.id ?? 'unknown',
      severity: v.severity ?? 'unknown',
      category: v.category ?? 'unknown',
      engine: v.engine ?? 'unknown',
      file: v.file ?? null,
      line: v.line ?? null,
    };
    appendRow(gp, row);
    appendRow(pp, row);
  }
  return violations.length;
}
