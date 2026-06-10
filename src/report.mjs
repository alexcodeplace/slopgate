// src/report.mjs
export function printGateReport(violations, mode, { baselinedCount = 0 } = {}) {
  const R = '\x1b[31m'; const Y = '\x1b[33m'; const B = '\x1b[1m'; const D = '\x1b[2m'; const Z = '\x1b[0m';
  const title = mode === 'file'
    ? 'SLOP-GATE — VIOLATIONS IN EDITED FILE               '
    : 'VIOLATIONS IN STAGED FILES — COMMIT BLOCKED         ';
  process.stderr.write(`\n${B}${R}╔═ SLOP-GATE ═════════════════════════════════════════╗${Z}\n`);
  process.stderr.write(`${B}${R}║ ${title}║${Z}\n`);
  process.stderr.write(`${B}${R}╚═════════════════════════════════════════════════════╝${Z}\n\n`);

  const order = (v) => (v.engine ?? 'regex');
  const sorted = [...violations].sort((a, b) => order(a).localeCompare(order(b)) || a.file.localeCompare(b.file) || a.line - b.line);
  let currentGroup = null;
  for (const v of sorted) {
    const group = order(v);
    if (group !== currentGroup) {
      currentGroup = group;
      process.stderr.write(`${B}── ${group} ──${Z}\n`);
    }
    const C = v.severity === 'critical' ? R : Y;
    process.stderr.write(`${B}${C}[${v.severity.toUpperCase()}]${Z} ${B}${v.id}${Z} ${D}${v.file}:${v.line}${Z}\n`);
    process.stderr.write(`  ${D}×${Z} ${v.text}\n`);
    process.stderr.write(`  ${B}✓${Z} ${v.resolution}\n\n`);
  }
  const files = new Set(violations.map((v) => v.file)).size;
  const tail = mode === 'file' ? 'Fix now while context is hot.' : 'Fix → retry commit.';
  process.stderr.write(`${B}${violations.length} violation(s) in ${files} file(s). ${tail}${Z}\n`);
  if (baselinedCount > 0) process.stderr.write(`${D}${baselinedCount} pre-existing (baselined) violation(s) ignored.${Z}\n`);
  process.stderr.write(`False positive? NEVER edit suppressions.json yourself — ask the user via AskUserQuestion.\n\n`);
}
