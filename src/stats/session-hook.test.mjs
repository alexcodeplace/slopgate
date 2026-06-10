import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const HOOK = fileURLToPath(new URL('../../hooks/session-start.sh', import.meta.url));

test('session-start hook writes model file keyed by sha256(gitRoot)[:16]', () => {
  const home = mkdtempSync(join(tmpdir(), 'sg-home-'));
  const root = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim();
  const input = JSON.stringify({ model: 'claude-opus-4-8', session_id: 'sess-1', cwd: root });
  execFileSync('bash', [HOOK], { input, env: { ...process.env, HOME: home } });

  const key = createHash('sha256').update(root).digest('hex').slice(0, 16);
  const p = join(home, '.slopgate', 'sessions', `${key}.json`);
  assert.ok(existsSync(p), 'session file written');
  const m = JSON.parse(readFileSync(p, 'utf8'));
  assert.equal(m.model, 'claude-opus-4-8');
  assert.equal(m.sessionId, 'sess-1');
  assert.equal(typeof m.startedAt, 'string');
});

test('session-start hook: malformed stdin -> model unknown, still exits 0', () => {
  const home = mkdtempSync(join(tmpdir(), 'sg-home-'));
  const root = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim();
  execFileSync('bash', [HOOK], { input: 'not json', env: { ...process.env, HOME: home } });
  const key = createHash('sha256').update(root).digest('hex').slice(0, 16);
  const m = JSON.parse(readFileSync(join(home, '.slopgate', 'sessions', `${key}.json`), 'utf8'));
  assert.equal(m.model, 'unknown');
});
