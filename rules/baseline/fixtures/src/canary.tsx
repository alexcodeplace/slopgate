// Fixture for slop-gate self-test. Contains deliberate canary tokens.
// no-stubs SHOULD match stub markers (not bare placeholder UI/i18n tokens):
// placeholder for now
// TODO: implement later
// not implemented
export const x = __SLOPGATE_AST_CANARY__;
// negative: must NOT match no-stubs — placeholder={...}, placeholder: class, i18n keys
const _neg = <input placeholder={t('x')} className="placeholder:text-ink" />;
const _titleKey = admin.incidents.titlePlaceholder;
const namePlaceholder = 1;
// ts-suppress SHOULD match @ts-* only (not eslint-disable):
// @ts-ignore
/* eslint-disable zync/no-raw-html-in-pages */
function bad(el: HTMLElement) { el.innerHTML = '<b>hi</b>'; }