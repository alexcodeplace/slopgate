// src/checkers/index.mjs
import tsc from './tsc.mjs';
import knip from './knip.mjs';
import jscpd from './jscpd.mjs';
import depcruise from './depcruise.mjs';
import typeCoverage from './type-coverage.mjs';
import diffShape from './diff-shape.mjs';

/** Commit-tier checkers, in execution order. Mutable on purpose: tests inject fakes. */
export const CHECKERS = [tsc, knip, jscpd, depcruise, typeCoverage, diffShape];
