#!/usr/bin/env node
// Lists golden cases whose meta.json.reviewed_by is not an owner approval ("pending-owner-review ..."
// or null). Prints only: the exit code is always 0 so CI never fails on pending cases.
// Usage: node tools/golden-review-status.mjs [--quiet]
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'tests', 'golden');
const pending = [];
let total = 0;
for (const area of fs.readdirSync(root, { withFileTypes: true }).filter((d) => d.isDirectory())) {
  const ad = path.join(root, area.name);
  for (const c of fs.readdirSync(ad, { withFileTypes: true }).filter((d) => d.isDirectory())) {
    const mp = path.join(ad, c.name, 'meta.json');
    if (!fs.existsSync(mp)) continue;
    total++;
    const by = JSON.parse(fs.readFileSync(mp, 'utf8')).reviewed_by ?? null;
    if (by === null || String(by).startsWith('pending')) pending.push([`${area.name}/${c.name}`, by]);
  }
}
console.log(`golden review status: ${total} cases, ${pending.length} pending owner review`);
if (!process.argv.includes('--quiet')) for (const [id, by] of pending) console.log(`  ${id}  ${by ?? '(unset)'}`);
