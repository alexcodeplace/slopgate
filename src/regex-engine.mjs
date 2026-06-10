import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { lineHash } from './suppressions.mjs';

// g/y make RegExp.prototype.test stateful (lastIndex advances) → alternate-line
// matches when scanning line-by-line. Strip them for line scanning.
const STATEFUL_FLAGS = /[gy]/g;

/** Compile a rule's regex for line-by-line scanning (never stateful). */
export function compileLineRegex(pattern, flags) {
  const safe = (flags || '').replace(STATEFUL_FLAGS, '');
  return new RegExp(pattern, safe || undefined);
}

function pathMatchesGlobs(filePath, globs) {
  if (!globs?.length) return false;
  return globs.some((g) => {
    const norm = g
      .replace(/[.+^${}()|[\]\\]/g, '\\$&')
      .replace(/\*\*\//g, '\x00')
      .replace(/\*\*/g, '\x01')
      .replace(/\*/g, '[^/]*')
      .replace(/\x00/g, '(?:.*/)?')
      .replace(/\x01/g, '.*');
    return new RegExp('^' + norm + '$').test(filePath);
  });
}

/**
 * Single-pass regex scan: read each file once, test every pattern against every line,
 * apply per-pattern minFiles thresholds, expand surviving hits into violations.
 * @param {import('./config.mjs').ResolvedConfig} config
 * @param {string[]} files repo-relative source paths (already enumerated by the caller)
 * @param {{ fileMode?: boolean }} [opts] fileMode → drop cross-file (minFiles>1) rules
 * @returns {{ id:string, severity:string, category:string, file:string, line:number, lineHash:string, text:string, resolution:string, engine:'regex' }[]}
 */
export function scanRegex(config, files, { fileMode = false } = {}) {
  const compiled = [];
  for (const p of config.patterns) {
    if (fileMode && (p.minFiles ?? 1) > 1) continue; // cross-file thresholds meaningless on one file
    try { compiled.push({ p, re: compileLineRegex(p.pattern, p.flags) }); }
    catch { /* unparseable pattern: skip, mirrors prior swallow */ }
  }

  // pass 1: one read per file; record hits per pattern as file -> [{line, text}]
  const hits = new Map(); // patternId -> Map(file -> [{line, text}])
  for (const file of files) {
    let lines;
    try { lines = readFileSync(join(config.repoRoot, file), 'utf8').split('\n'); }
    catch { continue; }
    for (const { p, re } of compiled) {
      if (p.includeGlobs?.length && !pathMatchesGlobs(file, p.includeGlobs)) continue;
      if (pathMatchesGlobs(file, p.excludeGlobs)) continue;
      let perFile = null;
      for (let i = 0; i < lines.length; i++) {
        if (re.test(lines[i])) {
          (perFile ??= []).push({ line: i + 1, text: lines[i] });
        }
      }
      if (perFile) {
        let byFile = hits.get(p.id);
        if (!byFile) { byFile = new Map(); hits.set(p.id, byFile); }
        byFile.set(file, perFile);
      }
    }
  }

  // pass 2: minFiles threshold + expand to violations (no re-read, no re-test)
  const violations = [];
  for (const { p } of compiled) {
    const byFile = hits.get(p.id);
    if (!byFile || byFile.size < (p.minFiles ?? 1)) continue;
    for (const file of [...byFile.keys()].sort()) {
      for (const { line, text } of byFile.get(file)) {
        violations.push({
          id: p.id, severity: p.severity, category: p.category, file, line,
          lineHash: lineHash(text),
          text: text.trim().slice(0, 90),
          resolution: p.resolution, engine: 'regex',
        });
      }
    }
  }
  return violations;
}
