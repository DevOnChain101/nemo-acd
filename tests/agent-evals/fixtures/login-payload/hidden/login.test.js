import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from '../src/server.js';
import { login } from '../src/client.js';

async function withServer(fn) {
  const server = createServer();
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  try {
    await fn('http://127.0.0.1:' + server.address().port);
  } finally {
    server.closeAllConnections();
    await new Promise((r) => server.close(r));
  }
}

test('second user logs in', () => withServer(async (url) => {
  assert.equal(await login(url, 'bob', 'hunter2'), 'tok-bob');
}));

test('wrong credentials still give null', () => withServer(async (url) => {
  assert.equal(await login(url, 'bob', 'secret'), null);
}));

test('server contract is unchanged', () => withServer(async (url) => {
  const r = await fetch(url + '/login', { method: 'POST', body: JSON.stringify({ user: 'bob', pass: 'hunter2' }) });
  assert.equal(r.status, 400);
}));
