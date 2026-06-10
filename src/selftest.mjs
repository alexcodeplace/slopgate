import { runAstGrepScan } from './ast-engine.mjs';

/** @param {import('./config.mjs').ResolvedConfig} config */
export function runSelfTest(config) {
  let failed = 0;
  for (const p of config.patterns) {
    if (!p.canary) { console.error(`WARN ${p.id}: no canary — cannot prove rule still fires`); continue; }
    let re;
    try { re = new RegExp(p.pattern, p.flags || undefined); } catch (e) { console.error(`FAIL ${p.id}: regex invalid: ${e}`); failed++; continue; }
    if (!re.test(p.canary)) { console.error(`FAIL ${p.id}: canary not matched: ${p.canary}`); failed++; }
    else console.error(`OK ${p.id}`);
    for (const neg of [].concat(p.negativeCanary ?? [])) {
      if (re.test(neg)) { console.error(`FAIL ${p.id}: negative canary matched: ${neg}`); failed++; }
      else console.error(`OK ${p.id} (negative)`);
    }
  }
  const ast = runAstGrepScan(config, null);
  if (!ast.available) {
    console.error(`WARN ast-grep unavailable — bucket-B self-test skipped: ${ast.errors.join('; ')}`);
  } else if (!ast.violations.some((v) => v.id === 'slopgate-canary')) {
    console.error('FAIL ast-grep canary: slopgate-canary did not fire on fixtures'); failed++;
  } else {
    console.error(`OK ast-grep canary (${ast.violations.length} fixture violations)`);
  }
  return failed ? 1 : 0;
}