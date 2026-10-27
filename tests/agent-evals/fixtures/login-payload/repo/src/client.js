// Returns the session token, or null when the credentials are wrong.
export async function login(baseUrl, user, pass) {
  const res = await fetch(baseUrl + '/login', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ username: user, password: pass }),
  });
  if (res.status === 401) return null;
  if (!res.ok) throw new Error('login failed: ' + res.status);
  return (await res.json()).token;
}
