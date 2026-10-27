export function cartTotalCents(items) {
  return items.reduce((sum, i) => sum + Math.round(i.price * 100) * i.qty, 0);
}

export function renderTotal(items) {
  return `Total: $${(cartTotalCents(items) / 100).toFixed(2)}`;
}

export function renderCart(items) {
  const lines = items.map((i) => `${i.name} x${i.qty}`);
  return [...lines, renderTotal(items)].join('\n');
}
