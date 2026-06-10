import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const BIN = fileURLToPath(new URL('../../bin/slopgate', import.meta.url));

function runStats(args, env) {
  return execFileSync('node', [BIN, 'stats', ...args], { encoding: 'utf8', env: { ...process.env, ...env } });
}

function seedGlobal(home, rows) {
  const dir = join(home, '.slopgate');
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'stats.jsonl'), rows.map((r) => JSON.stringify(r)).join('\n') + '\n');
}

test('stats reads global store (no --config)', () => {
  const home = mkdtempSync(join(tmpdir(), 'sg-home-'));
  seedGlobal(home, [
    { ts: '2026-01-01T00:00:00Z', ruleId: 'no-stubs', model: 'm', project: 'p', severity: 'high', engine: 'regex', category: 'c' },
    { ts: '2026-01-02T00:00:00Z', ruleId: 'no-stubs', model: 'm', project: 'p', severity: 'high', engine: 'regex', category: 'c' },
  ]);
  const out = runStats([], { HOME: home });
  assert.match(out, /2 incident\(s\) stopped/);
  assert.match(out, /no-stubs/);
});

test('stats --by model', () => {
  const home = mkdtempSync(join(tmpdir(), 'sg-home-'));
  seedGlobal(home, [
    { ts: '2026-01-01T00:00:00Z', ruleId: 'r', model: 'opus', project: 'p', severity: 'high', engine: 'regex', category: 'c' },
  ]);
  const out = runStats(['--by', 'model'], { HOME: home });
  assert.match(out, /MODEL/);
  assert.match(out, /opus/);
});

test('stats --json', () => {
  const home = mkdtempSync(join(tmpdir(), 'sg-home-'));
  seedGlobal(home, [
    { ts: '2026-01-01T00:00:00Z', ruleId: 'r', model: 'm', project: 'p', severity: 'high', engine: 'regex', category: 'c' },
  ]);
  assert.equal(JSON.parse(runStats(['--json'], { HOME: home })).total, 1);
});

test('stats empty store -> 0 incidents, exit 0', () => {
  const home = mkdtempSync(join(tmpdir(), 'sg-home-'));
  assert.match(runStats([], { HOME: home }), /0 incident\(s\) stopped/);
});

test('stats unknown --by -> exit 2', () => {
  const home = mkdtempSync(join(tmpdir(), 'sg-home-'));
  assert.throws(
    () => runStats(['--by', 'bogus'], { HOME: home }),
    (e) => e.status === 2,
  );
});
