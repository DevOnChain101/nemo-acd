import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from '../src/server.js';

async function get(path) {
  const server = createServer();
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  try {
    const res = await fetch('http://127.0.0.1:' + server.address().port + path);
    return await res.json();
  } finally {
    server.closeAllConnections();
    await new Promise((r) => server.close(r));
  }
}

test('page 1 has the first ten items', async () => {
  const body = await get('/items?page=1');
  assert.deepEqual(body.items.map((i) => i.id), [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
  assert.equal(body.total, 25);
});
