import { fetch } from '../services/http';   // fetch is the service seam, not global
export function Shadowed() {
  const go = async () => await fetch('/x');  // must NOT flag — locally bound
  return <button onClick={go}>go</button>;
}
