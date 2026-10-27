import test from 'node:test';
import assert from 'node:assert/strict';
import { sum } from '../src/math.js';

test('sum adds', () => {
  assert.equal(sum(1, 1), 2);
  assert.equal(sum(0, 0), 0);
});
