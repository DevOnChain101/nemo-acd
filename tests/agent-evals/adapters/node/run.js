// Drives the frozen Node NativeAgent programmatically (read-only require of NEMO_NODE_REF) and converts the
// transcript into a minimal events.md stream. Node has no headless mode, so this is the baseline path.
//   EVAL_FAKE_PROVIDER=1  scripted fake provider (self-test, no network)
//   otherwise             real provider from config/env; EVAL_MODEL="provider/model" optional
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { makeEmitter, startTask, endTask } from '../_events.js';

const REF = process.env.NEMO_NODE_REF || path.join(os.homedir(), 'Vibe', 'nemo-cli');
const cwd = process.env.EVAL_CWD ? fs.realpathSync(process.env.EVAL_CWD) : '';
const prompt = process.env.EVAL_PROMPT || '';
const fake = process.env.EVAL_FAKE_PROVIDER === '1';
const die = (msg) => { process.stderr.write(`adapter "node": ${msg}\n`); process.exit(3); };

if (!cwd || !prompt) die('EVAL_CWD and EVAL_PROMPT are required');
if (!fs.existsSync(path.join(REF, 'src', 'native', 'agent.js'))) die(`Node reference not found at ${REF} (set NEMO_NODE_REF)`);
if (!fake && !Object.keys(process.env).some((k) => /(_API_KEY|_AUTH_TOKEN)$/.test(k) && process.env[k])) die('no provider API key in the environment (set e.g. NEMO_ANTHROPIC_API_KEY) and EVAL_FAKE_PROVIDER is not 1');

const req = createRequire(path.join(REF, 'package.json'));
const { NativeAgent } = req('./src/native/agent');
const { defaults } = req('./src/native/config');
const version = req('./package.json').version;

const nemoHome = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'eval-nemohome-'))); // never the real ~/.nemo
const home = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'eval-home-')));
const workspace = { // no discovery of the machine's real ~/.claude files
  settings: { permissions: { allow: [], deny: [], ask: [], defaultMode: 'default', additionalDirectories: [] }, hooks: {}, disableAllHooks: true },
  memory: [], autoMemoryDir: '', agents: [], skills: [], commands: [], projectRoot: cwd, claudeDir: cwd, warnings: [],
};

const msg = (id, blocks, stop, usage) => [
  { type: 'message_start', message: { id, model: 'fake-model', usage: {} } },
  ...blocks.flatMap((b, i) => b.type === 'text'
    ? [{ type: 'content_block_start', index: i, content_block: { type: 'text', text: '' } }, { type: 'content_block_delta', index: i, delta: { type: 'text_delta', text: b.text } }, { type: 'content_block_stop', index: i }]
    : [{ type: 'content_block_start', index: i, content_block: { type: 'tool_use', id: b.id, name: b.name, input: {} } }, { type: 'content_block_delta', index: i, delta: { type: 'input_json_delta', partial_json: JSON.stringify(b.input) } }, { type: 'content_block_stop', index: i }]),
  { type: 'message_delta', delta: { stop_reason: stop }, usage },
  { type: 'message_stop' },
];
const FAKE_TURNS = [
  msg('m1', [{ type: 'tool_use', id: 't1', name: 'Write', input: { file_path: 'FAKE_AGENT.txt', content: 'written by the scripted fake provider\n' } }], 'tool_use', { input_tokens: 10, output_tokens: 5 }),
  msg('m2', [{ type: 'text', text: 'Done (fake provider).' }], 'end_turn', { input_tokens: 12, output_tokens: 4 }),
];

const opts = { cwd, home, nemoHome, env: process.env };
if (fake) {
  const config = defaults();
  config.model = 'anthropic/fake-sonnet';
  config.providers.anthropic.models = ['fake-sonnet'];
  config._modelExplicit = true;
  let i = 0;
  Object.assign(opts, { config, workspace, auth: { getCredential: () => ({ type: 'api', key: 'fake-key' }) },
    providerFactory: () => ({ provider: { async *stream() { for (const e of FAKE_TURNS[Math.min(i++, FAKE_TURNS.length - 1)]) yield e; } }, model: 'fake-model' }) });
} else {
  opts.workspace = workspace;
}

const emit = makeEmitter();
const agent = new NativeAgent(opts);
agent.interactive = false; // no prompt can be shown
agent.setMode(process.env.EVAL_PERMISSION_MODE || 'acceptEdits');
if (!fake && process.env.EVAL_MODEL) agent.setModel(process.env.EVAL_MODEL);
// headless approver: every ask is denied (events.md §6)
const denied = [];
agent.on('update', () => { for (const p of [...agent.pending]) { denied.push({ name: p.name || p.tool, input: p.input }); agent.answer(p, 'no'); } });
agent.on('error', () => {});

const model = fake ? 'node-fake/fake-model' : `node/${agent.activeModel || agent.model || process.env.EVAL_MODEL || 'default'}`;
startTask(emit, { model, prompt });
emit('notice', { level: 'info', text: `NativeAgent ${version}` });
agent.send(prompt);

const deadline = Date.now() + Number(process.env.EVAL_MAX_WALL_MS || 900000);
await new Promise((r) => setTimeout(r, 100));
while (agent.status !== 'idle' && Date.now() < deadline) await new Promise((r) => setTimeout(r, 50));

// transcript -> events
const errors = (agent.items || []).filter((it) => it.kind === 'note' && it.tone === 'error').map((it) => it.text);
const resultItem = (agent.items || []).filter((it) => it.kind === 'result').at(-1);
let input_tokens = 0;
let output_tokens = 0;
let iteration = 0;
const results = new Map();
for (const m of agent.messages) if (m.role === 'user' && Array.isArray(m.content)) for (const b of m.content) if (b.type === 'tool_result') results.set(b.tool_use_id, b);
for (const m of agent.messages) {
  if (m.role !== 'assistant') continue;
  emit('turn.started', { model, provider: 'node', iteration: ++iteration });
  const content = Array.isArray(m.content) ? m.content : [{ type: 'text', text: String(m.content) }];
  for (const b of content) {
    if (b.type !== 'tool_use') continue;
    emit('tool.requested', { tool_call_id: b.id, name: b.name, input: b.input, input_truncated: false });
    const r = results.get(b.id);
    emit('tool.completed', { tool_call_id: b.id, name: b.name, status: !r ? 'interrupted' : r.is_error ? 'error' : 'ok', is_error: !!(r && r.is_error), not_run: !r, duration_ms: 0, preview: r ? String(typeof r.content === 'string' ? r.content : JSON.stringify(r.content)).slice(0, 200) : '', bytes: 0, truncated: false });
  }
  emit('assistant.message', { content, stop_reason: m.stop_reason || 'end_turn' });
}
if (agent.live && agent.live.tokens) { input_tokens = agent.live.tokens.input || 0; output_tokens = agent.live.tokens.output || 0; }
for (const d of denied) emit('tool.approval_resolved', { approval_id: 'headless', tool_call_id: '', decision: 'auto_denied', reviewer: 'headless', input_changed: false, message: String(d.name) });

const timedOut = agent.status !== 'idle';
const failed = timedOut || errors.length > 0 || (resultItem && resultItem.ok === false);
endTask(emit, failed ? 'failed' : 'completed', timedOut ? 'wall time' : failed ? (errors[0] || (resultItem && resultItem.text) || 'model error') : 'end_turn', { input_tokens, output_tokens });
agent.dispose();
fs.rmSync(nemoHome, { recursive: true, force: true });
fs.rmSync(home, { recursive: true, force: true });
process.exit(0);
