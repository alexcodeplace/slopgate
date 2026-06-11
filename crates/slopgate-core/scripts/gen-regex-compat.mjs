#!/usr/bin/env node
/** One-off: emit regex_compat.json from JS compileLineRegex oracle over all embedded patterns. */
import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { compileLineRegex } from '../../../src/regex-engine.mjs';
import { BASELINE_PACKS } from '../../../rules/baseline/index.mjs';
import { STACK_PACKS } from '../../../rules/stack/index.mjs';
import { UX_PACKS } from '../../../rules/ux/index.mjs';

const __dirname = dirname(fileURLToPath(import.meta.url));
const out = join(__dirname, '../tests/parity_vectors/regex_compat.json');

function collectPatterns() {
  const out = [];
  for (const patterns of Object.values(BASELINE_PACKS)) out.push(...patterns);
  for (const patterns of Object.values(STACK_PACKS)) out.push(...patterns);
  for (const pack of Object.values(UX_PACKS)) out.push(...pack.regex);
  return out;
}

function casesFor(p) {
  const re = compileLineRegex(p.pattern, p.flags);
  const lines = [];
  if (p.canary) lines.push(p.canary);
  for (const n of p.negativeCanary ?? []) lines.push(n);
  // Ensure at least one match and one non-match per pattern.
  const seen = new Set();
  const cases = [];
  for (const line of lines) {
    if (seen.has(line)) continue;
    seen.add(line);
    cases.push({ line, match: re.test(line) });
  }
  const filler = 'const __slopgate_compat_negative__ = 0;';
  if (!cases.some((c) => c.match)) {
    throw new Error(`pattern ${p.id}: no matching canary`);
  }
  if (!cases.some((c) => !c.match)) {
    cases.push({ line: filler, match: re.test(filler) });
  }
  if (!cases.some((c) => !c.match)) {
    throw new Error(`pattern ${p.id}: could not derive a non-match case`);
  }
  return cases.slice(0, 4);
}

const vector = collectPatterns().map((p) => ({
  id: p.id,
  pattern: p.pattern,
  flags: p.flags ?? '',
  cases: casesFor(p),
}));

writeFileSync(out, JSON.stringify(vector, null, 2) + '\n');
console.log(`wrote ${vector.length} patterns → ${out}`);
