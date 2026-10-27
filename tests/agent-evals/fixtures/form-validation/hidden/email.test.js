import test from 'node:test';
import assert from 'node:assert/strict';
import { isValidEmail } from '../src/validate.js';
import { validateForm } from '../src/form.js';

test('plus addressing is valid', () => {
  for (const v of ['a+b@x.io', 'first+tag@sub.example.co.uk', ' a+b@x.io ']) assert.equal(isValidEmail(v), true, v);
});

test('still rejects garbage', () => {
  for (const v of ['a+b@', '@x.io', 'a+b@x', 'a +b@x.io', 'a+b@x..io']) assert.equal(isValidEmail(v), false, v);
});

test('form accepts plus addressing', () => {
  assert.deepEqual(validateForm({ name: 'Ann', email: 'a+b@x.io' }), {});
});
