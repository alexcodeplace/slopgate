export function Swallow({ fn }: { fn: () => void }) {
  try { fn(); } catch (e) {}
  return <div />;
}
