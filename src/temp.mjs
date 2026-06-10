import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/** Make a temp dir, pass it to fn (sync or async), always remove it. Returns fn's result. */
export async function withTempDir(prefix, fn) {
  const dir = mkdtempSync(join(tmpdir(), prefix));
  try { return await fn(dir); }
  finally { rmSync(dir, { recursive: true, force: true }); }
}
