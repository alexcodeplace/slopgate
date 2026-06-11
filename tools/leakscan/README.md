# leakscan

Native (Rust + [oxc](https://oxc.rs)) detector for **leaky abstractions** — direct
database or external-API I/O inside presentation-layer files. Enforces Dependency
Inversion: components/pages depend on a service layer, never on the transport.

Why a binary and not a regex / ast-grep rule: leakscan parses each file to an AST
and **suppresses the global-call rule when the call name is locally bound**
(`import { fetch } from './api'`, `const fetch = wrap(...)`). That scope check is
the false-positive killer a pattern matcher can't express.

## Rules

| rule | severity | fires on (inside a presentation file) |
|------|----------|----------------------------------------|
| `banned-import-in-component` | high | `import ... from 'pg' \| 'drizzle-orm' \| 'axios' \| ...` |
| `raw-global-io-in-component` | high | global `fetch(...)` / `XMLHttpRequest(...)`, unless locally bound |
| `raw-db-call-in-component` | high | `x.query(...)`, `x.execute(...)`, `x.$queryRaw(...)`, ... |
| `inline-query-in-component` | medium | `` sql`...` `` tagged template |

A file is *presentation layer* when it matches `presentation_globs` and not
`exempt_globs` (services, data-access, tests — the allowed I/O seam).

## Build & run

```bash
cargo build --release
./target/release/leakscan [--config <file.json>] <root> [<root> ...]
```

Emits one JSON document on stdout: `{ violations: [...], scanned, errors }`.
Exit code is always 0 — the gate decides pass/fail from the JSON.

## Config

All lists default to sensible values; a JSON file overrides any of them (a
non-empty list **replaces** the default for that key):

```json
{
  "presentation_globs": ["**/src/{components,pages,app,ui}/**", "**/*.tsx"],
  "exempt_globs": ["**/services/**", "**/data/**", "**/*.test.*"],
  "banned_modules": ["pg", "drizzle-orm", "@prisma/client", "axios"],
  "db_methods": ["query", "execute", "raw"],
  "query_tags": ["sql"],
  "global_calls": ["fetch", "XMLHttpRequest"]
}
```

## slopgate integration

Wired as the `leakscan` checker (`src/checkers/leakscan.mjs`). Enable in
`.slopgate/config.mjs`:

```js
checkers: { leakscan: {} }   // or { leakscan: { rules: 'leakscan.json', timeout: 60 } }
```

The adapter resolves the binary from `cfg.bin` → `$LEAKSCAN_BIN` →
`tools/leakscan/target/release/leakscan` → `…/debug/leakscan`, and reads an
optional `.slopgate/leakscan.json` for project overrides.

## Test

```bash
cargo test --release   # integration tests over ./fixtures
```
