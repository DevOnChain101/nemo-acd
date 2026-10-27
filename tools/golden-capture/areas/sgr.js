'use strict';
const { ref } = require('../env');
const path = require('path');
const sgr = require(path.join(ref, 'src/sgr.js'));

const source = 'src/sgr.js';
const E = '\x1b';
const out = [];
const add = (id, input, extra = {}) => out.push({ id, input, ...extra });
const RESET = `${E}[0m`;
const fg = (r, g, b) => `${E}[38;2;${r};${g};${b}m`;
const bg = (r, g, b) => `${E}[48;2;${r};${g};${b}m`;
const red = fg(248, 113, 113);

const LINES = {
  'redundant-fg-collapsed': `${red}a${RESET}${red}b${RESET}${red}c${RESET}  ${bg(10, 20, 30)}${red}d${RESET}`,
  'plain-text-untouched': 'hello world',
  'empty-line': '',
  'bold-dim-intensity': `${E}[1mB${E}[2mD${E}[22mN${E}[0m`,
  'italic-underline-inverse-strike': `${E}[3mi${E}[4mu${E}[7mr${E}[9ms${E}[23m${E}[24m${E}[27m${E}[29mx`,
  'blank-cells-skip-fg-change': `${red}a${E}[32m   ${red}b`,
  'blank-cell-with-bg-keeps-bg': `${bg(1, 2, 3)}   ${RESET}x`,
  'blank-cell-with-underline-shows-fg': `${E}[4m${red}  x`,
  'palette-indices-8-and-bright': `${E}[31ma${E}[92mb${E}[41mc${E}[104md${E}[39;49me`,
  'palette-256-indexed': `${E}[38;5;123ma${E}[48;5;200mb`,
  'default-fg-bg-reset-codes': `${red}a${E}[39mb${bg(5, 5, 5)}c${E}[49md`,
  'non-colour-csi-passthrough': `a${E}[2Kb${E}[1;5Hc${E}[?25l`,
  'wide-and-emoji-chars': `${red}日本😀${RESET}é`,
  'trailing-state-gets-reset': `${red}tail`,
  'empty-sgr-param-is-reset': `${red}a${E}[mb`,
  'greys-and-cube-corners': `${fg(0, 0, 0)}a${fg(255, 255, 255)}b${fg(128, 128, 128)}c${fg(47, 47, 47)}d${fg(114, 114, 114)}e${fg(238, 238, 238)}f${fg(239, 239, 239)}g`,
};
for (const [k, line] of Object.entries(LINES)) for (const mode of ['truecolor', '256']) add(`encode-${k}-${mode}`, { op: 'encode', line, mode });
add('encode-default-mode-is-truecolor', { op: 'encode', line: `${red}a`, mode: undefined });
add('to256-table', { op: 'to256', colors: [[0, 0, 0], [255, 255, 255], [128, 128, 128], [248, 113, 113], [95, 135, 175], [47, 47, 47], [48, 48, 48], [114, 114, 114], [115, 115, 115], [10, 20, 30], [238, 238, 238], [239, 239, 239], [255, 0, 0], [0, 255, 0], [0, 0, 255], [200, 100, 50]] });
const DM = [
  [{ TERM_PROGRAM: 'Apple_Terminal' }, 'darwin'], [{ TERM_PROGRAM: 'iTerm.app' }, 'darwin'], [{ COLORTERM: 'truecolor' }, 'linux'], [{ COLORTERM: '24BIT' }, 'linux'],
  [{ TERM_PROGRAM: 'Apple_Terminal', CLAUDE_MONITOR_COLOR: 'truecolor' }, 'darwin'], [{ CLAUDE_MONITOR_COLOR: '256', COLORTERM: 'truecolor' }, 'linux'], [{ CLAUDE_MONITOR_COLOR: '24bit' }, 'linux'],
  [{}, 'win32'], [{}, 'linux'], [{}, 'darwin'], [{ WT_SESSION: 'x' }, 'linux'], [{ TERM_PROGRAM: 'vscode' }, 'linux'], [{ TERM_PROGRAM: 'WezTerm' }, 'linux'], [{ TERM: 'xterm-kitty' }, 'linux'], [{ TERM: 'alacritty-direct' }, 'linux'], [{ TERM: 'xterm-256color' }, 'linux'], [{ TERM: 'dumb' }, 'win32'],
];
DM.forEach(([env, platform], n) => add(`detect-color-mode-${String(n).padStart(2, '0')}`, { op: 'detect', env, platform }));
add('apply-sgr-state-machine', { op: 'applySgr', steps: ['1', '3;4', '38;2;1;2;3', '48;5;9', '31', '0', '', '22', '39', '90', '107', '38;9', '99'] });

exports.source = source;
exports.cases = () => out;
exports.run = async (i) => {
  switch (i.op) {
    case 'encode': return { result: sgr.encode(i.line, i.mode) };
    case 'to256': return i.colors.map(([r, g, b]) => ({ rgb: [r, g, b], index: sgr.to256(r, g, b) }));
    case 'detect': return { mode: sgr.detectColorMode(i.env, i.platform) };
    case 'applySgr': {
      const st = { fg: null, bg: null, int: 0, attr: 0 };
      return i.steps.map((p) => { sgr.applySgr(st, p); return { params: p, state: { ...st } }; });
    }
    default: throw new Error(i.op);
  }
};
