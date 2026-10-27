'use strict';
// Must be required BEFORE any Node reference module: pins a deterministic,
// hermetic process environment so the real ~/.nemo and ~/.claude are never read.
const fs = require('fs');
const os = require('os');
const path = require('path');

const ref = (process.env.NEMO_NODE_REF || path.join(os.homedir(), 'Vibe', 'nemo-cli')).replace(/^~(?=$|\/)/, os.homedir());
const realHome = os.homedir();

const base = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'golden-env-')));
for (const k of Object.keys(process.env)) {
  if (/^(NEMO_|ANTHROPIC_|OPENAI_|CLAUDE_|COLOR|FORCE_COLOR)/.test(k) || k === 'COLORTERM') delete process.env[k];
}
Object.assign(process.env, {
  HOME: path.join(base, 'home'),
  USERPROFILE: path.join(base, 'home'),
  NEMO_HOME: path.join(base, 'nemo_home'),
  CLAUDE_CONFIG_DIR: path.join(base, 'claude_home'),
  TERM: 'dumb',
  NO_COLOR: '1',
  TZ: 'UTC',
  LANG: 'C.UTF-8',
});
for (const d of ['home', 'nemo_home', 'claude_home']) fs.mkdirSync(path.join(base, d), { recursive: true });

module.exports = { ref, realHome, envBase: base };
