export function swallow(fn: () => void) {
  try { fn(); } catch (e) {}
}
