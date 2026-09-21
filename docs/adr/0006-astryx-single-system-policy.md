# ADR 0006: Add opt-in Astryx single-system policy pack

## Decision

Add an opt-in built-in stack pack named `astryx-single-system`. The pack contains critical regex policies that reject imports from competing UI/design systems and competing styling engines in projects that have selected Astryx as their sole design system. It does not alter Slopgate defaults: projects must explicitly enable the pack with `stack = ["astryx-single-system"]`. The rule is intentionally dependency/import oriented; project-specific component-boundary constraints remain project-owned rules.

## Compatibility

Existing Slopgate projects are unaffected unless they explicitly enable the new pack. The pack uses the existing regex-pack schema and scanner, so there is no protocol, configuration-format, adapter, baseline, or subprocess change. Astryx itself remains allowed, including direct Astryx imports; stricter projects may layer a project-owned rule that requires an adapter such as `@platform-modules/ui-primitives/astryx`. Existing project packs and ruleOverride semantics are unchanged.

## Performance

The pack adds two line-scoped regexes only when enabled. Both expressions are bounded import-specifier checks with include globs limited to ordinary JavaScript/TypeScript/Astro source. No additional processes, parsers, filesystem passes, or semantic adapters are introduced. Existing native performance budgets remain authoritative and must pass unchanged.

## Verification

The pack is covered by an explicit Rust unit test asserting both critical rules, positive canaries and negative canaries. Slopgate's canonical self-test config enables the pack so the shipped scanner exercises both canaries through the normal rule pipeline. Full workspace tests, architecture checks, clippy, formatting and hosted CI remain required. A consumer acceptance check in PDF2HTML will prove a competing design-system import is rejected while the selected Astryx/StyleX imports are accepted.

## Review authorization

The repository owner explicitly requested that this policy be added to Slopgate, wired into PDF2HTML's PR gate, self-reviewed, merged and deployed as part of the single-styling-system convergence. ADR 0004 authorizes owner-directed autonomous substantive review and merge after required CI. This record does not waive any protected check, architecture requirement or hosted review gate.
