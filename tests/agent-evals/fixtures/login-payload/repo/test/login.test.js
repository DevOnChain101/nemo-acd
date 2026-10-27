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

test('login returns a token', () => withServer(async (url) => {
  assert.equal(await login(url, 'alice', 'secret'), 'tok-alice');
}));

test('wrong password returns null', () => withServer(async (url) => {
  assert.equal(await login(url, 'alice', 'nope'), null);
}));
