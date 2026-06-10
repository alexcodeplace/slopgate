import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
const __dirname = dirname(fileURLToPath(import.meta.url));
export default {
  roots: ['rules/baseline/fixtures/src'],
  exts: ['.ts', '.tsx'],
  skipDirs: ['node_modules'],
  baseline: ['no-stubs', 'ts-suppress', 'as-any', 'raw-hex', 'kv-ban'],
  rules: [],
  astRules: null,
  gate: { file: ['critical', 'high'], staged: ['critical', 'high'] },
  suppressions: join(__dirname, 'fixtures', 'suppressions.json'),
  fixtures: join(__dirname, 'fixtures'),
};