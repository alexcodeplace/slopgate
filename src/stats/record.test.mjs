import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { readRows } from './store.mjs';
import { recordIncidents, resolveSession } from './record.mjs';

function withEnv(overrides, fn) {
  const saved = {};
  for (const k of Object.keys(overrides)) saved[k] = process.env[k];
  for (const [k, v] of Object.entries(overrides)) {
    if (v === undefined) delete process.env[k]; else process.env[k] = v;
  }
  try { return fn(); }
  finally {
    for (const [k, v] of Object.entries(saved)) {
      if (v === undefined) delete process.env[k]; else process.env[k] = v;
    }
  }
}

test('recordIncidents writes per-violation rows to global + project; ruleId=id; model from session file', () => {
  const home = mkdtempSync(join(tmpdir(), 'sg-home-'));
  withEnv({ HOME: home, SLOPGATE_MODEL: undefined }, () => {
    const repoRoot = mkdtempSync(join(tmpdir(), 'sg-repo-'));
    const configDir = join(repoRoot, '.slopgate');
    mkdirSync(configDir, { recursive: true });
    const config = { repoRoot, configDir };

    const key = createHash('sha256').update(repoRoot).digest('hex').slice(0, 16);
    const sdir = join(home, '.slopgate', 'sessions');
    mkdirSync(sdir, { recursive: true });
    writeFileSync(join(sdir, `${key}.json`), JSON.stringify({ model: 'claude-opus-4-8', sessionId: 's1' }));

    const violations = [
      { id: 'no-stubs', severity: 'high', category: 'conv', engine: 'regex', file: 'a.ts', line: 3 },
      { id: 'tsc-TS1', severity: 'critical', category: 'types', engine: 'checker:tsc', file: 'b.ts', line: 9 },
    ];
    assert.equal(recordIncidents(violations, config, { mode: 'staged' }), 2);

    const gRows = readRows(join(home, '.slopgate', 'stats.jsonl'));
    const pRows = readRows(join(configDir, 'stats.jsonl'));
    assert.equal(gRows.length, 2);
    assert.equal(pRows.length, 2);
    assert.equal(gRows[0].ruleId, 'no-stubs');
    assert.equal(gRows[0].model, 'claude-opus-4-8');
    assert.equal(gRows[0].sessionId, 's1');
    assert.equal(gRows[0].projectPath, repoRoot);
    assert.equal(gRows[0].mode, 'staged');
    assert.equal(gRows[1].engine, 'checker:tsc');
    assert.equal(typeof gRows[0].ts, 'string');
  });
});

test('resolveSession: env SLOPGATE_MODEL overrides file', () => {
  withEnv({ SLOPGATE_MODEL: 'manual-model' }, () => {
    assert.equal(resolveSession('/any/root').model, 'manual-model');
  });
});

test('resolveSession: no file -> unknown', () => {
  const home = mkdtempSync(join(tmpdir(), 'sg-home-'));
  withEnv({ HOME: home, SLOPGATE_MODEL: undefined }, () => {
    assert.equal(resolveSession('/no/session/here').model, 'unknown');
  });
});

test('recordIncidents: empty array -> 0, no throw', () => {
  assert.equal(recordIncidents([], { repoRoot: '/x', configDir: '/x/.slopgate' }, { mode: 'staged' }), 0);
});

test('recordIncidents: missing violation fields default to unknown/null', () => {
  const home = mkdtempSync(join(tmpdir(), 'sg-home-'));
  withEnv({ HOME: home, SLOPGATE_MODEL: 'm' }, () => {
    const repoRoot = mkdtempSync(join(tmpdir(), 'sg-repo-'));
    const configDir = join(repoRoot, '.slopgate');
    mkdirSync(configDir, { recursive: true });
    recordIncidents([{ id: 'x' }], { repoRoot, configDir }, { mode: 'staged' });
    const row = readRows(join(home, '.slopgate', 'stats.jsonl'))[0];
    assert.equal(row.severity, 'unknown');
    assert.equal(row.engine, 'unknown');
    assert.equal(row.file, null);
    assert.equal(row.line, null);
  });
});
