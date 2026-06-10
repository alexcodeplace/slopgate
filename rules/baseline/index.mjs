import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
export const BASELINE_AST_DIR = join(__dirname, 'ast');
export const BASELINE_FIXTURES_DIR = join(__dirname, 'fixtures');

/** @type {Record<string, import('../../src/config.mjs').Pattern[]>} */
export const BASELINE_PACKS = {
  'no-stubs': [{
    id: 'no-stubs-placeholder', title: 'Stub / placeholder / not-implemented marker',
    category: 'convention', severity: 'critical',
    pattern: '(?:for now|in a real app|placeholder\\s+(?:for now|implementation|impl|until)\\b|TODO: ?implement|not implemented)',
    flags: 'i',
    description: 'Stub or deferred-work marker — global rule forbids stubs/placeholders/workarounds.',
    resolution: 'Implement the real behavior now; remove the placeholder.',
    canary: '// placeholder for now',
    negativeCanary: [
      "placeholder={t('x')}",
      'placeholder:text-ink',
      'admin.incidents.titlePlaceholder',
      'const namePlaceholder = 1',
    ],
  }],
  'ts-suppress': [{
    id: 'ts-suppress-added', title: 'TypeScript suppression directive',
    category: 'convention', severity: 'high',
    pattern: '(?:\\/\\/\\s*|\\/\\*\\s*)?@ts-(?:ignore|expect-error|nocheck)\\b',
    description: 'Suppressing the type checker instead of fixing the cause.',
    resolution: 'Fix the underlying type error; remove the suppression.',
    canary: '// @ts-expect-error',
    negativeCanary: ['/* eslint-disable zync/no-raw-html-in-pages */'],
  }],
  'as-any': [{
    id: 'as-any-cast', title: '`as any` cast',
    category: 'convention', severity: 'high',
    pattern: 'as any\\b',
    description: 'Escape-hatch cast that disables type safety.',
    resolution: 'Use a precise type or a discriminated narrowing.',
    canary: 'const x = foo as any;',
  }],
  'raw-hex': [{
    id: 'raw-hex-color', title: 'Hardcoded hex color',
    category: 'convention', severity: 'high',
    pattern: '#[0-9a-fA-F]{3,8}\\b',
    description: 'Raw hex color in source instead of a design token.',
    resolution: 'Use a CSS custom property / design token.',
    excludeGlobs: ['**/tokens.css', '**/tokens/**'],
    canary: 'color: #ff0044;',
  }],
  'kv-ban': [{
    id: 'kv-binding-usage', title: 'Cloudflare KV usage',
    category: 'boundary', severity: 'critical',
    pattern: 'env\\.KV\\b|KV_NAMESPACE|\\.kv\\.',
    description: 'KV is eventually-consistent; banned for stateful/read-after-write paths (global preference).',
    resolution: 'Use a Durable Object (strong consistency) or cache.default (read caching).',
    canary: 'await env.KV.put(k, v);',
  }],
};