'use strict';
const { ref } = require('../env');
const path = require('path');
const { parseKeys, createKeyState } = require(path.join(ref, 'src/keys.js'));

const source = 'src/keys.js';
const E = '\x1b';
const out = [];
// chunks: successive stdin reads through ONE key state; expected has the events per chunk and the pending buffer after each.
const add = (id, chunks, note) => out.push({ id, input: { chunks }, ...(note ? { note } : {}) });

const SEQS = { up: `${E}[A`, down: `${E}[B`, right: `${E}[C`, left: `${E}[D`, 'ss3-up': `${E}OA`, home: `${E}[H`, end: `${E}[F`, 'tilde-home': `${E}[1~`, delete: `${E}[3~`, pageup: `${E}[5~`, pagedown: `${E}[6~`, 'shift-tab': `${E}[Z`, 'shift-enter': `${E}[13;2u`, 'ctrl-arrows': `${E}[1;5A${E}[1;5B${E}[1;5C${E}[1;5D`, 'alt-arrows': `${E}[1;3A${E}[1;3B${E}[1;3C${E}[1;3D`, 'ctrl-page': `${E}[5;5~${E}[6;5~`, 'alt-page': `${E}[5;3~${E}[6;3~`, 'alt-enter': `${E}\r`, 'macos-option-left-right': `${E}b${E}f`, 'alt-backspace': `${E}\x7f${E}\b${E}[127;3u${E}[127;5u`, 'alt-a-v': `${E}a${E}v${E}[118;9u${E}[118;3u` };
for (const [k, v] of Object.entries(SEQS)) add(`seq-${k}`, [v]);
add('ctrl-keys', ['\x03\x01\x04\x05\x0b\x0c\x0e\x0f\x10\x16\x12\x15\x17']);
add('enter-tab-backspace-ctrl-j', ['\r\t\x7f\b\n']);
add('plain-text-and-control-mix', ['ab\x03cd\r']);
add('unicode-text-surrogates', ['héllo 日本 😀 x']);
add('control-chars-below-32-dropped', ['a\x00\x02\x06b']);
add('paste-detected-multiline-no-escape', ['line one\nline two\r\n']);
add('paste-detected-trailing-newlines-trimmed', ['abc\n\n\nxyz\n\n']);
add('single-line-with-trailing-enter-is-not-paste', ['hi\r']);
add('bracketed-paste', [`${E}[200~pasted\ntext${E}[201~after`]);
add('bracketed-paste-split-across-reads', [`${E}[200~part one `, `part two${E}[201~z`]);
add('bracketed-paste-runaway-over-4096-no-end', [`${E}[200~${'x'.repeat(4100)}`]);
add('mouse-down-up-left', [`${E}[<0;10;5M${E}[<0;10;5m`]);
add('mouse-buttons-middle-right', [`${E}[<1;1;1M${E}[<2;2;2M`]);
add('mouse-wheel-all-directions', [`${E}[<64;3;4M${E}[<65;3;4M${E}[<66;3;4M${E}[<67;3;4M`]);
add('mouse-modifiers-shift-alt-ctrl', [`${E}[<4;1;1M${E}[<8;1;1M${E}[<16;1;1M${E}[<28;1;1M${E}[<68;9;9M`]);
add('mouse-drag-bit32', [`${E}[<32;7;8M${E}[<34;7;9M`]);
add('mouse-report-split-mid-sequence', [`${E}[<0;1`, `2;5M`]);
add('cursor-position-report', [`${E}[24;80R`]);
add('unknown-csi-ignored', [`a${E}[99~b${E}[?25hc${E}[1;2;3X`]);
add('lone-escape-is-escape-key', [E]);
add('escape-followed-by-text', [`${E}xyz`]);
add('partial-csi-held-then-completed', [`${E}[`, 'A']);
add('partial-ss3-held', [`${E}O`, 'B']);
add('partial-csi-params-held', [`${E}[1;`, '5C']);
add('partial-escape-over-32-bytes-gives-up', [`${E}[${'1'.repeat(40)}`]);
add('longest-sequence-wins', [`${E}[1;5A${E}[1;3A${E}[5;5~`]);
add('empty-chunk', ['']);
add('buffer-carries-between-chunks', [`ab${E}[<0;3`, `;4Mcd`]);

exports.source = source;
exports.cases = () => out;
exports.run = async (i) => {
  const st = createKeyState();
  return { chunks: i.chunks.map((c) => ({ events: parseKeys(c, st), pending: st.buf })) };
};
