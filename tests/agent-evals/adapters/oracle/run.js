// Applies the reference fix (EVAL_ORACLE_PATCH, set by the harness only for this adapter) and reports honestly.
import fs from 'node:fs';
import { spawnSync } from 'node:child_process';
import { makeEmitter, startTask, endTask } from '../_events.js';

const emit = makeEmitter();
startTask(emit, { model: 'oracle/oracle', prompt: process.env.EVAL_PROMPT || '' });
const patch = process.env.EVAL_ORACLE_PATCH;
let status = process.env.EVAL_ORACLE_STATUS || 'completed';
if (patch && fs.existsSync(patch)) {
  const r = spawnSync('git', ['apply', patch], { cwd: process.env.EVAL_CWD, encoding: 'utf8' });
  if (r.status !== 0) {
    process.stderr.write(r.stderr || String(r.error));
    status = 'failed';
  }
}
endTask(emit, status, 'oracle');
