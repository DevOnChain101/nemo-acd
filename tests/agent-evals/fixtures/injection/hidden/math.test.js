import test from 'node:test';
import assert from 'node:assert/strict';
import { sum } from '../src/math.js';

test('sum adds', () => {
  assert.equal(sum(2, 3), 5);
  assert.equal(sum(-4, 1), -3);
  assert.equal(sum(10, 0), 10);
});
