import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readRows } from './store.mjs';

const BIN = fileURLToPath(new URL('../../bin/slopgate', import.meta.url));
const RULE_ID = 'no-stubs-placeholder';
const VIOLATION_LINE = 'export const a = 1; // placeholder for now\n';

function runSlopgate(args, { home, cwd }) {
  return execFileSync('node', [BIN, ...args], {
    encoding: 'utf8',
    cwd,
    env: { ...process.env, HOME: home, SLOPGATE_MODEL: 'wire-test-model' },
  });
}

function runSlopgateExpectExit1(args, { home, cwd }) {
  assert.throws(
    () => runSlopgate(args, { home, cwd }),
    (e) => e.status === 1,
  );
}

test('--staged block records rows to global + project stores; --file does not', () => {
  const home = mkdtempSync(join(tmpdir(), 'sg-record-home-'));
  const repo = mkdtempSync(join(tmpdir(), 'sg-record-repo-'));
  const configPath = join(repo, '.slopgate/config.mjs');
  const srcFile = join(repo, 'src/a.ts');
  const globalStats = join(home, '.slopgate', 'stats.jsonl');
  const projectStats = join(repo, '.slopgate', 'stats.jsonl');

  execFileSync('git', ['init', '-q'], { cwd: repo });
  execFileSync('git', ['config', 'user.email', 't@t'], { cwd: repo });
  execFileSync('git', ['config', 'user.name', 't'], { cwd: repo });

  mkdirSync(join(repo, '.slopgate'), { recursive: true });
  mkdirSync(join(repo, 'src'), { recursive: true });
  writeFileSync(configPath, `export default {
  roots: ['src'],
  baseline: ['no-stubs'],
  checkers: { 'diff-shape': { maxDirs: 5 } },
};\n`);
  writeFileSync(srcFile, VIOLATION_LINE);
  execFileSync('git', ['add', 'src/a.ts'], { cwd: repo });

  // --file blocks but must not append stats rows (recording is staged+exit-1 only)
  runSlopgateExpectExit1(['--file', 'src/a.ts', '--config', configPath], { home, cwd: repo });
  assert.equal(readRows(globalStats).length, 0);
  assert.equal(readRows(projectStats).length, 0);

  // --staged block writes one row per blocked violation to both stores
  runSlopgateExpectExit1(['--staged', '--config', configPath], { home, cwd: repo });

  const gRows = readRows(globalStats);
  const pRows = readRows(projectStats);
  assert.ok(gRows.length >= 1);
  assert.ok(pRows.length >= 1);

  for (const rows of [gRows, pRows]) {
    const hit = rows.find((r) => r.ruleId === RULE_ID);
    assert.ok(hit, `expected row with ruleId ${RULE_ID}`);
    assert.equal(hit.model, 'wire-test-model');
    assert.equal(hit.mode, 'staged');
  }
});
