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
    {
      id: 'cf-getCloudflareContext-banned',
      title: 'getCloudflareContext() bypasses Astro runtime env',
      category: 'boundary',
      severity: 'high',
      pattern: '\\bgetCloudflareContext\\s*\\(',
      description: 'Calling getCloudflareContext() from @opennextjs/cloudflare reads a module-level context snapshot — request-scoped bindings (secrets, KV, DO) may be stale or missing. Real bug: commit 36eebfc7.',
      resolution: 'In Astro pages/components use Astro.locals.runtime.env; in Workers use the fetch handler env parameter.',
      excludeGlobs: ['**/*.test.*', '**/*.spec.*', '**/scripts/**'],
      canary: 'const ctx = await getCloudflareContext();',
    },
    {
      id: 'hono-env-direct-access',
      title: 'Hono c.env access without optional chain (crashes in Hono tests)',
      category: 'convention',
      severity: 'high',
      pattern: '\\bc\\.env\\.(?!\\?)|\\(c\\.env[^)]*\\)\\.[a-zA-Z_$]',
      description: 'In Hono test apps created with `new Hono()`, c.env is undefined — c.env.X or (c.env as T).X throws at runtime.',
      resolution: 'Use optional chaining: c.env?.MY_KEY or (c.env as T | undefined)?.MY_KEY.',
      includeGlobs: ['**/*.test.ts', '**/*.spec.ts', '**/*.test.tsx', '**/*.spec.tsx'],
      canary: '(c.env as { MY_KEY: string }).MY_KEY',
      negativeCanary: [
        'const k = c.env?.MY_KEY;',
        '(c.env as { K: string } | undefined)?.K',
      ],
    },
  ],
};
