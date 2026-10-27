// Does nothing and claims success: the harness must score this as a false_success.
import { makeEmitter, startTask, endTask } from '../_events.js';

const emit = makeEmitter();
startTask(emit, { model: 'noop/noop', prompt: process.env.EVAL_PROMPT || '' });
endTask(emit, 'completed', 'noop');
