/** @type {Record<string, import('../../src/config.mjs').Pattern[]>} */
export const STACK_PACKS = {
  cloudflare: [
    {
      id: 'cf-env-spread-secrets',
      title: 'CF env object spread (drops secret bindings)',
      category: 'security', severity: 'critical',
      pattern: '\\{\\s*\\.\\.\\.env[,}\\s]',
      description: 'Spreading CF Workers env ({...env}) does NOT copy secret bindings — downstream receives undefined for secrets despite Object.keys showing them.',
      resolution: 'Copy required fields explicitly: { DATABASE_URL: env.DATABASE_URL, ... }. Never spread CF env.',
      canary: 'const e = {...env, strict: true};',
    },
    {
      id: 'waituntil-bare-method-ref',
      title: 'Bare .waitUntil method reference (loses this)',
      category: 'security', severity: 'high',
      // Matches .waitUntil NOT followed by: ( (call), . or [ (chain), ? (optional-chain), === / !== (equality), ) (truthiness check)
      // Lookbehind (?<!\?) excludes opts?.waitUntil (optional-chain property read).
      pattern: '(?<!\\?)\\.waitUntil(?!\\s*[(.?\\[]|\\s*===|\\s*!==|\\s*\\))',
      description: 'Passing cfCtx.waitUntil as a bare reference loses the this binding — workerd throws "Illegal invocation".',
      resolution: 'Arrow-wrap: (p) => cfCtx.waitUntil(p), or bind: cfCtx.waitUntil.bind(cfCtx).',
      canary: 'const wu = cfCtx.waitUntil;',
    },
    {
      id: 'process-env-access',
      title: 'Direct process.env access in CF Worker',
      category: 'security', severity: 'critical',
      pattern: 'process\\.env\\.',
      description: 'CF Worker env accessed via process.env — binding values never populate process.env at runtime.',
      resolution: 'Read env from the Workers fetch handler context or a typed env module. Never process.env in worker code.',
      excludeGlobs: ['**/*.config.*', '**/scripts/**'],
      canary: 'const url = process.env.DATABASE_URL;',
    },
  ],
};
