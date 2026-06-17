---
name: slopgate-ux
description: Inject the ANTI-SLOP UX framework as design directives before generating or modifying any UI. Enforces complete states, action hierarchy, feedback loops, accessibility, performance, and human-centric microcopy — the semantic UX rules a static scanner cannot catch. Use before building or editing components, pages, forms, modals, tables, or any user-facing interface. Complements the slopgate `ux:{}` static rule module (which gates the mechanical subset).
---

# slopgate-ux — Anti-Slop UX Directives

Prompt-time companion to the slopgate `ux:{}` static module. The static module gates the *mechanical*
UX slop a scanner can see (`<div onClick>`, `<img>` without dimensions, emoji-as-icon, magic px/hex,
linear easing). **This skill carries the semantic half** — the cross-component, dataflow, and judgment
directives no regex/AST rule can enforce. Run the checklist **silently before outputting UI code**; if any
answer is NO, revise before emitting.

> Mission: generate human-centric, intuitive, **complete** interfaces. Avoid "UX slop": lazy, incomplete,
> or cognitively overwhelming designs optimized only for the happy path.

## Step 0 — Ensure gate initialized (run FIRST)

A `/slopgate-*` skill MUST NOT operate on an un-gated repo. Verify the gate; init if absent, so the `ux:{}` static module backs these prompt-time directives. Idempotent — `slopgate init` preserves an existing config and only fills what is missing (it also installs the fail-closed pre-commit hook). Single source of truth = the CLI's own idempotency; this step just calls it.

```bash
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "slopgate: not a git repo — cannot init gate"; exit 1; }
if [ ! -f "$ROOT/.slopgate/config.toml" ]; then
  echo "slopgate: gate not initialized — running 'slopgate init' first"
  ( cd "$ROOT" && slopgate init ) || { echo "slopgate: init failed"; exit 1; }
fi
```

`config.toml` present → proceed with the checklist below.

## 0. Generic-AI-slop alarm bells — reject on sight
Centered hero + two stacked CTAs over a gradient · "Trusted by" logo strip under the hero · bento-box
grids (3–4 icon/heading/paragraph cards) · emoji as bullets/section markers/icons · decorative
Lucide/Heroicons filling empty space · floating image cards with heavy drop shadows.
*(emoji / Trusted-by / drop-shadow are also caught statically by `ux:taste`.)*

## 1. Four states (complete states)
Every data-driven view implements **all four**: **Empty** (one sentence, one action, no illustration,
written like a person speaks) · **Loading** (skeleton blocks matching final layout, no shimmer, never
freeze the UI) · **Error** (friendly, actionable, with retry) · **Populated** (happy path).

## 2. Hierarchy protocol (no "button soup")
ONE primary (solid) action per view/section/form. Secondary = outline, tertiary = ghost/text. Tuck
low-priority actions in kebab menus. Destructive = visually distinct (red) + confirmation.

## 3. Action–reaction (feedback loop)
Every interaction → immediate visual feedback. Distinct hover/focus/active states. During async: disable
the trigger + inline loading indicator. Completion: explicit success/error toast or inline alert.
*(async-button-without-disable is partially catchable; the rest is judgment.)*

## 4. Friction = risk
Low-stakes/high-frequency → zero friction (single click). High-stakes/low-frequency → intentional
friction (confirm modal, or type "DELETE").

## 5. Cognitive load (progressive disclosure)
Group into cards/sections. Tabs/accordions/modals/drill-downs for secondary info. ≤5–7 primary
actions or major data points per glance.

## 6. Microcopy
Human, conversational ("Save changes", not "Submit Data"). No Lorem ipsum in functional drafts —
generate realistic, domain-specific placeholder data to test real text wrapping.

## 7. Defensive forms
Validate on type/blur, not only on submit. Accept natural formatting (spaces/dashes in phone/CC),
strip on the backend. Show input rules up front; don't make the user guess and fail.

## 8. Spatial & touch ergonomics
Min 44×44px touch target regardless of icon size. Physically separate destructive actions (Delete,
Cancel) from progression actions (Save, Next).

## 9. Data density (anti-scroll)
Any list/table >20 items → pagination / infinite scroll / "Load More". Complex tables get ≥2 parsing
tools (search, column sort, filter).

## 10. Wayfinding (no dead ends)
Every screen/modal/overlay has explicit Cancel/Close/Back — never rely on browser back. Global nav
highlights current location. Nesting >2 levels → breadcrumbs. Use an existing breadcrumb primitive;
if none exists, **create the primitive first** (never inline ad-hoc breadcrumbs).

## 11. Functional accessibility
Semantic HTML first (`<button>`, `<nav>`, `<dialog>`) over `div`+handler. Visible `:focus` on every
interactive element; logical tab order. Text contrast ≥12:1 (no WCAG-minimum greys). Hairline borders
within 0.04 L of their surface. Accent color ≤ once per section.
*(`<div onClick>` is gated statically by `ux:a11y`.)*

## 12. State-machine integrity
Debounce/throttle high-frequency inputs (search, slider). Mutex: once a mutating action fires (submit),
lock the whole form until the promise resolves — prevent double-submit.

## 13. Perceived performance
Optimistic updates for high-confidence reversible actions (favorite heart); roll back on failure.
Zero CLS: reserve space via aspect-ratio / fixed min-height so the page never jumps.
*(`<img>` without dimensions is gated statically by `ux:cls`.)*

## 14. URL truth (the refresh test)
Sync view modifiers — active tab, page index, search query, filters — to URL query params
(`?tab=billing&page=2`). A copied URL must reproduce the exact view for a colleague.

## 15. Tokenization over hardcoding
Never hardcode hex or px. Use design tokens / CSS vars / utility scale (`var(--color-primary)`,
`mt-4`). *(raw hex + magic px gated statically by baseline `raw-hex`.)*

## 16. Advanced a11y
Trap focus inside an open modal; return focus to the opener on close. Wrap dynamic announcements
(toasts, "5 results") in `aria-live="polite"`.

## 17–26 — Psychology, flow, ethics
17 **Undo over permission**: for mid-level destructive acts, execute + show "Undo" toast; hard-confirm
only catastrophic/irreversible ones. · 18 **Command palette** (Cmd/Ctrl+K) + visible shortcuts for SaaS.
· 19 **Local-first**: treat offline as normal — queue mutations, subtle "saving…will sync" indicator,
keep cached data interactive. · 20 **Predictive defaults**: infer timezone, default date ranges,
`autoFocus` the key input. · 21 **Fitts's law**: reveal row/card actions inline on hover; place context
menus at the cursor. · 22 **Emotional tone**: never celebrate destructive/stressful actions (no confetti
on "account deleted"); keep them somber. · 23 **2 AM rule**: scannable, bold key metrics, icons+text;
never rely on subtle color shifts. · 24 **Chronological grace**: auto-save any >30s process; restore
unsubmitted state on return. · 25 **Symmetrical effort**: undoing/cancelling costs the same friction as
doing/starting (no roach-motels). · 26 **Organic motion**: never `linear` easing — spring/bezier,
200–300ms. *(linear easing gated statically by `ux:taste`.)*

---

## Execution checklist (run silently before outputting code)
- [ ] **State & data** — Empty / Loading / Error handled? Large lists paginated?
- [ ] **Hierarchy & spatial** — exactly ONE primary action? Touch targets ≥44×44px?
- [ ] **Feedback & mutex** — immediate feedback? Forms locked during submit?
- [ ] **Performance** — optimistic where sensible? CLS prevented (reserved space)?
- [ ] **Routing** — tabs/filters synced to URL (passes the refresh test)?
- [ ] **Scalability** — design tokens, not magic numbers?
- [ ] **A11y & focus** — semantic elements? Focus trapped in modals + returned on close? Dynamic alerts via `aria-live`?
- [ ] **Human empathy** — emotional tone fits the action? Respects the 2 AM rule? Forms auto-save? Motion uses organic easing?

**If any answer is NO, revise before emitting the final code.**
