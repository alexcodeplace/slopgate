import axios from 'axios';
export async function getUser(id: string) { return (await fetch(`/api/users/${id}`)).json(); }
