// Minimal events.md producer shared by the adapters: envelope v1, JSONL on stdout.
import { randomUUID } from 'node:crypto';

export function makeEmitter() {
  const session_id = randomUUID();
  const task_id = randomUUID();
  let seq = 0;
  return function emit(type, payload = {}, extra = {}) {
    const ev = { v: 1, seq: ++seq, ts: new Date().toISOString(), session_id, task_id, turn_id: null, agent_id: null, type, payload, ...extra };
    process.stdout.write(JSON.stringify(ev) + '\n');
  };
}

export function startTask(emit, { model, prompt }) {
  emit('session.started', { workspace: { workspace_id: 'eval', root: process.env.EVAL_CWD, cwd: process.env.EVAL_CWD, additional_dirs: [] }, model, permission_mode: 'acceptEdits', source: 'startup' });
  emit('task.started', { request: prompt, acceptance_criteria: [], budget: {} });
}

export function endTask(emit, status, reason, usage = { input_tokens: 0, output_tokens: 0 }) {
  emit('task.ended', { status, reason, usage, changed_files: [], artifacts_root: '' });
}
