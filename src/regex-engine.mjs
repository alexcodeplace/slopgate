import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { listSourceFiles } from './enumerate.mjs';
import { lineHash } from './suppressions.mjs';

function pathMatchesGlobs(filePath, globs) {
  if (!globs?.length) return false;
  return globs.some((g) => {
    const norm = g.replace(/\*\*/g, '§').replace(/\*/g, '[^/]*').replace(/§/g, '.*');
    return new RegExp(norm + '$').test(filePath);
  });
}

function searchPattern(config, files, pattern, excludeGlobs) {
  const re = new RegExp(pattern);
  const byFile = new Map();
  for (const file of files) {
    if (pathMatchesGlobs(file, excludeGlobs)) continue;
    const lines = readFileSync(join(config.repoRoot, file), 'utf8').split('\n');
    for (let i = 0; i < lines.length; i++) {
      if (re.test(lines[i])) {
        if (!byFile.has(file)) byFile.set(file, []);
        byFile.get(file).push(i + 1);
      }
    }
  }
  return byFile;
}

/**
 * @param {import('./config.mjs').ResolvedConfig} config
 * @param {{ staged?:boolean, file?:string }} opts
 * @returns {{ id:string, severity:string, category:string, resolution:string, files:string[], byFile:Map }[]}
 */
export function runPatternScan(config, opts = {}) {
  const files = listSourceFiles(config, opts);
  const fileMode = !!opts.file;
  const findings = [];
  for (const p of config.patterns) {
    if (fileMode && (p.minFiles ?? 1) > 1) continue; // cross-file thresholds meaningless on one file
    let byFile;
    try { byFile = searchPattern(config, files, p.pattern, p.excludeGlobs); }
    catch { continue; }
    const hitFiles = [...byFile.keys()].sort();
    if (hitFiles.length < (p.minFiles ?? 1)) continue;
    findings.push({ id: p.id, severity: p.severity, category: p.category, resolution: p.resolution, files: hitFiles, byFile });
  }
  return findings;
}

/** Expand findings into per-line violations (critical/high gated upstream). */
export function collectRegexViolations(config, findings) {
  const violations = [];
  for (const f of findings) {
    const re = new RegExp(config.patterns.find((p) => p.id === f.id).pattern);
    for (const file of f.files) {
      const lines = readFileSync(join(config.repoRoot, file), 'utf8').split('\n');
      for (let i = 0; i < lines.length; i++) {
        if (re.test(lines[i])) {
          violations.push({
            id: f.id, severity: f.severity, category: f.category, file, line: i + 1,
            lineHash: lineHash(lines[i]),
            text: lines[i].trim().slice(0, 90),
            resolution: f.resolution, engine: 'regex',
          });
        }
      }
    }
  }
  return violations;
}