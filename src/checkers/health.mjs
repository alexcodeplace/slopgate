// src/checkers/health.mjs
/** Consecutive infra-failure tracking per checker across commit-tier runs.
 *  The gate fails open on checker crash/timeout/missing-binary — correct per-commit,
 *  but a checker that infra-fails EVERY run is silently off forever. This counter
 *  escalates that into a loud warning without ever flipping the exit code.
 *  State lives in the self-gitignored cache dir (per-machine, not committed). */
import { readFileSync, writeFileSync, existsSync } from 'node:fs';

export const FAILURE_THRESHOLD = 2;

/** An error string that means "the tool did not actually run/produce results". */
export function isInfraError(msg) {
  return /failed:|crashed|killed by signal|JSON parse error/.test(msg);
}

/**
 * @param {string} path  health JSON path
 * @param {{ id:string, infraFailed:boolean, detail?:string, seconds?:number }[]} outcomes  one per enabled checker
 * @param {string} now   ISO timestamp
 * @returns {string[]} escalation warnings (consecutive failures ≥ threshold)
 */
export function updateCheckerHealth(path, outcomes, now) {
  let state = {};
  if (existsSync(path)) {
    try { state = JSON.parse(readFileSync(path, 'utf8')).checkers ?? {}; } catch { state = {}; }
  }
  const warnings = [];
  for (const { id, infraFailed, detail, seconds } of outcomes) {
    const cur = state[id] ?? { consecutiveFailures: 0, lastOk: null, lastFailure: null, lastError: null };
    if (infraFailed) {
      cur.consecutiveFailures += 1;
      cur.lastFailure = now;
      cur.lastError = detail ?? null;
    } else {
      cur.consecutiveFailures = 0;
      cur.lastOk = now;
    }
    if (typeof seconds === 'number') cur.lastDurationSeconds = seconds;
    state[id] = cur;
    if (cur.consecutiveFailures >= FAILURE_THRESHOLD) {
      warnings.push(`CHECKER OFF: ${id} infra-failed ${cur.consecutiveFailures} consecutive commit runs — its checks are NOT gating (fail-open). Last error: ${cur.lastError ?? 'unknown'}`);
    }
  }
  try { writeFileSync(path, `${JSON.stringify({ version: 1, checkers: state }, null, 2)}\n`); } catch { /* fail-open: health must never break a commit */ }
  return warnings;
}
