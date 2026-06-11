import { db } from '../db/client';
import axios from 'axios';
import { sql } from 'drizzle-orm';

export function UserCard({ id }: { id: string }) {
  const onClick = async () => {
    const res = await fetch(`/api/users/${id}`);     // raw transport — leak
    const rows = await db.query(`SELECT * FROM users`); // raw db — leak
    const q = sql`SELECT 1`;                           // inline query — leak
    return res;
  };
  return <button onClick={onClick}>load</button>;
}
