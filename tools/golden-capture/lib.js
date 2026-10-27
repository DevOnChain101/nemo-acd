'use strict';
const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');

// ---- plain-data conversion + stable JSON ---------------------------------
// Map -> object (string keys), Set -> sorted array, undefined/functions dropped, NaN/Infinity -> string.
function plain(v) {
  if (v === undefined || typeof v === 'function') return undefined;
  if (v === null || typeof v === 'string' || typeof v === 'boolean') return v;
  if (typeof v === 'number') return Number.isFinite(v) ? v : String(v);
  if (typeof v === 'bigint') return String(v);
  if (v instanceof Map) return plain(Object.fromEntries(v));
  if (v instanceof Set) return [...v].map(plain).sort((a, b) => (JSON.stringify(a) < JSON.stringify(b) ? -1 : 1));
  if (Buffer.isBuffer(v)) return { base64: v.toString('base64') };
  if (Array.isArray(v)) return v.map((x) => { const p = plain(x); return p === undefined ? null : p; });
  if (v instanceof Error) return { name: v.name, code: v.code, message: v.message };
  const out = {};
  for (const k of Object.keys(v).sort()) {
    const p = plain(v[k]);
    if (p !== undefined) out[k] = p;
  }
  return out;
}
const stable = (v) => JSON.stringify(plain(v), null, 2) + '\n';

function mapStrings(v, f) {
  if (typeof v === 'string') return f(v);
  if (Array.isArray(v)) return v.map((x) => mapStrings(x, f));
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [f(k), mapStrings(x, f)]));
  return v;
}

// ---- sandbox --------------------------------------------------------------
// <TMP>/home = fake $HOME (<HOME>), <TMP>/nemo_home (<NEMO_HOME>), <TMP>/ws = cwd, <TMP>/outside.
const UUID = /\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi;
const TS = /\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z\b/g;

function sandbox({ volatileTs = true } = {}) {
  const tmp = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'golden-')));
  const sb = { tmp, home: path.join(tmp, 'home'), nemoHome: path.join(tmp, 'nemo_home'), ws: path.join(tmp, 'ws'), outside: path.join(tmp, 'outside') };
  for (const d of [sb.home, sb.nemoHome, sb.ws, sb.outside]) fs.mkdirSync(d, { recursive: true });

  sb.expand = (v) => mapStrings(v, (s) => s.split('<NEMO_HOME>').join(sb.nemoHome).split('<HOME>').join(sb.home).split('<TMP>').join(tmp));
  const roots = [[sb.nemoHome, '<NEMO_HOME>'], [sb.home, '<HOME>'], [tmp, '<TMP>']];
  const reRoot = roots.map(([p, ph]) => [new RegExp(p.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '(?![\\w.-])', 'g'), ph]);
  sb.normalize = (v) => mapStrings(plain(v), (s) => {
    for (const [re, ph] of reRoot) s = s.replace(re, ph);
    s = s.replace(UUID, '<UUID>');
    return volatileTs ? s.replace(TS, '<TS>') : s;
  });

  // files: { path: string | {json} | {base64} }, dirs: [], symlinks: { link: target }
  // Files get mtimes BASE+i seconds in listing order so mtime-sorted tools are deterministic.
  sb.setup = ({ files = {}, dirs = [], symlinks = {} } = {}) => {
    for (const d of dirs) fs.mkdirSync(d, { recursive: true });
    Object.entries(files).forEach(([p, c], i) => {
      fs.mkdirSync(path.dirname(p), { recursive: true });
      if (c && typeof c === 'object' && 'base64' in c) fs.writeFileSync(p, Buffer.from(c.base64, 'base64'));
      else if (c && typeof c === 'object') fs.writeFileSync(p, JSON.stringify(c, null, 2));
      else fs.writeFileSync(p, c);
      const t = 1700000000 + i;
      fs.utimesSync(p, t, t);
    });
    for (const [l, t] of Object.entries(symlinks)) {
      fs.mkdirSync(path.dirname(l), { recursive: true });
      fs.symlinkSync(t, l);
    }
  };
  // Sorted listing of a directory tree (relative paths, utf8 content or base64 when binary).
  sb.tree = (root) => {
    const out = [];
    const walk = (dir) => {
      for (const name of fs.readdirSync(dir).sort()) {
        const p = path.join(dir, name);
        const rel = path.relative(root, p).split(path.sep).join('/');
        const st = fs.lstatSync(p);
        if (st.isSymbolicLink()) out.push({ path: rel, type: 'symlink', target: fs.readlinkSync(p) });
        else if (st.isDirectory()) { out.push({ path: rel, type: 'dir' }); walk(p); }
        else {
          const b = fs.readFileSync(p);
          out.push(b.includes(0) ? { path: rel, type: 'file', base64: b.toString('base64') } : { path: rel, type: 'file', content: b.toString('utf8') });
        }
      }
    };
    walk(root);
    return out;
  };
  sb.cleanup = () => fs.rmSync(tmp, { recursive: true, force: true });
  return sb;
}

// run fn; return { value } or { error: {name, code, message} } (never rethrows)
async function attempt(fn) {
  try {
    return { value: await fn() };
  } catch (e) {
    return { error: { name: e && e.name, code: e && e.code, message: e && e.message } };
  }
}

// sha256 over sorted relative paths + contents of the given dirs (LF-joined records)
function treeHash(root, dirs) {
  const h = crypto.createHash('sha256');
  const files = [];
  const walk = (d) => {
    for (const n of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, n.name);
      if (n.isDirectory()) walk(p);
      else if (n.isFile()) files.push(path.relative(root, p).split(path.sep).join('/'));
    }
  };
  for (const d of dirs) if (fs.existsSync(path.join(root, d))) walk(path.join(root, d));
  for (const f of files.sort()) {
    h.update(f + '\0');
    h.update(fs.readFileSync(path.join(root, f)));
    h.update('\0');
  }
  return h.digest('hex');
}

module.exports = { plain, stable, mapStrings, sandbox, attempt, treeHash };
