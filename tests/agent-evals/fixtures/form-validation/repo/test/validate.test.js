import test from 'node:test';
import assert from 'node:assert/strict';
import { isValidEmail } from '../src/validate.js';
import { validateForm } from '../src/form.js';

test('accepts plain addresses', () => {
  assert.equal(isValidEmail('a@x.io'), true);
  assert.equal(isValidEmail('john.doe@example.com'), true);
});

test('rejects malformed addresses', () => {
  for (const v of ['a@', 'no-at', 'a b@x.io', '', null]) assert.equal(isValidEmail(v), false, String(v));
});

test('form reports a missing name', () => {
  assert.deepEqual(validateForm({ name: '', email: 'a@x.io' }), { name: 'Name is required' });
});
