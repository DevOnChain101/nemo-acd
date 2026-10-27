'use strict';
const { ref } = require('../env');
const path = require('path');
const fs = require('fs');
const { builtinTools, findTool } = require(path.join(ref, 'src/native/tools/index.js'));

// Only fs-only tools: Read, Write, Edit, Glob, Grep. Grep always uses the pure-JS fallback (ctx.forceJsGrep) so the
// result does not depend on ripgrep being installed. Bash/web/notebook/pdf/tasks are not captured here (see README).
const source = 'src/native/tools/{read,write,edit,glob,grep}.js';
const tools = builtinTools();
const WS = '<TMP>/ws';
const out = [];
const add = (id, files, steps, o = {}) => out.push({ id, input: { files: Object.fromEntries(Object.entries(files).map(([k, v]) => [`${WS}/${k}`, v])), dirs: (o.dirs || []).map((d) => `${WS}/${d}`), steps }, ...(o.note ? { note: o.note } : {}) });
const R = (file_path, extra = {}) => ({ tool: 'Read', input: { file_path, ...extra } });
const W = (file_path, content) => ({ tool: 'Write', input: { file_path, content } });
const E = (file_path, old_string, new_string, extra = {}) => ({ tool: 'Edit', input: { file_path, old_string, new_string, ...extra } });
const G = (pattern, { sort_lines, ...extra } = {}) => ({ tool: 'Grep', input: { pattern, ...extra }, ...(sort_lines ? { sort_lines } : {}) });
const GL = (pattern, extra = {}) => ({ tool: 'Glob', input: { pattern, ...extra } });
const lines = (n, f = (i) => `line ${i}`) => Array.from({ length: n }, (_, i) => f(i + 1)).join('\n') + '\n';
const UNORDERED = { sort_lines: true }; // directory-walk order is OS-dependent: compare the lines as a set

// ---- Read ----
add('read-basic-cat-n', { 'a.txt': 'one\ntwo\nthree\n' }, [R('a.txt')]);
add('read-absolute-path', { 'a.txt': 'x\n' }, [R(`${WS}/a.txt`)]);
add('read-offset-and-limit', { 'a.txt': lines(10) }, [R('a.txt', { offset: 3, limit: 2 })]);
add('read-offset-beyond-end-is-empty', { 'a.txt': lines(3) }, [R('a.txt', { offset: 50 })]);
add('read-invalid-offset-limit-use-defaults', { 'a.txt': lines(3) }, [R('a.txt', { offset: -4, limit: 0 }), R('a.txt', { offset: 1.5, limit: 'x' })]);
add('read-no-trailing-newline', { 'a.txt': 'a\nb' }, [R('a.txt')]);
add('read-only-last-newline-stripped', { 'a.txt': 'a\n\n\n' }, [R('a.txt')]);
add('read-crlf-keeps-carriage-return', { 'a.txt': 'a\r\nb\r\n' }, [R('a.txt')]);
add('read-unicode', { 'a.txt': 'héllo\n日本語\n😀\n' }, [R('a.txt')]);
add('read-line-longer-than-2000-truncated', { 'a.txt': `${'x'.repeat(2100)}\nshort\n` }, [R('a.txt')]);
add('read-default-limit-2000-lines', { 'a.txt': lines(2005) }, [R('a.txt', { offset: 1998 }), R('a.txt', { offset: 2000 })]);
add('read-empty-file-note', { 'a.txt': '' }, [R('a.txt')]);
add('read-missing-file-soft-enoent', {}, [R('nope.txt')]);
add('read-directory-soft-error', {}, [R('sub')], { dirs: ['sub'] });
add('read-binary-file-error', { 'b.bin': { base64: 'AAECAwQ=' } }, [R('b.bin')]);
add('read-image-returns-base64-block', { 'p.png': { base64: 'iVBORw0KGgo=' }, 'j.JPG': { base64: '/9j/4A==' } }, [R('p.png'), R('j.JPG')]);
add('read-notebook-cells', { 'n.ipynb': { nbformat: 4, metadata: {}, cells: [{ cell_type: 'markdown', id: 'c1', source: ['# Title'] }, { cell_type: 'code', id: 'c2', source: ['print(1)\n'], outputs: [{ output_type: 'stream', name: 'stdout', text: ['1\n'] }], execution_count: 1 }] } }, [R('n.ipynb')]);
add('read-notebook-invalid-json-falls-back-to-text', { 'n.ipynb': 'not json\n' }, [R('n.ipynb')]);
add('read-missing-file_path', {}, [{ tool: 'Read', input: {} }, { tool: 'Read', input: { file_path: '' } }, { tool: 'Read' }]);

// ---- Write ----
add('write-creates-file-and-parent-dirs', {}, [W('deep/er/new.txt', 'hello')]);
add('write-multibyte-reports-bytes', {}, [W('u.txt', 'héllo 日本')]);
add('write-empty-content-allowed', {}, [W('e.txt', '')]);
add('write-existing-requires-read', { 'a.txt': 'old' }, [W('a.txt', 'new')]);
add('write-after-read-overwrites', { 'a.txt': 'old' }, [R('a.txt'), W('a.txt', 'new'), W('a.txt', 'newer')]);
add('write-denied-when-changed-since-read', { 'a.txt': 'old' }, [R('a.txt'), { tool: '__write', input: { path: 'a.txt', content: 'external', mtime: 1800000000 } }, W('a.txt', 'mine')]);
add('write-directory-target-soft-error', {}, [W('sub', 'x')], { dirs: ['sub'] });
add('write-missing-params', {}, [{ tool: 'Write', input: { content: 'x' } }, { tool: 'Write', input: { file_path: 'a.txt' } }, { tool: 'Write', input: { file_path: 'a.txt', content: 5 } }]);
add('write-absolute-path', {}, [W(`${WS}/abs.txt`, 'abs')]);

// ---- Edit ----
add('edit-unique-replacement-with-snippet', { 'a.txt': lines(8) }, [R('a.txt'), E('a.txt', 'line 5', 'LINE FIVE')]);
add('edit-requires-prior-read', { 'a.txt': 'foo\n' }, [E('a.txt', 'foo', 'bar')]);
add('edit-stale-read-rejected', { 'a.txt': 'foo\n' }, [R('a.txt'), { tool: '__write', input: { path: 'a.txt', content: 'foo changed\n', mtime: 1800000000 } }, E('a.txt', 'foo', 'bar')]);
add('edit-old-string-not-found', { 'a.txt': 'foo\n' }, [R('a.txt'), E('a.txt', 'zzz', 'bar')]);
add('edit-multiple-matches-need-replace-all', { 'a.txt': 'x x x\n' }, [R('a.txt'), E('a.txt', 'x', 'y')]);
add('edit-replace-all', { 'a.txt': 'x x x\nx\n' }, [R('a.txt'), E('a.txt', 'x', 'y', { replace_all: true })]);
add('edit-identical-strings-rejected', { 'a.txt': 'x\n' }, [R('a.txt'), E('a.txt', 'x', 'x')]);
add('edit-empty-old-string-not-found', { 'a.txt': 'x\n' }, [R('a.txt'), E('a.txt', '', 'y')]);
add('edit-deletion-with-empty-new-string', { 'a.txt': 'keep\nremove me\nkeep\n' }, [R('a.txt'), E('a.txt', 'remove me\n', '')]);
add('edit-multiline-new-string-snippet-bounds', { 'a.txt': lines(6) }, [R('a.txt'), E('a.txt', 'line 2\nline 3', 'A\nB\nC\nD')]);
add('edit-crlf-file-lf-old-string-keeps-crlf', { 'a.txt': 'a\r\nb\r\nc\r\n' }, [R('a.txt'), E('a.txt', 'b', 'B1\nB2')]);
add('edit-crlf-file-crlf-old-string', { 'a.txt': 'a\r\nb\r\nc\r\n' }, [R('a.txt'), E('a.txt', 'a\r\nb', 'X')]);
add('edit-first-line-snippet-start', { 'a.txt': 'first\nsecond\n' }, [R('a.txt'), E('a.txt', 'first', '1st')]);
add('edit-chained-edits-after-write-need-no-reread', { 'a.txt': 'a b\n' }, [R('a.txt'), E('a.txt', 'a', 'A'), E('a.txt', 'b', 'B')]);
add('edit-missing-file-soft-enoent', {}, [E('nope.txt', 'a', 'b')]);
add('edit-directory-soft-error', {}, [E('sub', 'a', 'b')], { dirs: ['sub'] });
add('edit-missing-params', {}, [{ tool: 'Edit', input: { old_string: 'a', new_string: 'b' } }, { tool: 'Edit', input: { file_path: 'a', old_string: 'a' } }]);
add('edit-dollar-and-special-chars-literal', { 'a.txt': 'price $1 (x)\n' }, [R('a.txt'), E('a.txt', '$1 (x)', '$& $$ \\n')]);

// ---- Glob ----
const SRC = { 'src/a.js': '1', 'src/b.js': '2', 'src/deep/c.js': '3', 'src/d.ts': '4', 'README.md': '5', 'node_modules/p/i.js': '6', '.git/config': '7', 'lib/e.js': '8' };
add('glob-recursive-sorted-by-mtime-newest-first', SRC, [GL('**/*.js')]);
add('glob-excludes-node_modules-and-git', SRC, [GL('**/*')]);
add('glob-pattern-naming-node_modules-includes-it', SRC, [GL('node_modules/**/*.js')]);
add('glob-path-argument', SRC, [GL('*.js', { path: 'src' }), GL('*.js', { path: `${WS}/lib` })]);
add('glob-no-match', SRC, [GL('**/*.zig')]);
add('glob-missing-pattern', {}, [{ tool: 'Glob', input: {} }]);
add('glob-truncates-at-100', Object.fromEntries(Array.from({ length: 105 }, (_, i) => [`f/${String(i).padStart(3, '0')}.txt`, 'x'])), [GL('f/*.txt')]);
add('glob-braces-and-question-mark', SRC, [GL('src/*.{js,ts}'), GL('src/?.js')]);

// ---- Grep (JS fallback) ----
const CODE = { 'a.js': 'const foo = 1;\nfunction bar() {}\nFOO again\n', 'b.ts': 'let foo: number;\n', 'sub/c.md': 'foo in markdown\nline2\nfoo again\n', 'node_modules/x.js': 'foo hidden\n', '.git/h': 'foo hidden\n', 'bin.dat': { base64: 'Zm9vAGZvbw==' } };
add('grep-files-with-matches-default', CODE, [G('foo', { ...UNORDERED })]);
add('grep-content-mode-line-numbers', { 'a.js': CODE['a.js'] }, [G('foo', { output_mode: 'content', path: 'a.js' })]);
add('grep-content-case-insensitive', { 'a.js': CODE['a.js'] }, [G('foo', { output_mode: 'content', '-i': true, path: 'a.js' })]);
add('grep-count-mode', CODE, [G('foo', { output_mode: 'count', ...UNORDERED })]);
add('grep-context-after-before-both', { 'a.txt': lines(10) }, [G('line 5', { output_mode: 'content', '-A': 1, path: 'a.txt' }), G('line 5', { output_mode: 'content', '-B': 2, path: 'a.txt' }), G('line 5', { output_mode: 'content', '-C': 1, path: 'a.txt' })]);
add('grep-context-at-file-edges', { 'a.txt': lines(3) }, [G('line 1', { output_mode: 'content', '-B': 5, '-A': 5, path: 'a.txt' })]);
add('grep-glob-filter', CODE, [G('foo', { glob: '*.js', ...UNORDERED }), G('foo', { glob: '**/*.md', ...UNORDERED })]);
add('grep-type-filter', CODE, [G('foo', { type: 'ts', ...UNORDERED }), G('foo', { type: 'md', ...UNORDERED })]);
add('grep-unknown-type-no-filter', { 'a.js': 'foo\n' }, [G('foo', { type: 'nonsense' })]);
add('grep-multiline', { 'a.txt': 'start\nmiddle\nend\nstart\nx\n' }, [G('start.*?end', { multiline: true, output_mode: 'content', path: 'a.txt' }), G('start.*?end', { multiline: true, output_mode: 'count', path: 'a.txt' })]);
add('grep-head-limit', { 'a.txt': lines(10) }, [G('line', { output_mode: 'content', head_limit: 3, path: 'a.txt' }), G('line', { output_mode: 'content', head_limit: 0, path: 'a.txt' })]);
add('grep-no-matches', CODE, [G('zzzzz')]);
add('grep-invalid-regex', CODE, [G('(unclosed')]);
add('grep-missing-path', CODE, [G('foo', { path: 'nope' })]);
add('grep-missing-pattern', CODE, [{ tool: 'Grep', input: {} }]);
add('grep-single-file-path', CODE, [G('foo', { path: 'sub/c.md' })]);
add('grep-skips-binary-node_modules-and-git', CODE, [G('foo', { output_mode: 'count', ...UNORDERED })]);
add('grep-absolute-search-path', CODE, [G('foo', { path: `${WS}/sub` })]);
add('grep-line-numbers-flag-ignored-by-js-fallback', { 'a.txt': 'foo\n' }, [G('foo', { output_mode: 'content', '-n': false, path: 'a.txt' })], { note: 'rg honours -n:false; the JS fallback always prints line numbers. Rust must decide which to match.' });

exports.source = source;
exports.cases = () => out;
exports.run = async (i, sb) => {
  sb.setup({ files: i.files, dirs: i.dirs });
  const ctx = { cwd: sb.ws, env: {}, signal: undefined, readFiles: new Map(), setTodos: () => {}, skills: [], readSkill: async () => '', home: sb.home, forceJsGrep: true };
  const steps = [];
  for (const s of i.steps) {
    if (s.tool === '__write') {
      const p = path.resolve(sb.ws, s.input.path);
      fs.writeFileSync(p, s.input.content);
      fs.utimesSync(p, s.input.mtime, s.input.mtime);
      steps.push({ tool: '__write', ok: true });
      continue;
    }
    const tool = findTool(tools, s.tool);
    const r = await tool.run(s.input, ctx);
    const rec = { tool: s.tool, content: r.content, isError: !!r.isError, soft: !!r.soft, ...(r.meta ? { meta: r.meta } : {}) };
    if (s.sort_lines && typeof rec.content === 'string') rec.content = rec.content.split('\n').sort().join('\n');
    steps.push(rec);
  }
  return { steps, tree_after: sb.tree(sb.ws) };
};
