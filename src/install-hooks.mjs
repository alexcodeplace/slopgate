// src/install-hooks.mjs
/**
 * Native git pre-commit installer. Marker-delimited block: idempotent rewrite of our
 * block, foreign hook content always preserved (block inserted before first `exec`,
 * else appended). Engine root resolved dynamically from this file's location.
 */
import { execSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync, chmodSync, mkdirSync } from 'node:fs';
import { join, isAbsolute, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

// engine root = parent of src/ (this file lives in src/)
export const ENGINE_ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
export const MARKER_BEGIN = '# slop-gate-hook v1 BEGIN';
export const MARKER_END = '# slop-gate-hook v1 END';

function hookBlock() {
  return [
    MARKER_BEGIN,
    'SLOPGATE_ROOT=$(git rev-parse --show-toplevel 2>/dev/null)',
    'if [ -n "$SLOPGATE_ROOT" ] && [ -f "$SLOPGATE_ROOT/.slop-gate/config.mjs" ]; then',
    `  node ${ENGINE_ROOT}/bin/slop-gate --staged --config "$SLOPGATE_ROOT/.slop-gate/config.mjs" || exit 1`,
    'fi',
    MARKER_END,
  ].join('\n');
}

export function renderHookContent(existing) {
  const block = hookBlock();
  if (!existing) return { content: `#!/usr/bin/env bash\n${block}\n`, action: 'created' };
  if (existing.includes(MARKER_BEGIN)) {
    const start = existing.indexOf(MARKER_BEGIN);
    const end = existing.indexOf(MARKER_END) + MARKER_END.length;
    const content = existing.slice(0, start) + block + existing.slice(end);
    return { content, action: content === existing ? 'unchanged' : 'updated' };
  }
  const lines = existing.split('\n');
  const execIdx = lines.findIndex((l) => /^\s*exec\s/.test(l));
  if (execIdx === -1) {
    return { content: `${existing.replace(/\n*$/, '\n')}${block}\n`, action: 'appended' };
  }
  lines.splice(execIdx, 0, block);
  return { content: lines.join('\n'), action: 'appended' };
}

export function resolveHooksDir(repoRoot) {
  let hooksPath = '';
  try { hooksPath = execSync('git config core.hooksPath', { cwd: repoRoot, encoding: 'utf8' }).trim(); } catch { /* unset */ }
  if (hooksPath) return isAbsolute(hooksPath) ? hooksPath : join(repoRoot, hooksPath);
  const gitDir = execSync('git rev-parse --git-dir', { cwd: repoRoot, encoding: 'utf8' }).trim();
  return join(isAbsolute(gitDir) ? gitDir : join(repoRoot, gitDir), 'hooks');
}

/** @returns {{ action: 'created'|'updated'|'appended'|'unchanged', path: string }} */
export function installPreCommitHook(repoRoot) {
  const hooksDir = resolveHooksDir(repoRoot);
  mkdirSync(hooksDir, { recursive: true });
  const path = join(hooksDir, 'pre-commit');
  const existing = existsSync(path) ? readFileSync(path, 'utf8') : null;
  const { content, action } = renderHookContent(existing);
  if (action !== 'unchanged') writeFileSync(path, content);
  chmodSync(path, 0o755);
  return { action, path };
}
