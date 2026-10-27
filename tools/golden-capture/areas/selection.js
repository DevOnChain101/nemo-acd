'use strict';
const { ref } = require('../env');
const path = require('path');
const sel = require(path.join(ref, 'src/selection.js'));

const source = 'src/selection.js';
const out = [];
const add = (id, input, extra = {}) => out.push({ id, input, ...extra });
const E = '\x1b';

// ---- selectionTransition: one case per (mode, event type) with every flag combination ----
const bools = [false, true];
const combos = (keys) => keys.reduce((acc, k) => acc.flatMap((a) => bools.map((b) => ({ ...a, [k]: b }))), [{}]);
const EVENTS = {
  down: combos(['text', 'body', 'shift', 'alt']).map((c) => ({ type: 'down', ...c })),
  drag: [{ type: 'drag' }],
  up: combos(['moved', 'shift', 'alt', 'extending']).map((c) => ({ type: 'up', ...c })),
  wheel: combos(['shift', 'alt']).map((c) => ({ type: 'wheel', ...c })),
  key: [{ type: 'key', page: true }, { type: 'key', page: false }, { type: 'key' }],
  cancel: [{ type: 'cancel' }],
  unknown: [{ type: 'bogus' }, { type: 'resize' }],
};
for (const mode of ['idle', 'dragging', 'selected', 'pending']) {
  for (const [kind, events] of Object.entries(EVENTS)) add(`transition-${mode}-${kind}`, { op: 'transition', mode, events });
}

// ---- pure geometry / text helpers ----
add('edge-scroll-step-table', { op: 'edgeScrollStep', bounds: { y0: 5, y1: 20 }, ys: [-10, 0, 4, 5, 6, 19, 20, 21, 22, 26, 27, 40, 100] });
add('describe-mouse', { op: 'describeMouse', events: [{ name: 'mouse', action: 'down', shift: true, alt: false, ctrl: false, x: 3, y: 4 }, { name: 'wheel-up', x: 1, y: 2 }, { name: 'mouse-drag', shift: false, alt: true, ctrl: true, x: 9, y: 9 }] });
const RECT = { x: 10, y: 2, w: 60 };
add('body-geom-table', { op: 'bodyGeom', cases: [{ rect: RECT, body: { total: 100, h: 20 }, scroll: 0 }, { rect: RECT, body: { total: 100, h: 20 }, scroll: 30 }, { rect: RECT, body: { total: 100, h: 20 }, scroll: 999 }, { rect: RECT, body: { total: 5, h: 20 }, scroll: 0 }, { rect: RECT, body: { total: 0, h: 20 }, scroll: 0 }, { rect: RECT, body: { total: 10, h: 1 }, scroll: 0 }, { rect: null, body: { total: 10, h: 5 }, scroll: 0 }, { rect: { x: 0, y: 0, w: 1 }, body: { total: 10, h: 5 }, scroll: undefined }] });
add('point-to-content-and-to-screen', { op: 'geomPoints', rect: RECT, body: { total: 100, h: 20 }, scroll: 30, points: [{ x: 0, y: 0 }, { x: 15, y: 5 }, { x: 999, y: 999 }, { x: 12, y: 21 }], sels: [{ a: { x: 3, y: 60 }, b: { x: 7, y: 65 } }] });
add('cell-has-text', { op: 'cellHasText', line: `${E}[31mab ${E}[0m日本x`, xs: [0, 1, 2, 3, 4, 5, 6, 7, 8, 99, -1] });
add('clamp-point-table', { op: 'clampPoint', bounds: { x0: 2, x1: 12, y0: 1, y1: 3 }, points: [{ x: 99, y: 99 }, { x: -4, y: -4 }, { x: 5, y: 2 }, { x: 12, y: 3 }, { x: 11, y: 0 }] });
const B = { x0: 2, x1: 12, y0: 1, y1: 3 };
const S = { a: { x: 5, y: 1 }, b: { x: 7, y: 3 } };
add('row-range-forward-and-reversed', { op: 'rowRange', bounds: B, sel: S, ys: [0, 1, 2, 3, 4] });
add('row-range-reversed-selection', { op: 'rowRange', bounds: B, sel: { a: S.b, b: S.a }, ys: [0, 1, 2, 3, 4] });
add('row-range-single-row-and-empty', { op: 'rowRange', bounds: B, sel: { a: { x: 6, y: 2 }, b: { x: 6, y: 2 } }, ys: [1, 2, 3] });
add('row-range-clipped-to-bounds', { op: 'rowRange', bounds: B, sel: { a: { x: 0, y: 2 }, b: { x: 99, y: 2 } }, ys: [2] });
const LINES = ['border', '  hello wor', '  second row', '  third'];
add('highlight-cells-plain', { op: 'highlightCells', line: 'plain text', from: 2, to: 5 });
add('highlight-cells-colour-inside-range', { op: 'highlightCells', line: `ab${E}[31mcd${E}[0mef`, from: 1, to: 4 });
add('highlight-cells-out-of-range-unchanged', { op: 'highlightCells', line: 'plain', from: 9, to: 12 });
add('highlight-cells-wide-chars', { op: 'highlightCells', line: '日本 ab', from: 0, to: 2 });
add('highlight-cells-keeps-outer-colour-after-range', { op: 'highlightCells', line: `${E}[1m${E}[32mabcdef`, from: 1, to: 3 });
add('apply-selection-frame', { op: 'applySelection', lines: LINES, sel: { a: { x: 2, y: 1 }, b: { x: 5, y: 2 } }, bounds: B });
add('apply-selection-null-sel-returns-same', { op: 'applySelection', lines: LINES, sel: null, bounds: B });
add('apply-selection-blank-rows-unpainted', { op: 'applySelection', lines: ['', '   ', '  x  '], sel: { a: { x: 0, y: 0 }, b: { x: 9, y: 2 } }, bounds: { x0: 0, x1: 10, y0: 0, y1: 2 } });
add('slice-cells', { op: 'sliceCells', line: `${E}[1m日本 ab${E}[0m`, ranges: [[0, 2], [0, 4], [1, 3], [5, 7], [0, 99], [3, 3]] });
add('text-of-selection-colour-and-trailing-spaces', { op: 'textOfSelection', lines: ['border', `${E}[1m  hello wor${E}[0m   `, '  second row', '  third'], sel: { a: { x: 4, y: 1 }, b: { x: 6, y: 2 } }, bounds: B });
add('text-of-selection-wide-chars', { op: 'textOfSelection', lines: ['', '日本 ab'], sel: { a: { x: 0, y: 1 }, b: { x: 3, y: 1 } }, bounds: { x0: 0, x1: 20, y0: 1, y1: 1 } });
add('text-of-selection-collapses-blank-runs', { op: 'textOfSelection', lines: ['a', '', '', '', 'b', '', ''], sel: { a: { x: 0, y: 0 }, b: { x: 5, y: 6 } }, bounds: { x0: 0, x1: 10, y0: 0, y1: 6 } });
add('text-of-selection-null-sel', { op: 'textOfSelection', lines: LINES, sel: null, bounds: B });
add('text-of-content-offscreen-rows', { op: 'textOfContent', rows: ['  alpha beta', '  gamma', '  delta epsilon'], from: 10, sel: { a: { x: 4, y: 10 }, b: { x: 5, y: 12 } }, xb: { x0: 2, x1: 20 } });
add('copy-to-clipboard-osc52', { op: 'copy', platform: 'linux', isTTY: true, text: 'text xin chào 日本' });
add('copy-to-clipboard-not-tty-writes-nothing', { op: 'copy', platform: 'linux', isTTY: false, text: 'x' });
add('copy-to-clipboard-win32-utf16le-bom', { op: 'copy', platform: 'win32', isTTY: true, text: 'xin chào 日本' });
add('copy-to-clipboard-win32-spawn-fails-falls-back', { op: 'copy', platform: 'win32', isTTY: true, text: 'abc', spawnThrows: true });

exports.source = source;
exports.cases = () => out;
exports.run = async (i) => {
  switch (i.op) {
    case 'transition': return i.events.map((ev) => ({ event: ev, result: sel.selectionTransition(i.mode, ev) }));
    case 'edgeScrollStep': return i.ys.map((y) => ({ y, step: sel.edgeScrollStep(y, i.bounds) }));
    case 'describeMouse': return i.events.map((e) => sel.describeMouse(e));
    case 'bodyGeom': return i.cases.map((c) => ({ ...c, geom: sel.bodyGeom(c.rect, c.body, c.scroll), bounds: ((g) => (g ? sel.bodyBounds(g) : null))(sel.bodyGeom(c.rect, c.body, c.scroll)) }));
    case 'geomPoints': {
      const g = sel.bodyGeom(i.rect, i.body, i.scroll);
      return { geom: g, bounds: sel.bodyBounds(g), content: i.points.map((p) => ({ screen: p, content: sel.pointToContent(g, p) })), toScreen: i.sels.map((s) => ({ sel: s, screen: sel.toScreen(s, g) })) };
    }
    case 'cellHasText': return i.xs.map((x) => ({ x, hasText: sel.cellHasText(i.line, x) }));
    case 'clampPoint': return i.points.map((p) => ({ p, clamped: sel.clampPoint(p, i.bounds) }));
    case 'rowRange': return i.ys.map((y) => ({ y, range: sel.rowRange(i.sel, i.bounds, y) }));
    case 'highlightCells': return { result: sel.highlightCells(i.line, i.from, i.to) };
    case 'applySelection': { const r = sel.applySelection(i.lines, i.sel, i.bounds); return { same_reference: r === i.lines, lines: r }; }
    case 'sliceCells': return i.ranges.map(([a, b]) => ({ range: [a, b], text: sel.sliceCells(i.line, a, b) }));
    case 'textOfSelection': return { text: sel.textOfSelection(i.lines, i.sel, i.bounds) };
    case 'textOfContent': return { text: sel.textOfContent(i.sel, i.xb, (a, b) => i.rows.slice(a - i.from, b - i.from + 1)) };
    case 'copy': {
      const written = [];
      const spawned = [];
      const chunks = [];
      const spawnImpl = (cmd) => {
        if (i.spawnThrows) throw new Error('ENOENT');
        spawned.push(cmd.replace(/\\/g, '/').split('/').slice(-2).join('/'));
        return { on() {}, stdin: { on() {}, end: (b) => chunks.push(b.toString('base64')) } };
      };
      const ret = sel.copyToClipboard(i.text, { out: { isTTY: i.isTTY, write: (d) => written.push(d) }, platform: i.platform, spawnImpl });
      return { returned: ret, osc_written: written, spawned_tail: spawned, stdin_base64: chunks };
    }
    default: throw new Error(i.op);
  }
};
