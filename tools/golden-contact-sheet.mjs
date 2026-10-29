#!/usr/bin/env node
// Contact sheet of the TUI redesign: (1) before/after of the rows of the render goldens that differ
// from a git revision (the header and the welcome greeting; identical pairs are shown once with the
// list of cases), (2) the default, Classic and Graphite themes side by side at 140x40 from the ANSI
// dumps of tools/once-frames.py. Rows are rendered from the SGR codes as HTML spans.
// Usage: node tools/golden-contact-sheet.mjs [--base REV] [--out FILE]
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repo = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const arg = (k, d) => { const i = process.argv.indexOf(k); return i > 0 ? process.argv[i + 1] : d; };
const base = arg('--base', '05063f4');
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

const strip = (l) => l.replace(/\x1b\[[0-9;]*m/g, '');
const changed = git('diff', '--name-only', base, '--', 'tests/golden/render').split('\n').filter((f) => f.endsWith('expected.ansi.json'));
// (before row, after row) -> cases; only rows whose text or colour changed
const groups = new Map();
for (const f of changed) {
  const id = path.basename(path.dirname(f));
  let before;
  try { before = JSON.parse(git('show', `${base}:${f}`)).lines; } catch { continue; }
  const after = JSON.parse(fs.readFileSync(path.join(repo, f), 'utf8')).lines;
  after.forEach((a, i) => {
    const b = before[i];
    if (b === undefined || b === a) return;
    // run structure alone is not a change: compare text plus the colour of every cell
    const cells = (r) => rowToHtml(r).replace(/<span style="([^"]*)">([^<]*)<\/span>/g, (_, st, tx) => [...tx].map((c) => `${st}|${c}`).join('\n'));
    if (cells(a) === cells(b)) return;
    const key = JSON.stringify([b, a]);
    if (!groups.has(key)) groups.set(key, { b, a, cases: [] });
    groups.get(key).cases.push(`${id}#${i}`);
  });
}
let cards = '';
for (const { b, a, cases } of groups.values()) {
  cards += `<section><h2><small>${esc(cases.slice(0, 6).join(', '))}${cases.length > 6 ? ` and ${cases.length - 6} more` : ''}</small></h2><h3>before (${esc(base)})</h3><pre>${rowToHtml(b)}</pre><h3>after</h3><pre>${rowToHtml(a)}</pre></section>\n`;
}
const dump = (n) => fs.readFileSync(path.join(repo, `docs/research/tui-p1-frames/themes/compare-140x40-${n}.ansi.txt`), 'utf8').replace(/\n$/, '').split('\n');
const themes = ['synthesis', 'classic', 'graphite'].map((n) => `<div><h3>${n}${n === 'synthesis' ? ' (default)' : ''}</h3>${frame(dump(n))}</div>`).join('');
const html = `<!doctype html><meta charset="utf-8"><title>TUI redesign contact sheet</title>
<style>body{background:#080e12;color:#c8ccd2;font:13px system-ui;margin:16px}pre{font:12px/1.15 ui-monospace,Menlo,Consolas,monospace;background:#080e12;border:1px solid #3a3f4a;padding:6px;margin:0;overflow:auto;color:#c8ccd2}
.themes{display:grid;grid-template-columns:1fr;gap:12px}h2{font-size:14px;margin:14px 0 4px}h3{font-size:12px;margin:6px 0 2px;color:#8c929b}small{color:#8c929b;font-weight:400}</style>
<h1>TUI redesign: header and greeting before/after, and the themes</h1>
<h2>1. Rows that changed in the render goldens (${groups.size} distinct, from ${changed.length} files; ${esc(base)} vs working tree)</h2>
<p>Only rows whose text or cell colours differ are shown, one pair for all cases that share it. Page background is the default theme's reference terminal background #080E12.</p>
${cards}
<h2>2. Themes at 140x40: synth --once, welcome frame</h2>
<div class="themes">${themes}</div>`;
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, html);
console.log(`wrote ${out}: ${groups.size} row pairs, ${(html.length / 1024).toFixed(0)} KiB`);
