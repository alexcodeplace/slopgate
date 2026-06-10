// src/regex-engine.test.mjs — locks the single-pass scanner + /g statefulness fix
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { scanRegex, compileLineRegex } from './regex-engine.mjs';

let failed = 0;
const ok = (c, m) => { if (c) console.error(`PASS: ${m}`); else { console.error(`FAIL: ${m}`); failed++; } };

// compileLineRegex strips stateful flags so .test() is repeatable on the same input
const re = compileLineRegex('x', 'gi');
ok(re.test('x') && re.test('x') && re.test('x'), 'compileLineRegex: g/y stripped, .test repeatable');
ok(re.ignoreCase === true, 'compileLineRegex: non-stateful flags (i) preserved');

// scanRegex: a g-flagged rule must hit EVERY matching line, not every other one
const dir = mkdtempSync(join(tmpdir(), 'slopgate-re-'));
try {
  mkdirSync(join(dir, 'src'), { recursive: true });
  writeFileSync(join(dir, 'src/a.ts'), 'BAD\nBAD\nBAD\nok\nBAD\n');
  const config = {
    repoRoot: dir,
    patterns: [{ id: 'bad', severity: 'high', category: 'x', resolution: 'fix', pattern: 'BAD', flags: 'g' }],
  };
  const v = scanRegex(config, ['src/a.ts'], { fileMode: false });
  ok(v.length === 4, `g-flag rule hits all 4 BAD lines (got ${v.length})`);
  ok(v.every((x) => x.engine === 'regex' && x.id === 'bad'), 'violations carry engine=regex + id');
  ok(v[0].line === 1 && v[1].line === 2 && v[2].line === 3 && v[3].line === 5, 'line numbers correct, ok line skipped');
} finally { rmSync(dir, { recursive: true, force: true }); }

process.exit(failed ? 1 : 0);
