import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { withTempDir } from './temp.mjs';

test('withTempDir creates a dir, passes it to fn, removes it after', async () => {
  let seen;
  const ret = await withTempDir('slopgate-test-', async (dir) => {
    seen = dir;
    assert.ok(existsSync(dir));
    return 42;
  });
  assert.equal(ret, 42);
  assert.equal(existsSync(seen), false);
});

test('withTempDir removes the dir even when fn throws', async () => {
  let seen;
  await assert.rejects(withTempDir('slopgate-test-', async (dir) => {
    seen = dir;
    throw new Error('boom');
  }), /boom/);
  assert.equal(existsSync(seen), false);
});
