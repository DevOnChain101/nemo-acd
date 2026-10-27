'use strict';
// Hooks.run: matcher semantics, dedupe, outcome merging. Commands are POSIX `printf`/`cat`/`exit` one-liners run by the Bash tool's shell
// (Git Bash on Windows); no timing, no `ms` in the expected output. Hooks that time out / spawn failures are NOT captured (timing).
const { ref } = require('../env');
const path = require('path');
const { Hooks } = require(path.join(ref, 'src/native/hooks.js'));

const source = 'src/native/hooks.js';
const out = [];
const add = (id, input, extra = {}) => out.push({ id, input, ...extra });
const NOTE = 'runs `sh -c` one-liners (printf/cat/exit); POSIX shell required';
const say = (s) => `printf '%s' '${s.replace(/'/g, `'\\''`)}'`;
const j = (o) => say(JSON.stringify(o));
const cmd = (command, x = {}) => ({ type: 'command', command, ...x });
const grp = (matcher, ...hooks) => ({ ...(matcher === undefined ? {} : { matcher }), hooks });
// run: one Hooks.run; `config` = hooks section, `event`, `payload`, `target` (matcherTarget)
const hk = (id, config, o = {}) => add(id, { op: 'run', config, event: o.event || 'PreToolUse', payload: o.payload || { tool_name: 'Bash', tool_input: { command: 'ls' } }, target: 'target' in o ? o.target : 'Bash', opts: o.opts || {} }, o.noshell ? {} : { note: NOTE });
const pre = (...groups) => ({ PreToolUse: groups });

// ---- matcher semantics (no match => nothing spawned) ----
hk('match-exact-name-no-match', pre(grp('Write', cmd('exit 2'))), { noshell: true });
hk('match-alternation-hits', pre(grp('Write|Bash', cmd(say('hit')))));
hk('match-is-anchored-full-match', pre(grp('Bas', cmd('exit 2'))), { noshell: true });
hk('match-regex-prefix-wildcard', pre(grp('Ba.*', cmd(say('hit')))));
hk('match-star-and-empty-and-missing-all', pre(grp('*', cmd(say('a'))), grp('', cmd(say('b'))), grp(undefined, cmd(say('c')))));
hk('match-invalid-regex-never-matches', pre(grp('(', cmd('exit 2'))), { noshell: true });
hk('match-case-sensitive', pre(grp('bash', cmd('exit 2'))), { noshell: true });
hk('match-target-undefined-matches-only-wildcards', pre(grp('Bash', cmd('exit 2')), grp('*', cmd(say('w')))), { target: null });
hk('match-mcp-tool-name-regex', pre(grp('mcp__.*__read', cmd(say('mcp')))), { target: 'mcp__srv__read' });
hk('no-config-for-event', { Stop: [grp('*', cmd('exit 2'))] }, { noshell: true });
hk('non-command-and-empty-entries-skipped', pre(grp('*', { type: 'prompt', command: 'exit 2' }, { type: 'command' }, null, cmd(''), cmd(say('ok')))));
hk('groups-without-hooks-array-skipped', pre({ matcher: '*' }, { matcher: '*', hooks: 'nope' }, grp('*', cmd(say('ok')))), {});
hk('disabled-instance-is-noop', pre(grp('*', cmd('exit 2'))), { noshell: true, opts: { disabled: true } });
// ---- dedupe ----
hk('dedupe-same-matcher-same-command-runs-once', pre(grp('Bash', cmd(say('x'))), grp('Bash', cmd(say('x'))), grp('Bash', cmd(say('x'), { timeout: 5 }))));
hk('dedupe-different-matcher-same-command-runs-twice', pre(grp('Bash', cmd(say('x'))), grp('*', cmd(say('x')))));
hk('dedupe-within-one-group', pre(grp('*', cmd(say('x')), cmd(say('x')), cmd(say('y')))));
// ---- merge of outcomes ----
hk('exit2-blocks-with-stderr-reason', pre(grp('Bash', cmd("printf 'nope, blocked' >&2; exit 2"))));
hk('exit2-empty-stderr-default-reason', pre(grp('*', cmd('exit 2'))));
hk('first-block-reason-wins', pre(grp('*', cmd("printf 'first' >&2; exit 2"), cmd("printf 'second' >&2; exit 2"))));
hk('nonblocking-failure-exit1-ignored', pre(grp('*', cmd("printf '{\"decision\":\"block\",\"reason\":\"r\"}'; exit 1"))));
hk('json-permission-deny', pre(grp('*', cmd(j({ hookSpecificOutput: { permissionDecision: 'deny', permissionDecisionReason: 'policy says no' } })))));
hk('json-decision-priority-deny-beats-ask-beats-allow', pre(grp('*', cmd(j({ hookSpecificOutput: { permissionDecision: 'allow', permissionDecisionReason: 'a' } })), cmd(j({ hookSpecificOutput: { permissionDecision: 'deny', permissionDecisionReason: 'd' } })), cmd(j({ hookSpecificOutput: { permissionDecision: 'ask', permissionDecisionReason: 'k' } })))));
hk('json-first-of-equal-priority-keeps-reason', pre(grp('*', cmd(j({ hookSpecificOutput: { permissionDecision: 'ask', permissionDecisionReason: 'one' } })), cmd(j({ hookSpecificOutput: { permissionDecision: 'ask', permissionDecisionReason: 'two' } })))));
hk('json-decision-approve-is-allow', pre(grp('*', cmd(j({ decision: 'approve' })))));
hk('json-decision-block-needs-reason', pre(grp('a', cmd(j({ decision: 'block', reason: 'why' }))), grp('*', cmd(j({ decision: 'block' })))), { target: 'a' });
hk('json-block-overrides-allow-decision', pre(grp('*', cmd(j({ hookSpecificOutput: { permissionDecision: 'allow' } })), cmd(j({ decision: 'block', reason: 'stop it' })))));
hk('json-continue-false-sets-stop', pre(grp('*', cmd(j({ continue: false, stopReason: 'done here' })))), { event: 'Stop', payload: {}, target: null });
hk('json-updated-input-last-wins-and-context-collected', pre(grp('*', cmd(j({ hookSpecificOutput: { updatedInput: { command: 'one' }, additionalContext: 'c1' }, additionalContext: 'c2' })), cmd(j({ hookSpecificOutput: { updatedInput: { command: 'two' } } })))));
hk('json-invalid-json-ignored', pre(grp('*', cmd(say('{not json')))));
hk('plain-stdout-context-only-for-prompt-and-session-events', { UserPromptSubmit: [grp(undefined, cmd(say('remember: be concise')))] }, { event: 'UserPromptSubmit', payload: { prompt: 'hi' }, target: null });
hk('plain-stdout-ignored-for-pretooluse', pre(grp('*', cmd(say('remember')))));
hk('session-start-plain-stdout-context', { SessionStart: [grp('startup', cmd(say('ctx-from-session-start')))] }, { event: 'SessionStart', payload: { source: 'startup' }, target: 'startup' });
hk('results-keep-config-order-with-streams', pre(grp('*', cmd("printf 'o1'; printf 'e1' >&2"), cmd("printf 'o2'; exit 3"), cmd('exit 0'))));
// ---- stdin contract ----
hk('stdin-json-payload-contract', pre(grp('*', cmd('cat >&2'))), { opts: { sessionId: 's1', transcriptPath: 'x.jsonl', cwd: '<TMP>/ws' } });
hk('stdin-payload-keys-overridden-by-run', pre(grp('*', cmd('cat >&2'))), { payload: { session_id: 'forged', cwd: '/forged', hook_event_name: 'forged', tool_name: 'Bash' }, opts: { sessionId: 's1', transcriptPath: 'x.jsonl', cwd: '<TMP>/ws' } });
hk('env-project-dir-and-secret-stripping', { UserPromptSubmit: [grp('*', cmd(`printf '%s|%s|%s|%s' "$CLAUDE_PROJECT_DIR" "$NEMO_PROJECT_DIR" "\${NEMO_API_KEY:-absent}" "\${NEMO_ANTHROPIC_API_KEY:-absent}"`))] }, { event: 'UserPromptSubmit', payload: { prompt: 'x' }, target: null, opts: { cwd: '<TMP>/ws', env: { NEMO_API_KEY: 'flat-secret', NEMO_ANTHROPIC_API_KEY: 'secret' } } });

exports.source = source;
exports.cases = () => out;
exports.run = async (i, sb) => {
  const env = { PATH: process.env.PATH, ...(i.opts.env || {}) };
  const h = new Hooks(i.config, { cwd: i.opts.cwd || sb.ws, env, sessionId: i.opts.sessionId || '', transcriptPath: i.opts.transcriptPath || '', disabled: !!i.opts.disabled });
  const r = await h.run(i.event, i.payload, i.target === null ? {} : { matcherTarget: i.target });
  return { ...r, results: r.results.map(({ ms, ...x }) => x) };
};
