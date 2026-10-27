import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from '../src/server.js';

async function ids(path) {
  const server = createServer();
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  try {
    const res = await fetch('http://127.0.0.1:' + server.address().port + path);
    return (await res.json()).items.map((i) => i.id);
  } finally {
    server.closeAllConnections();
    await new Promise((r) => server.close(r));
  }
}

test('page 2 starts after page 1', async () => {
  assert.deepEqual(await ids('/items?page=2'), [11, 12, 13, 14, 15, 16, 17, 18, 19, 20]);
});

test('last partial page and beyond', async () => {
  assert.deepEqual(await ids('/items?page=3'), [21, 22, 23, 24, 25]);
  assert.deepEqual(await ids('/items?page=4'), []);
});

test('custom size', async () => {
  assert.deepEqual(await ids('/items?page=2&size=5'), [6, 7, 8, 9, 10]);
  assert.deepEqual(await ids('/items?page=1&size=5'), [1, 2, 3, 4, 5]);
});
