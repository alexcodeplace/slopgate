// Fixtures for the UX module ast rules. Each construct below must trigger
// exactly the rule named in its comment (selftest asserts every yml fires).
// Parsed syntactically by ast-grep — no imports / type-checking involved.

// triggers ux-div-onclick — onClick on a non-semantic element, no role
export const ClickyDiv = () => <div onClick={onThing}>Open</div>;

// triggers ux-img-no-dimensions — <img> with alt but no width/height
export const Hero = () => <img src="/hero.png" alt="hero" />;

// triggers ux-img-no-alt — <img> with dimensions but no alt
export const NoAlt = () => <img src="/x.png" width={32} height={32} />;

// triggers ux-anchor-no-href — <a onClick> with no href
export const FakeLink = () => <a onClick={onThing}>go</a>;

// triggers ux-button-no-type — <button> with no explicit type
export const BareButton = () => <button onClick={onThing}>Save</button>;

// triggers ux-media-no-dimensions — <iframe> with no width/height
export const Embed = () => <iframe src="/player" />;

// triggers ux-async-onclick-no-disable — async onClick, no disabled (type set so
// it does NOT also trip ux-button-no-type)
export const AsyncSave = () => <button type="button" onClick={async () => save()}>Save</button>;

// triggers ux-modal-no-close — Dialog with no onClose/onDismiss/onOpenChange
export const Sheet = ({ open }) => <Dialog open={open}>body</Dialog>;

// negative: all of these are correct and must NOT trigger anything
export const Ok = () => (
  <>
    <button type="button" onClick={onThing}>Open</button>
    <button type="button" disabled onClick={async () => save()}>Save</button>
    <a href="/page" onClick={onThing}>real link</a>
    <img src="/ok.png" alt="ok" width={320} height={200} />
    <Dialog open onOpenChange={onThing}>ok</Dialog>
  </>
);

function onThing() {}
function save() {}
declare const Dialog: (props: Record<string, unknown>) => unknown;
