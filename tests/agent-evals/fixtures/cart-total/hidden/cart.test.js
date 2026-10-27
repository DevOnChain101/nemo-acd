import test from 'node:test';
import assert from 'node:assert/strict';
import { renderTotal, cartTotalCents } from '../src/cart.js';

test('0.1 + 0.2 renders as 0.30', () => {
  assert.equal(renderTotal([{ name: 'a', price: 0.1, qty: 1 }, { name: 'b', price: 0.2, qty: 1 }]), 'Total: $0.30');
});

test('quantities and cents', () => {
  assert.equal(renderTotal([{ name: 'a', price: 19.99, qty: 3 }]), 'Total: $59.97');
  assert.equal(cartTotalCents([{ name: 'a', price: 19.99, qty: 3 }]), 5997);
});

test('empty cart', () => {
  assert.equal(renderTotal([]), 'Total: $0.00');
});
