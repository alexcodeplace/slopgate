import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, appendFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { appendRow, readRows } from './store.mjs';

test('appendRow + readRows round-trips', () => {
  const p = join(mkdtempSync(join(tmpdir(), 'sg-store-')), 'stats.jsonl');
  appendRow(p, { ruleId: 'a', ts: '2026-01-01T00:00:00Z' });
  appendRow(p, { ruleId: 'b', ts: '2026-01-02T00:00:00Z' });
  const rows = readRows(p);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].ruleId, 'a');
  assert.equal(rows[1].ruleId, 'b');
});

test('readRows: missing file -> []', () => {
  assert.deepEqual(readRows(join(tmpdir(), 'sg-nope-dir', 'x.jsonl')), []);
});

test('readRows: skips malformed lines', () => {
  const p = join(mkdtempSync(join(tmpdir(), 'sg-store-')), 's.jsonl');
  appendRow(p, { ruleId: 'ok' });
  appendFileSync(p, 'not json\n');
  appendRow(p, { ruleId: 'ok2' });
  const rows = readRows(p);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].ruleId, 'ok');
  assert.equal(rows[1].ruleId, 'ok2');
});
