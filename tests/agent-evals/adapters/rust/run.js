// Runs `nemo run` (the Rust binary) on the task. Binary: $NEMO_BIN, else target/{debug,release}/nemo.
//   EVAL_FAKE_SCRIPT=<script.json>  scripted model (needs a build with --features test-provider; selftest only)
//   EVAL_MODEL="provider/model"      optional; real mode needs NEMO_* credentials in the environment
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, '..', '..', '..', '..');
const exe = process.platform === 'win32' ? 'nemo.exe' : 'nemo';
const targetDir = process.env.CARGO_TARGET_DIR || path.join(repoRoot, 'target');
const bin = process.env.NEMO_BIN || ['debug', 'release'].map((p) => path.join(targetDir, p, exe)).find((p) => fs.existsSync(p));
const die = (msg) => { process.stderr.write(`adapter "rust": ${msg}\n`); process.exit(3); };

const cwd = process.env.EVAL_CWD;
const prompt = process.env.EVAL_PROMPT;
if (!cwd || !prompt) die('EVAL_CWD and EVAL_PROMPT are required');
if (!bin || !fs.existsSync(bin)) die('nemo binary not found (cargo build -p nemo-cli, or set NEMO_BIN)');

const tmp = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'eval-rust-')));
// Loopback: tasks start a local server in their tests (still no outside network). The node
// toolchain (npm lives next to node, often under $HOME) is granted read-only.
const args = ['run', '--cwd', cwd, '--prompt', prompt, '--no-session-store', '--network', 'loopback', '--sandbox-read', path.dirname(path.dirname(fs.realpathSync(process.execPath))), '--permission-mode', process.env.EVAL_PERMISSION_MODE || 'acceptEdits'];
const env = { ...process.env };
const fake = process.env.EVAL_FAKE_SCRIPT;
if (fake) { // never touch the real home in scripted mode
  Object.assign(env, { HOME: tmp, USERPROFILE: tmp, NEMO_HOME: path.join(tmp, 'nemo') });
  args.push('--provider', `fake:${fake}`);
} else if (process.env.EVAL_MODEL) {
  args.push('--model', process.env.EVAL_MODEL);
}
for (const [env_, flag] of [['EVAL_MAX_TURNS', '--max-turns'], ['EVAL_MAX_TOOL_CALLS', '--max-tool-calls']]) if (process.env[env_]) args.push(flag, process.env[env_]);
if (process.env.EVAL_MAX_WALL_MS) args.push('--max-wall-secs', String(Math.ceil(Number(process.env.EVAL_MAX_WALL_MS) / 1000)));

// Enforce (the default) refuses to start on a host without a usable sandbox that also honours
// `--network loopback` (`enforce_ready: yes` = the standard policy builds and wraps) (e.g. CI runners with userns blocked). That is a property of the host, not of
// the task: fall back to best-effort there. EVAL_SANDBOX=enforce|best-effort|off overrides.
let sandbox = process.env.EVAL_SANDBOX;
if (!sandbox) {
  const doc = spawnSync(bin, ['doctor', '--cwd', cwd], { env, encoding: 'utf8' }).stdout || '';
  sandbox = /enforce_ready: yes/.test(doc) && /loopback=(Weak|Strong)/.test(doc) ? 'enforce' : 'best-effort';
}
process.stderr.write(`adapter "rust": --sandbox ${sandbox}\n`);
args.push('--sandbox', sandbox);

// The task's acceptance criteria are what a user would hand to `--criteria-file`.
const manifest = path.join(here, '..', '..', 'tasks', process.env.EVAL_TASK_ID || '', 'task.json');
if (process.env.EVAL_TASK_ID && fs.existsSync(manifest)) {
  const criteria = JSON.parse(fs.readFileSync(manifest, 'utf8')).acceptance_criteria || [];
  if (criteria.length) {
    const f = path.join(tmp, 'criteria.json');
    fs.writeFileSync(f, JSON.stringify(criteria));
    args.push('--criteria-file', f);
    // The operator who hands over a check authorises that exact command (the gate asks like Bash).
    for (const c of criteria) if (c && typeof c.check === 'string' && c.check.trim()) args.push('--allow', `Bash(${c.check.trim()})`);
  }
}

// stdout (events) and stderr pass straight through to the harness.
const r = spawnSync(bin, args, { env, stdio: ['ignore', 'inherit', 'inherit'] });
fs.rmSync(tmp, { recursive: true, force: true });
if (r.error) die(String(r.error));
// nemo exits 1/3/4 for failed/unverified/blocked; the claim is in task.ended. 3 would read as "adapter unavailable".
const code = r.status ?? 70;
process.exit([0, 1, 3, 4].includes(code) ? 0 : code);
