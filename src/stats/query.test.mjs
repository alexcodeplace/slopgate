import { test } from 'node:test';
import assert from 'node:assert/strict';
import { aggregate, formatStats } from './query.mjs';

const rows = [
  { ts: '2026-01-01T00:00:00Z', ruleId: 'a', model: 'm1', project: 'p1', severity: 'high', engine: 'regex', category: 'c' },
  { ts: '2026-01-03T00:00:00Z', ruleId: 'a', model: 'm2', project: 'p1', severity: 'high', engine: 'ast', category: 'c' },
  { ts: '2026-01-02T00:00:00Z', ruleId: 'b', model: 'm1', project: 'p2', severity: 'low', engine: 'regex', category: 'd' },
];

test('aggregate by rule: counts + last/first seen, sorted by count desc', () => {
  const r = aggregate(rows, { by: 'rule' });
  assert.equal(r.total, 3);
  assert.equal(r.groups[0].key, 'a');
  assert.equal(r.groups[0].count, 2);
  assert.equal(r.groups[0].lastSeen, '2026-01-03T00:00:00Z');
  assert.equal(r.groups[0].firstSeen, '2026-01-01T00:00:00Z');
  assert.equal(r.lastSeen, '2026-01-03T00:00:00Z');
  assert.equal(r.firstSeen, '2026-01-01T00:00:00Z');
});

test('aggregate by model', () => {
  const r = aggregate(rows, { by: 'model' });
  assert.equal(r.groups.find((g) => g.key === 'm1').count, 2);
});

test('aggregate since filter (inclusive)', () => {
  const r = aggregate(rows, { by: 'rule', since: '2026-01-02T00:00:00Z' });
  assert.equal(r.total, 2);
});

test('aggregate unknown dimension throws', () => {
  assert.throws(() => aggregate(rows, { by: 'nope' }));
});

test('aggregate empty rows -> zeros', () => {
  const r = aggregate([], { by: 'rule' });
  assert.equal(r.total, 0);
  assert.equal(r.groups.length, 0);
  assert.equal(r.lastSeen, null);
});

test('aggregate missing dimension value -> "unknown" bucket', () => {
  const r = aggregate([{ ts: '2026-01-01T00:00:00Z' }], { by: 'model' });
  assert.equal(r.groups[0].key, 'unknown');
});

test('formatStats json', () => {
  assert.equal(JSON.parse(formatStats(aggregate(rows, { by: 'rule' }), { json: true })).total, 3);
});

test('formatStats text: header + rows', () => {
  const out = formatStats(aggregate(rows, { by: 'rule' }), {});
  assert.match(out, /3 incident\(s\) stopped/);
  assert.match(out, /RULE/);
  assert.match(out, /^a\b/m);
});

test('formatStats text empty', () => {
  assert.match(formatStats(aggregate([], { by: 'rule' }), {}), /0 incident\(s\) stopped/);
});
