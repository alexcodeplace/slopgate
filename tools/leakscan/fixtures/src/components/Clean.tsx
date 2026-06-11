import { getUser } from '../services/users';
export function Clean({ id }: { id: string }) {
  const onClick = async () => { const u = await getUser(id); return u; };
  return <button onClick={onClick}>ok</button>;
}
