import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveAstGrepBin } from './ast-engine.mjs';

test('resolveAstGrepBin reports its source (local|path|null)', () => {
  const r = resolveAstGrepBin(process.cwd());
  if (r !== null) {
    assert.ok(['local', 'path'].includes(r.source));
    assert.equal(typeof r.bin, 'string');
  }
});
