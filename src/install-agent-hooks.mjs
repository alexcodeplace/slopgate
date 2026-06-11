// src/install-agent-hooks.mjs
import { existsSync, readFileSync, writeFileSync, copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { homedir } from 'node:os';
import { execFileSync } from 'node:child_process';
import { ENGINE_ROOT } from './install-hooks.mjs';

const COMMIT_HOOK = `${ENGINE_ROOT}/hooks/commit-hook.sh`;
const EDIT_HOOK = `${ENGINE_ROOT}/hooks/edit-hook.sh`;
const SESSION_HOOK = `${ENGINE_ROOT}/hooks/session-start.sh`;

function which(cmd) {
  try { execFileSync('which', [cmd], { stdio: 'pipe' }); return true; } catch { return false; }
}

function isSlopgateCmd(cmd) {
  return typeof cmd === 'string' && cmd.includes(ENGINE_ROOT);
}

/** Idempotently merge slopgate hooks into a claude-format hooks JSON file. */
function mergeHooks(filePath) {
  const existed = existsSync(filePath);
  let root = {};
  if (existed) {
    try { root = JSON.parse(readFileSync(filePath, 'utf8')); }
    catch { return { action: 'invalid-json', path: filePath }; }
  }

  if (!root.hooks) root.hooks = {};
  let changed = false;

  // SessionStart
  if (!Array.isArray(root.hooks.SessionStart)) root.hooks.SessionStart = [];
  if (!root.hooks.SessionStart.some(e => Array.isArray(e?.hooks) && e.hooks.some(h => h?.command === SESSION_HOOK))) {
    root.hooks.SessionStart.push({ hooks: [{ type: 'command', command: SESSION_HOOK }] });
    changed = true;
  }

  // PreToolUse (Bash)
  if (!Array.isArray(root.hooks.PreToolUse)) root.hooks.PreToolUse = [];
  let preEntry = root.hooks.PreToolUse.find(e => e?.matcher === 'Bash');
  if (!preEntry) { preEntry = { matcher: 'Bash', hooks: [] }; root.hooks.PreToolUse.push(preEntry); }
  if (!Array.isArray(preEntry.hooks)) preEntry.hooks = [];
  if (!preEntry.hooks.some(h => h?.command === COMMIT_HOOK)) {
    preEntry.hooks.push({ type: 'command', command: COMMIT_HOOK });
    changed = true;
  }

  // PostToolUse (Edit|Write)
  if (!Array.isArray(root.hooks.PostToolUse)) root.hooks.PostToolUse = [];
  let postEntry = root.hooks.PostToolUse.find(e => e?.matcher === 'Edit|Write');
  if (!postEntry) { postEntry = { matcher: 'Edit|Write', hooks: [] }; root.hooks.PostToolUse.push(postEntry); }
  if (!Array.isArray(postEntry.hooks)) postEntry.hooks = [];
  if (!postEntry.hooks.some(h => h?.command === EDIT_HOOK)) {
    postEntry.hooks.push({ type: 'command', command: EDIT_HOOK });
    changed = true;
  }

  if (!changed) return { action: 'already-present', path: filePath };
  mkdirSync(dirname(filePath), { recursive: true });
  if (existed) copyFileSync(filePath, `${filePath}.bak`);
  writeFileSync(filePath, `${JSON.stringify(root, null, 2)}\n`);
  return { action: existed ? 'merged' : 'created', path: filePath };
}

/** Remove all slopgate hooks from a claude-format hooks JSON file. */
function removeHooks(filePath) {
  if (!existsSync(filePath)) return { action: 'not-found', path: filePath };
  let root;
  try { root = JSON.parse(readFileSync(filePath, 'utf8')); }
  catch { return { action: 'invalid-json', path: filePath }; }

  if (!root.hooks) return { action: 'not-present', path: filePath };
  let changed = false;

  for (const event of Object.keys(root.hooks)) {
    if (!Array.isArray(root.hooks[event])) continue;
    root.hooks[event] = root.hooks[event].map(entry => {
      if (!Array.isArray(entry?.hooks)) return entry;
      const filtered = entry.hooks.filter(h => !isSlopgateCmd(h?.command));
      if (filtered.length === entry.hooks.length) return entry;
      changed = true;
      return filtered.length ? { ...entry, hooks: filtered } : null;
    }).filter(Boolean);
    if (root.hooks[event].length === 0) { delete root.hooks[event]; changed = true; }
  }
  if (Object.keys(root.hooks).length === 0) { delete root.hooks; changed = true; }

  if (!changed) return { action: 'not-present', path: filePath };
  copyFileSync(filePath, `${filePath}.bak`);
  writeFileSync(filePath, `${JSON.stringify(root, null, 2)}\n`);
  return { action: 'removed', path: filePath };
}

/** Check how many of the 3 slopgate hooks are present. */
function checkStatus(filePath) {
  if (!existsSync(filePath)) return 'not-installed';
  let root;
  try { root = JSON.parse(readFileSync(filePath, 'utf8')); } catch { return 'invalid-json'; }
  const h = root?.hooks;
  if (!h) return 'not-installed';
  const s = Array.isArray(h.SessionStart) && h.SessionStart.some(e => Array.isArray(e?.hooks) && e.hooks.some(x => x?.command === SESSION_HOOK));
  const p = Array.isArray(h.PreToolUse) && h.PreToolUse.some(e => Array.isArray(e?.hooks) && e.hooks.some(x => x?.command === COMMIT_HOOK));
  const w = Array.isArray(h.PostToolUse) && h.PostToolUse.some(e => Array.isArray(e?.hooks) && e.hooks.some(x => x?.command === EDIT_HOOK));
  const n = [s, p, w].filter(Boolean).length;
  return n === 3 ? 'installed' : n > 0 ? 'partial' : 'not-installed';
}

// cursor-agent shares ~/.claude/settings.json with claude/cld
export const AGENTS = [
  {
    id: 'claude',
    label: 'claude / cld / cursor-agent',
    detect: () => which('claude') || which('cld') || which('cursor-agent'),
    filePath: join(homedir(), '.claude', 'settings.json'),
  },
  {
    id: 'codex',
    label: 'codex',
    detect: () => which('codex'),
    filePath: join(homedir(), '.codex', 'hooks.json'),
  },
  {
    id: 'grok',
    label: 'grok',
    detect: () => which('grok'),
    filePath: join(homedir(), '.grok', 'hooks', 'slopgate.json'),
  },
  {
    id: 'gemini',
    label: 'gemini',
    detect: () => which('gemini'),
    filePath: join(homedir(), '.gemini', 'settings.json'),
  },
];

/** Install slopgate hooks for all detected (or specified) agents. */
export function installAgentHooks({ agentIds } = {}) {
  const targets = agentIds
    ? AGENTS.filter(a => agentIds.includes(a.id))
    : AGENTS.filter(a => a.detect());
  return targets.map(a => ({ id: a.id, label: a.label, ...mergeHooks(a.filePath) }));
}

/** Remove slopgate hooks for all (or specified) agents. */
export function removeAgentHooks({ agentIds } = {}) {
  const targets = agentIds ? AGENTS.filter(a => agentIds.includes(a.id)) : AGENTS;
  return targets.map(a => ({ id: a.id, label: a.label, ...removeHooks(a.filePath) }));
}

/** Return status for all agents (detected or not). */
export function statusAgentHooks() {
  return AGENTS.map(a => ({
    id: a.id,
    label: a.label,
    detected: a.detect(),
    status: checkStatus(a.filePath),
    path: a.filePath,
  }));
}
