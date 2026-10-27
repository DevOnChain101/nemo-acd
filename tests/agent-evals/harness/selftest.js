// Self-test: oracle must succeed everywhere it can run, noop must be a false_success everywhere.
//   node tests/agent-evals/harness/selftest.js   (or: node --test tests/agent-evals/harness/selftest.js)
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { loadTasks, runEval, summarize, prepareRepo, copyHidden, runChecks } from './run.js';

const out = fs.mkdtempSync(path.join(os.tmpdir(), 'eval-selftest-'));
process.env.EVAL_LEAK_GRACE_MS = '500';
const names = (rs, pred) => rs.filter(pred).map((r) => r.task_id);

test('oracle succeeds on every runnable scenario', async () => {
  const rs = await runEval({ agent: 'oracle', runs: 1, out });
  console.log('\n== oracle ==\n' + summarize(rs).text);
  assert.deepEqual(names(rs, (r) => r.error), []);
  assert.deepEqual(names(rs, (r) => !r.skipped && !r.success), []);
  assert.deepEqual(names(rs, (r) => r.false_success), []);
});

test('noop is a false_success on every runnable scenario', async () => {
  const rs = await runEval({ agent: 'noop', runs: 1, out });
  console.log('\n== noop ==\n' + summarize(rs).text);
  assert.deepEqual(names(rs, (r) => r.error), []);
  assert.deepEqual(names(rs, (r) => !r.skipped && (r.success || !r.false_success)), []);
});

test('fixtures: regression passes before bug.patch, hidden fails after', async () => {
  for (const task of loadTasks()) {
    if (task.requires.includes('python3') && !fs.existsSync('/usr/bin/python3') && process.platform !== 'win32') continue;
    const clean = await prepareRepo(task, { bug: false });
    const buggy = await prepareRepo(task);
    try {
      if (task.regression_checks.length) assert.ok((await runChecks(task.regression_checks, clean.repo)).every((c) => c.passed), `${task.id}: clean repo must pass its own tests`);
      if (task.expected_status === 'completed') {
        copyHidden(task, buggy.repo);
        assert.ok((await runChecks(task.hidden_checks, buggy.repo)).some((c) => !c.passed), `${task.id}: hidden check must fail on the buggy repo`);
      }
    } finally {
      fs.rmSync(clean.dir, { recursive: true, force: true });
      fs.rmSync(buggy.dir, { recursive: true, force: true });
    }
  }
});

test('node adapter drives NativeAgent with the fake provider', async (t) => {
  if (!fs.existsSync(path.join(process.env.NEMO_NODE_REF || path.join(os.homedir(), 'Vibe', 'nemo-cli'), 'src/native/agent.js'))) return t.skip('Node reference not found');
  process.env.EVAL_FAKE_PROVIDER = '1';
  const rs = await runEval({ agent: 'node', taskIds: ['fe-cart-total'], runs: 1, out });
  delete process.env.EVAL_FAKE_PROVIDER;
  console.log('\n== node (fake provider) ==\n' + summarize(rs).text);
  assert.equal(rs[0].error, undefined);
  assert.equal(rs[0].claimed, 'completed');
  assert.equal(rs[0].metrics.tool_calls, 1);
  assert.equal(rs[0].false_success, true); // the scripted fake never fixes the bug
});

const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..', '..', '..');
const nemoBin = path.join(process.env.CARGO_TARGET_DIR || path.join(repoRoot, 'target'), 'debug', process.platform === 'win32' ? 'nemo.exe' : 'nemo');

test('rust adapter is skipped, not failed, when the binary is missing', async () => {
  process.env.NEMO_BIN = path.join(out, 'no-such-nemo');
  const rs = await runEval({ agent: 'rust', taskIds: ['fe-cart-total'], runs: 1, out });
  delete process.env.NEMO_BIN;
  assert.equal(rs[0].skipped, true);
});

test('rust adapter fixes be-node-pagination through the real binary (scripted model)', async (t) => {
  const b = spawnSync('cargo', ['build', '-q', '-p', 'nemo-cli', '--features', 'test-provider'], { cwd: repoRoot, encoding: 'utf8' });
  if (b.error || b.status !== 0) return t.skip('cannot build nemo with --features test-provider: ' + (b.error || b.stderr).toString().slice(0, 300));
  process.env.NEMO_BIN = nemoBin;
  process.env.EVAL_FAKE_SCRIPT = path.join(path.dirname(new URL(import.meta.url).pathname), '..', 'adapters', 'rust', 'scripts', 'be-node-pagination.json');
  const rs = await runEval({ agent: 'rust', taskIds: ['be-node-pagination'], runs: 1, out });
  delete process.env.NEMO_BIN;
  delete process.env.EVAL_FAKE_SCRIPT;
  console.log('\n== rust (scripted model) ==\n' + summarize(rs).text);
  assert.equal(rs[0].error, undefined);
  assert.equal(rs[0].success, true);
  assert.equal(rs[0].claimed, 'completed');
  assert.equal(rs[0].metrics.tool_calls, 2);
});

test.after(() => fs.rmSync(out, { recursive: true, force: true }));
