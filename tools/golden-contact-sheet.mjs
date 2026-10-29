#!/usr/bin/env node
// Before/after contact sheet of the render goldens changed against a git revision: renders the
// ANSI rows of expected.ansi.json (HEAD vs the working tree) as HTML spans, side by side.
// Usage: node tools/golden-contact-sheet.mjs [--base REV] [--out FILE]
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repo = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const arg = (k, d) => { const i = process.argv.indexOf(k); return i > 0 ? process.argv[i + 1] : d; };
const base = arg('--base', 'HEAD');
const out = arg('--out', path.join(repo, 'docs/research/tui-p1-contact-sheet.html'));
const git = (...a) => execFileSync('git', a, { cwd: repo, encoding: 'utf8', maxBuffer: 1 << 28 });
const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

function rowToHtml(row) {
  let fg = null, bg = null, bold = false, dim = false, ital = false, ul = false, rev = false, html = '';
  const style = () => {
    let f = fg, b = bg;
    if (rev) [f, b] = [b ?? '#0b0c0f', f ?? '#c8ccd2'];
    return [f && `color:${f}`, b && `background:${b}`, bold && 'font-weight:700', dim && 'opacity:.6', ital && 'font-style:italic', ul && 'text-decoration:underline'].filter(Boolean).join(';');
  };
  const re = /\x1b\[([0-9;]*)m|([^\x1b]+)/g;
  let m;
  while ((m = re.exec(row))) {
    if (m[2] !== undefined) { html += `<span style="${style()}">${esc(m[2])}</span>`; continue; }
    const p = m[1] === '' ? [0] : m[1].split(';').map(Number);
    for (let i = 0; i < p.length; i++) {
      const n = p[i];
      if (n === 0) { fg = bg = null; bold = dim = ital = ul = rev = false; }
      else if (n === 1) bold = true; else if (n === 2) dim = true; else if (n === 3) ital = true;
      else if (n === 4) ul = true; else if (n === 7) rev = true; else if (n === 22) bold = dim = false;
      else if (n === 39) fg = null; else if (n === 49) bg = null;
      else if ((n === 38 || n === 48) && p[i + 1] === 2) {
        const c = `rgb(${p[i + 2]},${p[i + 3]},${p[i + 4]})`;
        if (n === 38) fg = c; else bg = c;
        i += 4;
      }
    }
  }
  return html;
}
const frame = (rows) => `<pre>${rows.map(rowToHtml).join('\n')}</pre>`;

const changed = git('diff', '--name-only', base, '--', 'tests/golden/render').split('\n').filter((f) => f.endsWith('expected.ansi.json'));
let cards = '';
for (const f of changed) {
  const id = path.basename(path.dirname(f));
  let before;
  try { before = JSON.parse(git('show', `${base}:${f}`)).lines; } catch { before = []; }
  const after = JSON.parse(fs.readFileSync(path.join(repo, f), 'utf8')).lines;
  const meta = JSON.parse(fs.readFileSync(path.join(repo, path.dirname(f), 'meta.json'), 'utf8'));
  const textChanged = before.map((l) => l.replace(/\x1b\[[0-9;]*m/g, '')).map((l, i) => (l !== after[i]?.replace(/\x1b\[[0-9;]*m/g, '') ? i : -1)).filter((i) => i >= 0);
  cards += `<section><h2>${esc(id)} <small>${esc(meta.reviewed_by ?? '')} &middot; text changed in rows: ${textChanged.length ? textChanged.join(', ') : 'none (colour only)'}</small></h2><div class="pair"><div><h3>before (${esc(base)})</h3>${frame(before)}</div><div><h3>after</h3>${frame(after)}</div></div></section>\n`;
}
const html = `<!doctype html><meta charset="utf-8"><title>TUI P1 contact sheet</title>
<style>body{background:#0b0c0f;color:#c8ccd2;font:13px system-ui;margin:16px}pre{font:12px/1.15 ui-monospace,Menlo,Consolas,monospace;background:#0b0c0f;border:1px solid #3a3f4a;padding:6px;margin:0;overflow:auto;color:#c8ccd2}
.pair{display:grid;grid-template-columns:1fr 1fr;gap:8px}h2{font-size:14px;margin:18px 0 4px}h3{font-size:12px;margin:2px 0;color:#8c929b}small{color:#8c929b;font-weight:400}</style>
<h1>TUI redesign P1: before/after of ${changed.length} rebaselined render goldens</h1>
<p>Rendered from <code>expected.ansi.json</code> (${esc(base)} vs working tree). Cases listed in <code>golden_divergences.txt</code> are not rebaselined in P1 and are not shown. Background of the pages is the reference terminal background #0B0C0F.</p>
${cards}`;
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, html);
console.log(`wrote ${out}: ${changed.length} cases`);
