import test from 'node:test';
import assert from 'node:assert/strict';
import { renderTotal, renderCart } from '../src/cart.js';

test('whole-dollar total', () => {
  assert.equal(renderTotal([{ name: 'pen', price: 5, qty: 2 }]), 'Total: $10.00');
});

test('cart lists lines then total', () => {
  assert.equal(renderCart([{ name: 'pen', price: 5, qty: 2 }]), 'pen x2\nTotal: $10.00');
});
