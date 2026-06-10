// src/checkers/diff-shape.test.mjs
import diffShape, { concernGroups } from './diff-shape.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const rootsRel = ['src', 'workers/api/src'];
const groups = concernGroups(
  ['src/features/auth/a.ts', 'src/features/auth/b.ts', 'src/server/db.ts', 'workers/api/src/index.ts', 'README.md'],
  rootsRel,
);
assert('groups by root + first segment', groups.has('src/features') && groups.has('src/server') && groups.has('workers/api/src/(root)') === true);
assert('file directly under root → (root) group', concernGroups(['workers/api/src/index.ts'], rootsRel).has('workers/api/src/(root)'));
assert('non-root files ignored', !([...groups].some((g) => g.includes('README'))));
assert('group count', groups.size === 3);

const config = { rootsRel: ['src'] };
const wide = ['a', 'b', 'c', 'd', 'e', 'f'].map((d) => `src/${d}/x.ts`);
const r1 = diffShape.run(config, {}, { files: wide, mode: 'staged' });
assert('6 areas > default 5 → one violation', r1.violations.length === 1 && r1.violations[0].id === 'diff-shape-mixed-concerns');
assert('severity/category', r1.violations[0].severity === 'high' && r1.violations[0].category === 'hygiene');
const r2 = diffShape.run(config, {}, { files: wide.slice(0, 5), mode: 'staged' });
assert('5 areas ≤ 5 → clean', r2.violations.length === 0);
const r3 = diffShape.run(config, { maxDirs: 2 }, { files: wide.slice(0, 3), mode: 'staged' });
assert('maxDirs configurable', r3.violations.length === 1);
const r4 = diffShape.run(config, {}, { files: wide, mode: 'full' });
assert('full mode → never fires', r4.violations.length === 0);
assert('detect always available', diffShape.detect(config, {}).available === true);
assert('id', diffShape.id === 'diff-shape');

process.exit(failed ? 1 : 0);
