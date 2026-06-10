// Fixture for slop-gate self-test. Contains deliberate canary tokens.
// no-stubs SHOULD match stub markers (not JSX/Tailwind placeholder attr/class):
// placeholder for now
export const x = __SLOPGATE_AST_CANARY__;
// negative: must NOT match no-stubs — placeholder={...} and placeholder:text-...
const _neg = <input placeholder={x} className="placeholder:text-ink" />;
function bad(el: HTMLElement) { el.innerHTML = '<b>hi</b>'; }