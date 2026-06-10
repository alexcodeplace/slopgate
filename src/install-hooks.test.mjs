// src/install-hooks.test.mjs
import { mkdtempSync, mkdirSync, rmSync, statSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { execSync } from 'node:child_process';
import { installPreCommitHook, renderHookContent, MARKER_BEGIN } from './install-hooks.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

// pure rendering
const fresh = renderHookContent(null);
assert('fresh: shebang + marker + invocation', fresh.action === 'created'
  && fresh.content.startsWith('#!/usr/bin/env bash')
  && fresh.content.includes(MARKER_BEGIN)
  && fresh.content.includes('--staged --config'));

const again = renderHookContent(fresh.content);
assert('re-render same content → unchanged', again.action === 'unchanged' && again.content === fresh.content);

const foreignTail = '#!/bin/sh\necho lint\n';
const appended = renderHookContent(foreignTail);
assert('foreign no-exec → appended at end', appended.action === 'appended'
  && appended.content.startsWith('#!/bin/sh\necho lint')
  && appended.content.includes(MARKER_BEGIN));

const foreignExec = '#!/bin/sh\necho pre\nexec husky run\n';
const beforeExec = renderHookContent(foreignExec);
const lines = beforeExec.content.split('\n');
assert('foreign exec → our block before exec', beforeExec.action === 'appended'
  && lines.indexOf(MARKER_BEGIN) < lines.findIndex((l) => /^\s*exec\s/.test(l)));

// real git repo install
const repo = mkdtempSync(join(tmpdir(), 'slopgate-hookinstall-'));
execSync('git init -q', { cwd: repo });
const r1 = installPreCommitHook(repo);
const hookPath = join(repo, '.git/hooks/pre-commit');
assert('install creates hook', r1.action === 'created' && existsSync(hookPath));
assert('hook is executable', (statSync(hookPath).mode & 0o100) !== 0);
const r2 = installPreCommitHook(repo);
assert('idempotent', r2.action === 'unchanged');

// core.hooksPath respected
const repo2 = mkdtempSync(join(tmpdir(), 'slopgate-hookpath-'));
execSync('git init -q', { cwd: repo2 });
mkdirSync(join(repo2, 'custom-hooks'));
execSync('git config core.hooksPath custom-hooks', { cwd: repo2 });
const r3 = installPreCommitHook(repo2);
assert('core.hooksPath used', r3.action === 'created' && existsSync(join(repo2, 'custom-hooks/pre-commit')));

rmSync(repo, { recursive: true, force: true });
rmSync(repo2, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
