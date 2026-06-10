import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { applyGateFilters } from './gate.mjs';

test('applyGateFilters: severity-allow drops low, keeps high', () => {
  const config = { gate: { staged: ['critical','high'] }, suppressionsPath: '/nonexistent/suppressions.json' };
  const vs = [
    { id: 'a', severity: 'low', file: 'f.ts', lineHash: 'h1' },
    { id: 'b', severity: 'high', file: 'f.ts', lineHash: 'h2' },
  ];
  assert.deepEqual(applyGateFilters(vs, config, 'staged').map(v => v.id), ['b']);
});

test('suppression entry removes a matching high violation', () => {
  const d = mkdtempSync(join(tmpdir(), 'sg-seam-'));
  const p = join(d, 'suppressions.json');
  writeFileSync(p, JSON.stringify({ version: 1, entries: [{ id: 'b', file: 'f.ts', lineHash: 'h2' }] }));
  const out = applyGateFilters(
    [{ id: 'b', severity: 'high', file: 'f.ts', lineHash: 'h2' }],
    { gate: { staged: ['critical','high'] }, suppressionsPath: p }, 'staged');
  assert.equal(out.length, 0);
});

test('malformed suppressions.json treated as EMPTY (fail toward blocking)', () => {
  const d = mkdtempSync(join(tmpdir(), 'sg-seam-'));
  const p = join(d, 'suppressions.json');
  writeFileSync(p, '{ this is not json');
  const out = applyGateFilters(
    [{ id: 'b', severity: 'high', file: 'f.ts', lineHash: 'h2' }],
    { gate: { staged: ['critical','high'] }, suppressionsPath: p }, 'staged');
  assert.equal(out.length, 1);
});
