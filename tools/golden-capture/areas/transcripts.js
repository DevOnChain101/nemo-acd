'use strict';
const { ref } = require('../env');
const path = require('path');
const fs = require('fs');
const hist = require(path.join(ref, 'src/history.js'));
const parser = require(path.join(ref, 'src/parser.js'));

const source = 'src/history.js + src/parser.js';
const out = [];
const add = (id, input, extra = {}) => out.push({ id, input, ...extra });

// ---- entry builders (fixed timestamps; nothing reads a clock) ----
const T = (s) => `2026-10-02T00:${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}.000Z`;
const NOW = (s) => Date.parse(T(s));
const user = (s, content, x = {}) => ({ type: 'user', timestamp: T(s), message: { role: 'user', content }, ...x });
const asst = (s, id, content, usage, stop, x = {}) => ({ type: 'assistant', timestamp: T(s), message: { id, model: 'claude-opus-5.5', content, ...(usage ? { usage } : {}), ...(stop ? { stop_reason: stop } : {}) }, ...x });
const tu = (id, name, input = {}) => ({ type: 'tool_use', id, name, input });
const tr = (s, id, content = 'ok', isError = false, x = {}) => user(s, [{ type: 'tool_result', tool_use_id: id, content, is_error: isError }], x);
const txt = (text) => ({ type: 'text', text });

// replay: itemsFromEntries + full parser session + phase at `now`
const rp = (id, entries, o = {}, extra) => add(id, { op: 'replay', entries, ...(o.now != null ? { now: o.now } : { now: NOW(600) }), ...(o.limit != null ? { limit: o.limit } : {}) }, extra);

// ---- history.itemsFromEntries (history.test.js + edge cases) ----
rp('basic-dedupe-and-results', [
  user(0, '<system-reminder>skip me</system-reminder>'),
  user(1, 'fix the bug'),
  asst(2, 'm1', [{ type: 'thinking', thinking: 'hmm' }]),
  asst(2, 'm1', [txt('Looking.')]),
  asst(2, 'm1', [tu('t1', 'Read', { file_path: 'a.js' })]),
  asst(2, 'm1', [tu('t1', 'Read', { file_path: 'a.js' })]),
  tr(3, 't1', 'x\ny'),
  user(4, '<command-name>/compact</command-name><command-args></command-args>'),
  asst(5, 's1', [txt('sub-agent text')], null, null, { isSidechain: true }),
]);
rp('display-text-keeps-model-content', [user(1, 'Base directory for this skill: /skills/ponytail\n\nLong skill body\n\nARGUMENTS: full', { displayText: '/ponytail full' })]);
rp('display-text-with-images', [user(1, [txt('look'), { type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'AAAABBBB' } }, { type: 'image', source: { type: 'base64', media_type: 'image/jpeg', data: 'AAAA' } }], { displayText: '  see pics  ' })]);
rp('user-images-without-display-text', [user(1, [txt('what is this'), { type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'AAAABBBBCCCC' } }, { type: 'image', source: {} }])]);
rp('slash-command-with-args', [user(1, '<command-message>x</command-message><command-name> /review </command-name><command-args> src/a.js </command-args>')]);
rp('slash-command-blank-args', [user(1, '<command-name>/clear</command-name><command-args>   </command-args>')]);
rp('request-interrupted-note', [user(1, '[Request interrupted by user]'), user(2, '  [Request interrupted by user for tool use]')]);
rp('synthetic-user-messages-skipped', [user(1, '<local-command-stdout>x</local-command-stdout>'), user(2, 'Caveat: messages below were generated'), user(3, '<command-message>foo</command-message>'), user(4, '   '), user(5, 'real'), user(6, 'meta', { isMeta: true })]);
rp('ponytail-banner-typed-part', [user(1, 'PONYTAIL MODE ACTIVE - level: full\n\nlots of banner\n\nmore banner\n\nwhat I really typed')]);
rp('user-content-text-blocks-joined', [user(1, [txt('a'), { type: 'tool_use' }, txt('b')])]);
rp('tool-error-soft-vs-hard', [
  asst(1, 'm1', [tu('t1', 'Edit', { file_path: 'a' }), tu('t2', 'Bash', { command: 'false' }), tu('t3', 'Read', { file_path: 'x' })]),
  user(2, [
    { type: 'tool_result', tool_use_id: 't1', content: 'Edit: "old_string" was not found in /x/a.', is_error: true },
    { type: 'tool_result', tool_use_id: 't2', content: [txt('exit 1'), txt(' boom')], is_error: true },
    { type: 'tool_result', tool_use_id: 't3', content: 'Read: cannot access /x: ENOENT', is_error: true },
  ]),
]);
rp('unfinished-tool-is-interrupted', [asst(1, 'm1', [tu('t1', 'Bash', { command: 'sleep 9' }), tu('t2', 'Read', {})]), tr(2, 't2', 'done')]);
rp('tool-result-for-unknown-id-ignored', [tr(1, 'ghost', 'x'), user(2, 'hello')]);
rp('streamed-text-growth-keeps-each-length-once', [asst(1, 'm1', [txt('He')]), asst(1, 'm1', [txt('Hel')]), asst(1, 'm1', [txt('Hel')]), asst(1, 'm1', [txt('Hello')])]);
rp('queue-summary-final-text', [asst(1, 'q1', [txt('summary')], null, null, { synthetic: 'queue-summary' })]);
rp('empty-and-blank-text-skipped', [asst(1, 'm1', [txt('(empty)'), txt('  '), txt(''), { type: 'thinking', thinking: '' }, txt('real')])]);
rp('compact-boundary-and-handoff', [{ type: 'system', subtype: 'compact_boundary', trigger: 'auto', summary: 'S1', timestamp: T(1) }, { type: 'system', subtype: 'compact_boundary', trigger: 'manual', timestamp: T(2) }, { type: 'system', subtype: 'handoff', summary: 'H', bootstrap: true, timestamp: T(3) }, { type: 'system', subtype: 'other', timestamp: T(4) }]);
rp('garbage-entries-ignored', [null, 5, 'x', {}, { type: 'user' }, { type: 'assistant' }, { type: 'assistant', message: { content: 'str' } }, user(1, 'ok')]);
rp('limit-keeps-last-n-items', Array.from({ length: 6 }, (_, i) => user(i + 1, `m${i}`)), { limit: 2 });
rp('missing-timestamp-gives-ts-zero', [{ type: 'user', message: { role: 'user', content: 'no ts' } }, { type: 'assistant', message: { id: 'm', content: [tu('t1', 'Bash', {})] } }]);
rp('tool-input-missing-defaults-empty', [asst(1, 'm1', [{ type: 'tool_use', id: 't1', name: 'Bash' }])]);

// ---- parser session state (parser.test.js + edge cases) ----
const U = { input_tokens: 100, output_tokens: 10, cache_read_input_tokens: 50 };
rp('usage-streamed-lines-not-double-counted', [asst(1, 'm1', [{ type: 'thinking', thinking: '' }], U, 'tool_use'), asst(1, 'm1', [tu('t1', 'Bash', { command: 'ls' })], { ...U, output_tokens: 20 }, 'tool_use')]);
rp('usage-two-messages-accumulate-and-timeline', [asst(1, 'm1', [txt('a')], { input_tokens: 10, output_tokens: 5 }), asst(2, 'm2', [txt('b')], { input_tokens: 20, output_tokens: 7, cache_creation_input_tokens: 3 }), asst(3, 'm3', [txt('c')])]);
rp('tools-skills-agents-todos-errors', [
  user(0, 'build it'),
  asst(1, 'm1', [tu('t1', 'Skill', { skill: 'scout' }), tu('t2', 'Agent', { subagent_type: 'explorer', description: 'find files' }), tu('t3', 'TodoWrite', { todos: [{ content: 'a', status: 'completed' }, { content: 'b', status: 'in_progress', activeForm: 'Doing b' }] }), tu('t4', 'Bash', { command: 'false' })], null, 'tool_use'),
  tr(2, 't1'), tr(3, 't2'), tr(3, 't3'), tr(3, 't4', 'exit 1', true),
], { now: NOW(4) });
rp('phase-tool-running', [user(0, 'go'), asst(1, 'm1', [tu('t1', 'Bash', { command: 'sleep 1' })], null, 'tool_use')], { now: NOW(2) });
rp('phase-agents-running-recent', [asst(1, 'm1', [tu('t1', 'Agent', { description: 'bg' })], null, 'tool_use'), tr(2, 't1', 'launched', false, { toolUseResult: { status: 'async_launched', agentId: 'abc' } })], { now: NOW(60) });
rp('phase-agents-running-stale-over-10min', [asst(1, 'm1', [tu('t1', 'Agent', { description: 'bg' })], null, 'tool_use'), tr(2, 't1', 'launched', false, { toolUseResult: { status: 'async_launched', agentId: 'abc' } })], { now: NOW(2 + 11 * 60) });
rp('phase-idle-after-end-turn', [user(0, 'hi'), asst(1, 'm1', [txt('done')], null, 'end_turn')], { now: NOW(30) });
rp('phase-thinking-after-new-user-prompt', [asst(1, 'm1', [txt('done')], null, 'end_turn'), user(5, 'again')], { now: NOW(30) });
rp('phase-idle-after-5min-silence', [asst(1, 'm1', [txt('x')], null, 'tool_use'), user(2, 'p')], { now: NOW(2 + 6 * 60) });
rp('background-agent-async-stays-running', [asst(1, 'm1', [tu('t1', 'Agent', { description: 'bg' })]), tr(2, 't1', 'x', false, { toolUseResult: { status: 'async_launched', agentId: 'abc' } })]);
rp('agent-foreground-finishes-and-fails', [asst(1, 'm1', [tu('t1', 'Task', { subagent_type: 'planner', prompt: 'p'.repeat(100), run_in_background: false }), tu('t2', 'Agent', {})]), tr(5, 't1', 'r'), tr(6, 't2', 'err', true)]);
rp('task-create-update-list', [asst(1, 'm1', [tu('c1', 'TaskCreate', { subject: 'write tests' })]), tr(2, 'c1', 'ok', false, { toolUseResult: { task: { id: '7' } } }), asst(3, 'm2', [tu('u1', 'TaskUpdate', { taskId: '7', status: 'completed', subject: 'renamed', activeForm: 'Renaming' }), tu('u2', 'TaskUpdate', { id: 'zz', status: 'x' })])]);
rp('hooks-attachments-failures-in-feed', [
  { type: 'attachment', timestamp: T(1), attachment: { type: 'hook_success', hookName: 'PreToolUse:Bash', hookEvent: 'PreToolUse', exitCode: 0 } },
  { type: 'attachment', timestamp: T(2), attachment: { type: 'hook_blocking_error', hookName: 'PreToolUse:Bash', hookEvent: 'PreToolUse', exitCode: 2 } },
  { type: 'attachment', timestamp: T(3), attachment: { type: 'hook_cancelled' } },
  { type: 'attachment', timestamp: T(4), attachment: { type: 'other' } },
]);
rp('components-skill-mcp-tool-cmd', [user(1, '<command-name>/review</command-name>'), asst(2, 'm1', [tu('t1', 'mcp__github__create_issue', { title: 'x' }), tu('t2', 'Skill', { skill: 'ponytail', args: 'full' }), tu('t3', 'Grep', { pattern: 'foo' })]), tr(3, 't1'), tr(3, 't2', 'e', true)]);
rp('verify-tool-sets-last-verify', [asst(1, 'm1', [tu('t1', 'Bash', { command: 'npm test', description: 'run tests' })]), tr(2, 't1', 'fail', true), asst(3, 'm2', [tu('t2', 'Bash', { command: 'cargo test' })]), tr(4, 't2', 'ok')]);
rp('memory-write-counts-learn', [asst(1, 'm1', [tu('t1', 'Edit', { file_path: '/p/CLAUDE.md' }), tu('t2', 'Write', { file_path: '/p/memory/notes.md' }), tu('t3', 'Write', { file_path: '/p/src/a.js' })])]);
rp('soft-tool-error-not-counted', [asst(1, 'm1', [tu('t1', 'Edit', { file_path: 'a' })]), tr(2, 't1', 'Edit: "old_string" matches 3 times in /x/a. Add more', true)]);
rp('permission-mode-entry-ignored', [{ type: 'permission-mode', permissionMode: 'plan', timestamp: T(9) }, user(1, 'hi')]);
rp('prompt-counting-synthetic-excluded', [user(1, 'real one'), user(2, '<system-reminder>x</system-reminder>'), user(3, '[Request interrupted by user]'), user(4, 'Caveat: x'), user(5, '<command-name>/x</command-name>'), user(6, 'second', { isMeta: false }), user(7, 'meta', { isMeta: true }), user(8, 'side', { isSidechain: true })]);
rp('session-metadata-cwd-version-branch-model', [{ ...asst(1, 'm1', [txt('hi')]), cwd: '/work/proj', version: '1.2.3', gitBranch: 'main' }, { ...asst(2, 'm2', [txt('x')]), message: { id: 'm2', model: '<synthetic>', content: [txt('syn')] } }]);
rp('activity-first-text-per-turn-only', [user(1, 'q1'), asst(2, 'a', [txt('r1')]), asst(3, 'b', [txt('r1b')]), user(4, 'q2'), asst(5, 'c', [txt('r2')])]);
rp('feed-capped-at-200', Array.from({ length: 205 }, (_, i) => user(i + 1, `prompt ${i}`)));
rp('summarize-in-feed-long-text-ellipsized', [user(1, 'x'.repeat(300)), asst(2, 'm1', [txt('y  \n z'.repeat(60))])]);

// ---- parser pure helpers ----
add('parse-lines-corrupt-and-blank', { op: 'parseLines', text: '{"a":1}\n\n  \n{bad\n{"b":2}\n[1,2]\n"str"\n{"c":' });
add('summarize-tool-table', { op: 'summarizeTool', calls: [['Bash', { command: 'ls -la' }], ['Bash', { description: 'List  files', command: 'ls' }], ['PowerShell', { command: 'dir' }], ['Read', { file_path: '/a/b/c.js' }], ['Write', { file_path: 'C:\\x\\y.txt' }], ['NotebookEdit', { notebook_path: '/n/z.ipynb' }], ['Grep', { pattern: 'foo.*' }], ['Glob', { pattern: '**/*.rs' }], ['WebSearch', { query: 'rust' }], ['WebFetch', { url: 'https://x.test' }], ['Agent', { description: 'd' }], ['Task', { subagent_type: 'planner', description: 'plan it' }], ['Skill', { skill: 's', args: 'a b' }], ['TodoWrite', { todos: [1, 2] }], ['TodoWrite', {}], ['TaskCreate', { subject: 'S' }], ['TaskUpdate', { taskId: 3, status: 'completed' }], ['TaskUpdate', {}], ['SendMessage', { to: 'a', summary: 's' }], ['mcp__x__y', { foo: 1, bar: 'baz' }], ['Other', {}], ['Bash', { command: 'a'.repeat(200) }]] });
add('classify-tool-table', { op: 'classifyTool', calls: [['Agent', {}], ['Skill', {}], ['SendMessage', {}], ['Read', {}], ['Grep', {}], ['TodoWrite', {}], ['ExitPlanMode', {}], ['Edit', { file_path: '/p/CLAUDE.md' }], ['Write', { file_path: '/p/lessons.md' }], ['Edit', { file_path: 'C:\\p\\memory\\x.md' }], ['Edit', { file_path: '/p/a.js' }], ['Bash', { command: 'npm test' }], ['Bash', { command: 'ls', description: 'lint it' }], ['Bash', { command: 'cargo test' }], ['Bash', { command: 'terraform plan' }], ['Bash', { command: 'echo hi' }], ['PowerShell', { command: 'pytest -q' }], ['WebFetch', {}]] });
add('prompts-of-items-dedupes-and-skips-auto', { op: 'promptsOfItems', items: [{ kind: 'user', text: 'a' }, { kind: 'user', text: ' a ' }, { kind: 'text', text: 'x' }, { kind: 'user', text: 'b', auto: true }, { kind: 'user', text: '   ' }, { kind: 'user', text: 'c' }, { kind: 'user', text: 'a' }], max: 200 });
add('prompts-of-items-max', { op: 'promptsOfItems', items: [1, 2, 3, 4].map((n) => ({ kind: 'user', text: `p${n}` })), max: 2 });

// ---- file based loaders ----
const jl = (entries, tail = '') => entries.map((e) => JSON.stringify(e)).join('\n') + '\n' + tail;
const FILE_ENTRIES = [{ ...user(1, 'first prompt'), cwd: '/work/proj' }, asst(2, 'm1', [txt('ok'), tu('t1', 'Bash', { command: 'sleep' })], U, 'tool_use'), user(3, 'é 日本 second')];
add('file-load-all-items-skips-bad-lines', { op: 'loadFile', fn: 'loadAllTranscriptItems', content: jl(FILE_ENTRIES, '{not json\n') });
add('file-load-items-missing-file-empty', { op: 'loadFile', fn: 'loadAllTranscriptItems', content: null });
add('file-load-session-settles-open-tools', { op: 'loadFile', fn: 'loadTranscriptSession', content: jl(FILE_ENTRIES), now: NOW(700) });
add('file-load-activity', { op: 'loadFile', fn: 'loadTranscriptActivity', content: jl([user(1, 'q'), asst(2, 'm', [txt('reply')]), user(3, 'q2')]) });
add('file-session-meta-first-prompt-and-cwd', { op: 'loadFile', fn: 'sessionMeta', content: jl([user(1, '<system-reminder>x</system-reminder>'), { ...user(2, '  fix\n  the   bug '), cwd: '/work/p' }, user(3, 'later')]) });
add('file-session-meta-skips-tool-results-and-meta', { op: 'loadFile', fn: 'sessionMeta', content: jl([tr(1, 'x'), user(2, 'meta', { isMeta: true }), user(3, 'ok', { displayText: '/cmd arg' })]) });

exports.source = source;
exports.cases = () => out;
exports.run = async (i, sb) => {
  switch (i.op) {
    case 'replay': {
      const s = parser.createSession('sess');
      for (const e of i.entries) parser.applyEntry(s, e);
      return {
        items: hist.itemsFromEntries(i.entries, i.limit != null ? { limit: i.limit } : undefined),
        session: s,
        total_tokens: parser.totalTokens(s.tokens),
        phase: ((p) => ({ phase: p.phase, running: p.running.map((t) => t.id), runningAgents: p.runningAgents.map((a) => a.toolUseId) }))(parser.sessionPhase(s, i.now)),
      };
    }
    case 'parseLines': return parser.parseLines(i.text);
    case 'summarizeTool': return i.calls.map(([n, inp]) => ({ name: n, summary: parser.summarizeTool(n, inp) }));
    case 'classifyTool': return i.calls.map(([n, inp]) => ({ name: n, input: inp, cat: parser.classifyTool(n, inp) }));
    case 'promptsOfItems': return hist.promptsOfItems(i.items, i.max);
    case 'loadFile': {
      const file = path.join(sb.ws, 's1.jsonl');
      if (i.content !== null) fs.writeFileSync(file, i.content);
      if (i.fn === 'loadTranscriptSession') {
        const s = hist.loadTranscriptSession(file, { agents: false });
        return { session: s, phase: ((p) => p.phase)(parser.sessionPhase(s, i.now)) };
      }
      return hist[i.fn](file);
    }
    default: throw new Error(`unknown op ${i.op}`);
  }
};
