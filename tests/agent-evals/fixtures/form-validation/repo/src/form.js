import { isValidEmail } from './validate.js';

export function validateForm({ name, email }) {
  const errors = {};
  if (!name || !name.trim()) errors.name = 'Name is required';
  if (!isValidEmail(email)) errors.email = 'Enter a valid email address';
  return errors;
}
