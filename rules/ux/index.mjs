import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
export const UX_AST_DIR = join(__dirname, 'ast');

/**
 * UX module — opinionated UI-hygiene rules from the ANTI-SLOP UX framework.
 * OPTIONAL and opt-in per sub-module via the `ux:` config namespace:
 *
 *   ux: {
 *     a11y:    'high',      // enable + gate
 *     cls:     'high',
 *     taste:   'advisory',  // enable, report-only (maps to medium → non-gating)
 *     // omit a key  → sub-module OFF
 *   }
 *
 * Each sub-module declares: regex rules (line-scoped) + astIds (ids of the
 * ast-grep rules in ./ast that belong to it) + a defaultSeverity used when the
 * config value is `true`. A config value of 'advisory' aliases to 'medium'.
 *
 * @typedef {{ defaultSeverity: string, regex: import('../../src/config.mjs').Pattern[], astIds: string[] }} UxPack
 * @type {Record<string, UxPack>}
 */
export const UX_PACKS = {
  // ── a11y: semantic / interaction accessibility (ANTI-SLOP UX §11) ──
  a11y: {
    defaultSeverity: 'high',
    regex: [],
    astIds: ['ux-div-onclick'],
  },

  // ── cls: layout stability / Cumulative Layout Shift (ANTI-SLOP UX §13) ──
  cls: {
    defaultSeverity: 'high',
    regex: [],
    astIds: ['ux-img-no-dimensions'],
  },

  // ── taste: subjective cliché / microcopy / motion checks (§0/§6/§26) ──
  // Ships at MEDIUM (non-gating by default) — pure signal, low friction.
  taste: {
    defaultSeverity: 'medium',
    astIds: [],
    regex: [{
      id: 'ux-emoji-in-ui', title: 'Emoji used as UI icon / bullet',
      category: 'convention', severity: 'medium',
      pattern: '[\\u{1F300}-\\u{1FAFF}]|[\\u{2600}-\\u{26FF}]|[\\u{2700}-\\u{27BF}]',
      flags: 'u',
      description: 'Emoji as a section marker, bullet, or icon — generic AI-slop tell (ANTI-SLOP UX §0).',
      resolution: 'Use a real icon component or text label; reserve emoji for user content.',
      excludeGlobs: ['**/*.test.*', '**/*.spec.*', '**/*.md'],
      canary: 'const label = "🚀 Launch";',
      negativeCanary: ['const arrow = "->";', 'const x = 1;'],
    }, {
      id: 'ux-trusted-by-cliche', title: '"Trusted by" logo-strip cliché',
      category: 'convention', severity: 'medium',
      pattern: 'Trusted by',
      flags: 'i',
      description: 'Generic "Trusted by" social-proof strip under the hero — AI landing-page cliché (ANTI-SLOP UX §0).',
      resolution: 'Show concrete, specific proof (a real case study, metric, or quote) or drop it.',
      excludeGlobs: ['**/*.test.*', '**/*.spec.*'],
      canary: '<p>Trusted by 10,000 teams</p>',
      negativeCanary: ['const trustedByPolicy = true;'],
    }, {
      id: 'ux-lorem-ipsum', title: 'Lorem ipsum placeholder copy',
      category: 'convention', severity: 'medium',
      pattern: 'lorem ipsum',
      flags: 'i',
      description: 'Generic Lorem ipsum in a functional draft — never tests real text wrapping (ANTI-SLOP UX §6).',
      resolution: 'Use domain-specific, realistic placeholder copy.',
      canary: '<p>Lorem ipsum dolor sit amet</p>',
    }, {
      id: 'ux-robotic-microcopy', title: 'Robotic / generic microcopy',
      category: 'convention', severity: 'medium',
      pattern: '\\b(?:Submit Data|Click [Hh]ere)\\b',
      description: 'Robotic placeholder microcopy instead of human, contextual labels (ANTI-SLOP UX §6).',
      resolution: 'Write what the action does: "Save changes", "Send invite" — not "Submit Data".',
      canary: '<button>Submit Data</button>',
      negativeCanary: ['submitData();', 'const clickHandler = 1;'],
    }, {
      id: 'ux-heavy-drop-shadow', title: 'Heavy floating drop-shadow card',
      category: 'convention', severity: 'medium',
      pattern: '\\bshadow-2xl\\b',
      description: 'Floating card with a heavy drop shadow — generic AI-slop aesthetic (ANTI-SLOP UX §0).',
      resolution: 'Use a subtle elevation token or a hairline border for separation.',
      canary: '<div className="card shadow-2xl">',
      negativeCanary: ['<div className="shadow-sm">'],
    }, {
      id: 'ux-linear-easing', title: 'Linear easing on UI motion',
      category: 'convention', severity: 'medium',
      pattern: '\\bease-linear\\b|timing-function:\\s*linear\\b|easing:\\s*[\'"]linear[\'"]',
      description: 'Linear easing reads as robotic — UI motion should use spring/bezier curves (ANTI-SLOP UX §26).',
      resolution: 'Use ease-out / a custom cubic-bezier / spring physics; keep motion 200–300ms.',
      canary: '<div className="transition ease-linear">',
      negativeCanary: ['className="transition ease-in-out"', "transition-timing-function: cubic-bezier(0.4, 0, 0.2, 1)"],
    }],
  },
};

/** Aliases accepted as `ux:` values; 'advisory' = enable but never gate. */
const SEVERITY_ALIAS = { advisory: 'medium', report: 'medium' };

/** Resolve a `ux:` config value → concrete severity, or null if disabled. */
export function resolveUxSeverity(value, pack) {
  if (value == null || value === false) return null;
  if (value === true) return pack.defaultSeverity;
  return SEVERITY_ALIAS[value] ?? value;
}

/** All ast rule ids owned by the UX module (enabled or not) — used to gate them. */
export const UX_ALL_AST_IDS = Object.values(UX_PACKS).flatMap((p) => p.astIds);
