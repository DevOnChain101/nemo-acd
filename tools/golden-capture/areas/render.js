'use strict';
// Dashboard / agent-pane frames (render.renderFrame, agentview.renderAgent, tasksPanel, ...) plus theme/highlight/picker/dialog helpers.
// Inputs are plain JSON; `hydrate*` below turn them into the Node objects (documented in tests/golden/README.md).
// Per frame case: expected.json (plain lines + hit rects), expected.txt (stripAnsi lines, LF), expected.ansi.json (raw truecolor SGR + sgr.encode output).
const { ref } = require('../env');
const path = require('path');
const R = (m) => require(path.join(ref, 'src', m));
const render = R('render.js');
const agentview = R('agentview.js');
const theme = R('theme.js');
const parser = R('parser.js');
const sgr = R('sgr.js');
const { LineEditor } = R('editor.js');
const picker = R('picker.js');
const dialogs = R('dialogs.js');
const { highlight, langFromFile } = R('highlight.js');
const { stripAnsi } = theme;

const source = 'src/render.js + src/agentview.js + src/theme.js + src/highlight.js + src/picker.js + src/dialogs.js';
const out = [];
const add = (id, input, extra = {}) => out.push({ id, input, ...extra });

// ---- fixed clock + transcript builders ----
const NOW = Date.parse('2026-10-02T12:00:00.000Z');
const T = (s) => new Date(NOW - 600000 + s * 1000).toISOString();
const user = (s, content, x = {}) => ({ type: 'user', timestamp: T(s), message: { role: 'user', content }, ...x });
const asst = (s, id, content, usage, stop) => ({ type: 'assistant', timestamp: T(s), message: { id, model: 'claude-opus-5.5', content, ...(usage ? { usage } : {}), ...(stop ? { stop_reason: stop } : {}) } });
const tu = (id, name, input = {}) => ({ type: 'tool_use', id, name, input });
const tr = (s, id, content = 'ok', isError = false) => user(s, [{ type: 'tool_result', tool_use_id: id, content, is_error: isError }]);
const txt = (text) => ({ type: 'text', text });
const U = { input_tokens: 4000, output_tokens: 600, cache_read_input_tokens: 20000 };

// history_gen: `days` entries ending at `end`; the entry n days ago has tokens = n%7==0 ? 0 : base+n, output n%5, messages 3, tools 2, agents n%3==0?1:0, sessions 1.
function genHistory(g) {
  const end = Date.parse(g.end);
  const o = [];
  for (let n = g.days - 1; n >= 0; n--) {
    o.push({ date: new Date(end - n * 86400000), tokens: n % 7 === 0 ? 0 : (g.base ?? 1000) + n, output: n % 5, messages: 3, tools: 2, agents: n % 3 === 0 ? 1 : 0, sessions: 1 });
  }
  return o;
}
const HG = { end: '2026-10-02T12:00:00.000Z', days: 371 };

// session spec: { id, cwd, model, gitBranch, entries, set } -> parser.createSession + applyEntry, then `set` shallow-assigned.
function hydrateSession(s = {}) {
  const sess = parser.createSession(s.id || 'session-1');
  for (const k of ['cwd', 'model', 'gitBranch', 'version']) if (s[k] != null) sess[k] = s[k];
  for (const e of s.entries || []) parser.applyEntry(sess, e);
  Object.assign(sess, s.set || {});
  if (s.startedAt != null) sess.startedAt = s.startedAt;
  return sess;
}
const hydrateSessions = (list, n) => list || Array.from({ length: n || 0 }, (_, i) => ({
  id: `session-${i}-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa`, project: `proj-${i}`, cwd: `D:/workspace/project-${i}`, mtime: NOW - i * 1000,
  live: { name: `proj-${i}`, cwd: `D:/workspace/project-${i}` }, phase: 'idle', tokens: 1000 * (i + 1), ctxPct: 10,
}));

// agent spec: duck-typed NativeAgent subset the renderer reads.
function hydrateAgent(a, session) {
  const agent = {
    cwd: '/work/proj', mode: 'default', status: 'idle', effort: 'auto', model: 'anthropic/fake-sonnet', items: [], turnStart: 0, turnTokens: 0,
    info: { version: '1.0.0' }, live: session, contextWindow: 200000, ...a, pending: [], subagents: new Map(),
  };
  agent.pending = (a.pending || []).map((p) => {
    const q = { ...p };
    if (q.dialog === 'question' && q.questions) q.qstate = dialogs.questionState(q.questions);
    if (q.kind === 'permission' && q.dialog !== 'question' && !q.options) q.options = q.dialog === 'plan' ? dialogs.PLAN_OPTIONS : dialogs.permissionOptions(q);
    return q;
  });
  if (a.subagents) agent.subagents = new Map(a.subagents.map((s) => [s.id, { items: [], pending: [], mode: 'default', cwd: agent.cwd, ...s }]));
  return agent;
}
function hydrateEditor(e) {
  const ed = new LineEditor();
  if (e && e.text) ed.insert(e.text);
  return ed;
}
function hydratePicker(p) {
  if (!p) return null;
  const x = picker.createPicker({ id: p.id || 'p', title: p.title, subtitle: p.subtitle, items: p.items });
  Object.assign(x, p.set || {});
  return x;
}

function hydrateState(st) {
  const session = hydrateSession(st.session);
  const state = {
    session, sessions: hydrateSessions(st.sessions, st.sessions_n), index: st.index || 0, usage: st.usage || { tap: null, fiveHour: null, sevenDay: null }, follow: false,
    history: st.history_gen ? genHistory(st.history_gen) : [], usageMetric: st.usageMetric || 'tokens',
  };
  if (st.mcpServers) state.mcpServers = st.mcpServers;
  if (st.workspaces) state.workspaces = st.workspaces;
  if (st.activeWorkspace != null) state.activeWorkspace = st.activeWorkspace;
  if (st.agent) {
    state.agent = hydrateAgent(st.agent, session);
    state.chatAgent = state.agent;
    state.editor = hydrateEditor(st.editor);
    state.focus = st.focus || 'agent';
    state.agentView = { ...(st.agentView || {}) };
    if (st.agentView && st.agentView.picker) state.agentView.picker = hydratePicker(st.agentView.picker);
  }
  return state;
}

// ---- input constructors ----
const frameCase = (id, cols, rows, state, o = {}) => add(id, { op: 'frame', cols, rows, frame: o.frame || 0, now: o.now || NOW, state }, o.note ? { note: o.note } : {});
const agentCase = (id, w, h, agent, o = {}) => add(id, { op: 'agent', w, h, frame: o.frame || 0, now: o.now || NOW, agent, editor: o.editor, opts: o.opts || {}, session: o.session }, o.note ? { note: o.note } : {});
const base = (extra = {}) => ({ session: { id: 'session-1', cwd: 'D:/workspace/project', model: 'claude-sonnet', startedAt: NOW - 60000 }, sessions_n: 3, history_gen: HG, ...extra });
const mcp = (name, x = {}) => ({ name, type: 'stdio', command: 'node', scope: 'project', source: '/x/.mcp.json', enabled: true, ...x });

// ---- A. dashboard frames (no agent) ----
const SIZES = [[80, 24], [120, 40], [40, 20]]; // 40x20 is clamped to MIN_COLS/MIN_ROWS by renderFrame
const busyEntries = [
  user(0, 'refactor the parser'),
  asst(1, 'm1', [txt('Looking.'), tu('t1', 'Read', { file_path: 'src/a.js' }), tu('t2', 'Bash', { command: 'npm test' }), tu('t3', 'Skill', { skill: 'scout' }), tu('t4', 'Agent', { subagent_type: 'explorer', description: 'find files' }), tu('t5', 'TodoWrite', { todos: [{ content: 'Read', status: 'completed' }, { content: 'Fix', status: 'in_progress', activeForm: 'Fixing' }, { content: 'Test', status: 'pending' }] })], U, 'tool_use'),
  tr(2, 't1'), tr(3, 't3'), tr(4, 't5'), tr(5, 't2', 'exit 1\nboom', true),
  asst(6, 'm2', [txt('Second reply with **bold** text.')], { input_tokens: 5000, output_tokens: 900 }, 'end_turn'),
];
for (const [c, r] of SIZES) {
  frameCase(`dash-empty-session-${c}x${r}`, c, r, { session: { id: 'session-1', cwd: '/work/proj' }, history_gen: HG });
  frameCase(`dash-busy-tools-${c}x${r}`, c, r, base({ session: { id: 'session-1', cwd: 'D:/workspace/project', model: 'claude-opus-5.5', gitBranch: 'main', entries: busyEntries }, mcpServers: [mcp('alpha'), mcp('pending', { enabled: false, disabledReason: 'not approved' })] }));
}
frameCase('dash-wide-200x60-many-sessions-40', 200, 60, base({ sessions_n: 40, index: 20 }));
frameCase('dash-short-sessions-scroll-index-39-180x30', 180, 30, base({ sessions_n: 40, index: 39 }));
frameCase('dash-sessions-cap-half-height-220x80', 220, 80, base({ sessions_n: 40 }));
frameCase('dash-ctx-pct-over-100-bounded', 180, 60, base({ sessions: [{ id: 'session-0-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', project: 'proj-0', cwd: 'D:/workspace/project-0', mtime: NOW, live: { name: 'proj-0', cwd: 'D:/workspace/project-0' }, phase: 'idle', tokens: 1000, ctxPct: 113 }] }));
frameCase('dash-running-title-first-200x40', 200, 40, base({ sessions: [{ id: 'session-title-aaaaaaaaaaaaaaaa', project: 'project', cwd: 'D:/workspace/project', mtime: NOW, live: { name: 'first user message', cwd: 'D:/workspace/project', status: 'idle' }, phase: 'idle' }] }));
frameCase('dash-no-quota-rows-160x44', 160, 44, { session: { id: 'session-1', cwd: '/work/proj' }, history_gen: HG, sessions: [] });
frameCase('dash-post-compact-reset-ctx-zero-160x44', 160, 44, { session: { id: 'session-1', cwd: '/work/proj', set: { contextUsed: 160000, contextDisplayReset: true } }, usage: { tap: { contextPct: 80, contextSize: 200000 }, fiveHour: null, sevenDay: null }, history_gen: HG, sessions: [] });
frameCase('dash-quota-and-cost-chips-160x44', 160, 44, base({ usage: { tap: { contextPct: 42, contextSize: 200000, costUsd: 1.234, model: 'claude-x' }, fiveHour: { used: 61 }, sevenDay: { used: 20 } } }));
frameCase('dash-mcp-three-servers-short-220x20', 220, 20, base({ mcpServers: ['alpha', 'beta', 'gamma'].map((n) => mcp(n)) }));
frameCase('dash-frame-animated-busy-frame7', 120, 40, base({ session: { id: 'session-1', cwd: '/work/proj', entries: busyEntries }, sessions: [{ id: 'session-0-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', project: 'proj-0', cwd: '/work/proj', mtime: NOW, live: { name: 'proj-0', cwd: '/work/proj' }, phase: 'tool', tokens: 5000, ctxPct: 33 }] }), { frame: 7 });
frameCase('dash-cjk-emoji-names-120x40', 120, 40, base({ session: { id: 'session-1', cwd: '/work/日本語-プロジェクト', model: 'claude-sonnet', entries: [user(0, '日本語の質問 😀 です'), asst(1, 'm1', [txt('回答 ✅'), tu('t1', 'Bash', { command: 'echo 日本😀' })], U, 'tool_use')] }, sessions: [{ id: 'session-0-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', project: '日本語', cwd: '/work/日本語', mtime: NOW, live: { name: '日本語セッション😀', cwd: '/work/日本語' }, phase: 'idle' }] }));
frameCase('dash-long-lines-narrow-60x24', 60, 24, base({ session: { id: 'session-1', cwd: '/very/long/path/' + 'segment/'.repeat(12), model: 'provider/' + 'm'.repeat(80), gitBranch: 'feature/' + 'x'.repeat(60), entries: busyEntries } }));
frameCase('dash-single-day-history-120x40', 120, 40, { session: { id: 'session-1', cwd: '/work/proj' }, history_gen: { end: '2026-10-02T12:00:00.000Z', days: 1 }, sessions: [] });
frameCase('dash-layout-right-landscape-260x50', 260, 50, base());
frameCase('dash-layout-bottom-100x50', 100, 50, base());
frameCase('dash-layout-none-80x16', 80, 16, base());
frameCase('dash-usage-metric-tools-120x40', 120, 40, base({ usageMetric: 'tools' }));
frameCase('dash-clock-end-of-year-header', 120, 40, base(), { now: Date.parse('2026-12-31T23:59:59.000Z') });

// ---- B. agent pane (renderAgent) ----
const chat = [
  { kind: 'user', text: 'fix the flaky login test' },
  { kind: 'thinking', text: 'look', start: NOW - 3000, end: NOW - 1000 },
  { kind: 'text', text: 'I will check **login.ts** and `retry()`:\n- first\n- second' },
  { kind: 'tool', name: 'Bash', input: { command: 'npm test', description: 'Run tests' }, status: 'error', result: 'Exit code 1\nFAIL login.test.ts\nexpected 2' },
  { kind: 'tool', name: 'Read', input: { file_path: 'D:/p/src/login.ts' }, status: 'running' },
];
const many = Array.from({ length: 80 }, (_, i) => ({ kind: i % 2 ? 'text' : 'user', text: `message number ${i} ` + 'x'.repeat(i) }));
const bigOut = Array.from({ length: 20 }, (_, i) => `line ${i} ` + 'w'.repeat(i * 4)).join('\n');
agentCase('agent-welcome-empty-70x24', 70, 24, {});
agentCase('agent-welcome-narrow-37x20', 37, 20, {});
agentCase('agent-chat-running-todos-70x34', 70, 34, { items: chat, status: 'running', turnStart: NOW - 4000, mode: 'acceptEdits' }, { editor: { text: '/rev' }, session: { set: { todos: [{ content: 'Run tests', status: 'pending' }, { content: 'Fix race', activeForm: 'Fixing race', status: 'in_progress' }] } }, opts: { suggestions: { token: { kind: 'command' }, items: [{ label: '/review', detail: 'Review a PR' }] }, attachments: [{ name: 'next.png', bytes: 5000, width: 10, height: 10 }] } });
agentCase('agent-chat-idle-wide-120x40', 120, 40, { items: chat.slice(0, 4), mode: 'plan' });
for (const s of [0, 10, 9999]) agentCase(`agent-scrolled-window-50x20-scroll${s}`, 50, 20, { items: many }, { now: 1, opts: { scroll: s } });
agentCase('agent-status-thinking-running-90x20', 90, 20, { status: 'running', turnStart: 5000000, turnTokens: 12500, items: [{ kind: 'text', text: 'hi' }, { kind: 'thinking', text: 'hmm', streaming: true, start: 5000000 }] }, { now: 5000000 + 8 * 60000 + 10000 });
agentCase('agent-spinner-frame9-running-tool-70x20', 70, 20, { items: [{ kind: 'tool', name: 'Bash', input: { command: 'sleep 9' }, status: 'running' }] }, { frame: 9 });
agentCase('agent-busy-note-frame9-70x20', 70, 20, { items: [{ kind: 'note', text: 'checking claude config…', busy: true }] }, { frame: 9 });
agentCase('agent-bypass-confirmation-70x20', 70, 20, { mode: 'auto' }, { opts: { bypassConfirmation: true } });
agentCase('agent-permission-bash-70x34', 70, 34, { items: chat.slice(0, 2), pending: [{ kind: 'permission', tool: 'Bash', input: { command: 'rm -rf build', description: 'Clean build' }, suggestions: [{ type: 'addRules', rules: [{ toolName: 'Bash', ruleContent: 'rm:*' }] }], status: 'pending', choice: 1 }] });
agentCase('agent-question-dialog-72x36', 72, 36, { pending: [{ kind: 'permission', dialog: 'question', status: 'pending', questions: [{ question: 'Which colour?', header: 'Colour', multiSelect: false, options: [{ label: 'Red', description: 'warm' }, { label: 'Blue', description: 'cool' }] }], qi: 0, options: [] }] });
agentCase('agent-question-multiselect-review-72x30', 72, 30, { pending: [{ kind: 'permission', dialog: 'question', status: 'pending', questions: [{ question: 'Pick colours', header: 'Colours', multiSelect: true, options: [{ label: 'Red' }, { label: 'Blue' }] }, { question: 'Pick size', header: 'Size', multiSelect: false, options: [{ label: 'Small' }, { label: 'Large' }] }], qi: 2, options: [] }] });
agentCase('agent-plan-dialog-72x36', 72, 36, { pending: [{ kind: 'permission', dialog: 'plan', status: 'pending', input: { plan: '# Add README\n1. **draft** it\n2. commit' }, choice: 0 }] });
agentCase('agent-markdown-table-cjk-28x30', 28, 30, { items: [{ kind: 'text', text: '| Name | Score |\n| :--- | ---: |\n| **Việt Nam** | 42 |\n| 東京😀 | `longunbrokentoken` |' }] });
agentCase('agent-markdown-table-narrow-14x40', 14, 40, { items: [{ kind: 'text', text: 'A | B | C\n:---:|---|---:\nx | supercalifragilisticexpialidocious | z' }] });
agentCase('agent-markdown-code-fence-lists-60x30', 60, 30, { items: [{ kind: 'text', text: '# Title\n\n1. one\n2. two\n\n```js\nconst a = 1; // c\nfunction f(x) { return x + "s"; }\n```\n\n> quote\n\n**bold** *it* `code` [link](http://x)' }] });
agentCase('agent-tool-output-wrapped-expanded-60x30', 60, 30, { items: [{ kind: 'tool', name: 'Bash', input: { command: 'cat big' }, status: 'done', result: bigOut }] }, { opts: { expanded: true } });
agentCase('agent-tool-output-collapsed-60x30', 60, 30, { items: [{ kind: 'tool', name: 'Bash', input: { command: 'cat big' }, status: 'done', result: bigOut }] });
agentCase('agent-edit-diff-70x30', 70, 30, { items: [{ kind: 'tool', name: 'Edit', input: { file_path: 'src/a.js', old_string: 'const a = 1;\nconst b = 2;', new_string: 'const a = 10;\nconst b = 2;\nconst c = 3;' }, status: 'done', result: 'ok' }] });
agentCase('agent-user-multiline-queued-48x24', 48, 24, { items: [{ kind: 'user', text: `/ponytail full\r\n${'instruction '.repeat(30)}\nlast line`, queued: true }] });
agentCase('agent-cjk-emoji-wrap-40x24', 40, 24, { items: [{ kind: 'user', text: '日本語のとても長い質問がここにあります。折り返しを確認します😀😀😀' }, { kind: 'text', text: '回答です ✅ これも長い文章で折り返しが必要になるはずです。👨‍👩‍👧 family' }] });
agentCase('agent-long-unbroken-token-30x20', 30, 20, { items: [{ kind: 'text', text: 'x'.repeat(200) }, { kind: 'user', text: 'y'.repeat(100) }] });
agentCase('agent-input-multiline-editor-60x20', 60, 20, {}, { editor: { text: 'line one\nline two that is fairly long and should wrap around the prompt box edge' } });
agentCase('agent-input-bash-mode-60x20', 60, 20, {}, { editor: { text: 'ls -la' }, opts: { inputMode: 'bash' } });
agentCase('agent-history-search-60x20', 60, 20, {}, { opts: { search: { query: 'fix', matches: ['fix the flaky login test'], index: 0 } } });
agentCase('agent-focused-false-60x20', 60, 20, {}, { editor: { text: 'abc' }, opts: { focused: false } });
agentCase('agent-subagent-selector-70x20', 70, 20, { subagents: [{ id: 'sub-1', type: 'worker', def: { name: 'coordinator' }, status: 'running', description: 'focused task', model: 'openai/gpt-5-mini', effort: 'high' }] }, { opts: { chatTarget: 'sub-1' } });
const PICK = { id: 'model', title: 'Select model', subtitle: 'pick a model to use', items: [{ label: 'Opus (fake)', detail: 'most capable', value: 'opus', current: true }, { label: 'Sonnet (fake)', detail: 'balanced', value: 'sonnet' }] };
for (const w of [37, 70]) agentCase(`picker-model-${w}x30`, w, 30, {}, { opts: { picker: PICK } });
agentCase('picker-loading-frame2-70x30', 70, 30, {}, { frame: 2, opts: { picker: { ...PICK, set: { loading: true, loadingText: 'Checking Claude config…' } } } });
agentCase('picker-filter-scroll-70x16', 70, 16, {}, { opts: { picker: { id: 'many', title: 'Many', items: Array.from({ length: 30 }, (_, i) => ({ label: `item-${i}`, detail: `detail ${i}`, value: i })), set: { query: 'item-', choice: 15, offset: 10 } } } });
agentCase('picker-short-detail-100x30', 100, 30, {}, { opts: { picker: { id: 'import', title: 'Import config', items: [{ label: 'claude', detail: 'found · ready to import', value: 'claude' }] } } });

// ---- C. dashboard with agent (renderFrame + state.agent) ----
const agentState = (o = {}) => ({ session: { id: 'session-1', cwd: '/work/proj', model: 'claude-sonnet', startedAt: NOW - 60000, entries: busyEntries }, sessions_n: 2, history_gen: HG, agent: { items: chat, status: 'running', turnStart: NOW - 4000 }, editor: { text: 'hello' }, ...o });
for (const [c, r] of [[120, 40], [80, 24], [200, 50]]) frameCase(`frame-agent-chat-${c}x${r}`, c, r, agentState());
frameCase('frame-agent-permission-120x40', 120, 40, agentState({ agent: { items: chat.slice(0, 2), pending: [{ kind: 'permission', tool: 'Bash', input: { command: 'ls' }, suggestions: [], status: 'pending', choice: 0 }] } }));
frameCase('frame-agent-picker-120x40', 120, 40, agentState({ agentView: { picker: PICK } }));

// ---- D. panels / helpers ----
const todo = (content, status) => ({ content, status, activeForm: content });
const panelState = (todos) => ({ session: { set: { todos } }, history_gen: HG, usageMetric: 'tokens' });
add('panel-tasks-order-numbered-50x10', { op: 'tasks', w: 50, h: 10, state: panelState([todo('Upgrade LoginService', 'completed'), todo('Refactor auth', 'pending'), todo('Write tests', 'in_progress')]) });
add('panel-tasks-overflow-in-progress-visible-50x7', { op: 'tasks', w: 50, h: 7, state: panelState([todo('done-1', 'completed'), todo('done-2', 'completed'), todo('done-3', 'completed'), todo('doing', 'in_progress'), todo('next-1', 'pending'), todo('next-2', 'pending'), todo('next-3', 'pending')]) });
add('panel-tasks-empty-50x6', { op: 'tasks', w: 50, h: 6, state: panelState([]) });
add('panel-tasks-index-width-12-50x20', { op: 'tasks', w: 50, h: 20, state: panelState(Array.from({ length: 12 }, (_, i) => todo(`task-${i + 1}`, 'pending'))) });
for (const w of [24, 36, 60, 90]) add(`panel-usage-${w}x11`, { op: 'usage', w, h: 11, state: panelState([todo('a', 'pending')]) });
add('panel-usage-year-130x17', { op: 'usage', w: 130, h: 17, state: panelState([todo('x', 'completed')]) });
for (const w of [60, 120]) for (const h of [4, 11]) add(`panel-tasks-usage-row-${w}x${h}`, { op: 'tasksUsage', w, h, state: panelState([todo('a very long task title that must be clipped to the box width of the panel', 'in_progress'), todo('b', 'pending')]) });
add('todo-window-cases', { op: 'todoWindow', cases: [{ todos: ['completed', 'completed', 'pending', 'pending', 'pending'], cap: 5 }, { todos: ['completed', 'completed', 'pending', 'pending', 'pending'], cap: 3 }, { todos: ['completed', 'completed', 'completed', 'completed', 'completed'], cap: 2 }, { todos: ['pending', 'pending', 'pending', 'in_progress'], cap: 2 }] });
add('heat-level-table', { op: 'heatLevel', cases: [[-5, 100], [null, 100], [5, 0], [1e308, 5], [100, 100], [5, null], [5, -1], [null, 3], [1, 1e9], [50, 100], [1, 100], [25, 100]] });
add('layout-for-table', { op: 'layoutFor', sizes: [[60, 16], [80, 24], [100, 50], [120, 40], [200, 50], [260, 50], [140, 30], [90, 20]] });
add('fmt-helpers', { op: 'fmt', nums: [null, 0, 999, 1000, 1500, 999999, 1e6, 2.5e9], durs: [null, -1, 0, 59999, 60000, 3599000, 3600000, 86400000, 90000000] });
add('sessions-window-table', { op: 'sessionsWindow', cases: [[0, 0, 5], [3, 0, 6], [40, 20, 8], [40, 39, 8], [40, 0, 1], [5, 4, 100]] });
add('bar-widths', { op: 'bar', cases: [[0, 10], [50, 10], [100, 10], [33, 7], [150, 5], [-5, 5], [null, 4], [80, 0]] });

// ---- E. theme / highlight / dialogs ----
add('theme-width-fit', { op: 'width', strs: ['', 'abc', '日本語', '😀', 'é', 'a\u0301', '👨‍👩‍👧', '🇯🇵', '❤️', '✻', '\x1b[38;2;1;2;3mred\x1b[0m', '┏━┓'], fits: [['abcdef', 3], ['日本語', 5], ['abc', 6], ['\x1b[38;2;1;2;3mredgreen\x1b[0m', 4], ['😀😀😀', 5]] });
add('theme-color-fns', { op: 'colors', lerp: [[[0, 0, 0], [255, 255, 255], 0.5], [[10, 20, 30], [200, 100, 50], 0.25]], cycle: [0, 0.25, 0.5, 0.75, 1.3, -0.2], pulse: [0, 5, 10, 15, 25], agentTypes: ['', 'explorer', 'planner', 'developer', 'tester', 'x'], dots: [0, 3, 6, 9, 12, 100], waiting: [['Checking config…', 6], ['loading...', 3], ['plain', 9]] });
for (const [k, lang, code] of [
  ['js', 'js', 'const a = 1; // c\nfunction f(x) { return `t${x}` + "s" + \'q\'; }\n/* block\nspans */ let y = null;'],
  ['py', 'python', 'def f(x):\n    """doc"""\n    return x # c\nclass A: pass'],
  ['sh', 'bash', 'if [ -f "$X" ]; then echo $HOME; fi # c'],
  ['go', 'go', 'func main() { fmt.Println("hi") }'],
  ['json', 'json', '{"a": [1, true, null], "b": "s"}'],
  ['diff', 'diff', '--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new'],
  ['sql', 'sql', 'SELECT count(*) FROM t WHERE a = 1 -- c'],
  ['unknown-lang', 'brainfuck', '+++[->+<]'],
  ['empty', 'js', ''],
]) add(`highlight-${k}`, { op: 'highlight', lang, code });
add('highlight-lang-from-file', { op: 'langFromFile', files: ['a.js', 'A.TS', 'x.py', 'main.rs', 'Makefile', 'a.tar.gz', 'c:\\x\\y.json', 'x.yml', '', null] });
add('dialogs-modes-options-picker-state', { op: 'dialogs' });

exports.source = source;
exports.cases = () => out;

const frameOut = (frame) => {
  const lines = typeof frame === 'string' ? frame.split('\n') : frame;
  return { lines: lines.map(stripAnsi), ansi: lines };
};
exports.run = async (i) => {
  switch (i.op) {
    case 'frame': {
      const state = hydrateState(i.state);
      const f = render.renderFrame(state, i.cols, i.rows, i.frame, i.now);
      const av = state.agentView || {};
      const rects = {};
      for (const k of ['chatRect', 'sidebarRect', 'dockRect', 'activityRect', 'workspaceRect', 'sessionRows', 'workspaceRows']) if (av[k]) rects[k] = av[k];
      return { ...frameOut(f), cols: i.cols, rows: i.rows, rects };
    }
    case 'agent': {
      const session = hydrateSession({ id: 'session-1', ...(i.session || {}) });
      const agent = hydrateAgent(i.agent, session);
      const opts = { ...i.opts };
      if (opts.picker) opts.picker = hydratePicker(opts.picker);
      if (agent.subagents.size) opts.subagents = agent.subagents;
      const res = agentview.renderAgent(agent, hydrateEditor(i.editor), i.w, i.h, i.frame, i.now, opts);
      const hits = agent.pending[0] && agent.pending[0]._questionHits;
      return { ...frameOut(res.lines), maxScroll: res.maxScroll, w: i.w, h: i.h, ...(hits ? { question_hits: hits } : {}) };
    }
    case 'tasks': return frameOut(render.tasksPanel(hydrateState(i.state), i.w, i.h, 0));
    case 'usage': return frameOut(render.usagePanel(hydrateState(i.state), i.w, i.h, { valign: 'evenly' }));
    case 'tasksUsage': {
      const r = render.tasksUsageRow(hydrateState(i.state), i.w, i.h, 0, NOW);
      return { ...frameOut(r.rows), tasks: r.tasks, usage: r.usage };
    }
    case 'todoWindow': return i.cases.map((c) => ({ ...c, start: agentview.todoWindow(c.todos.map((status) => ({ content: 'x', status })), c.cap) }));
    case 'heatLevel': return i.cases.map(([v, max]) => ({ v, max, level: render.heatLevel(v === null ? undefined : v, max === null ? NaN : max) }));
    case 'layoutFor': return i.sizes.map(([w, h]) => ({ w, h, layout: render.layoutFor(w, h) }));
    case 'fmt': return { nums: i.nums.map((n) => ({ n, s: render.fmtNum(n) })), durs: i.durs.map((n) => ({ ms: n, s: render.fmtDur(n) })) };
    case 'sessionsWindow': return i.cases.map(([count, index, cap]) => ({ count, index, cap, win: render.sessionsWindow(count, index, cap) }));
    case 'bar': return i.cases.map(([p, w]) => ({ p, w, ansi: render.bar(p, w, 0, false), plain: stripAnsi(render.bar(p, w, 0, false)) }));
    case 'width': return { widths: i.strs.map((s) => ({ s, width: theme.width(s), plain: stripAnsi(s) })), fits: i.fits.map(([s, w]) => ({ s, w, out: theme.fit(s, w), width: theme.width(theme.fit(s, w)) })) };
    case 'colors': return {
      lerp: i.lerp.map(([a, b, t]) => theme.lerp(a, b, t)), cycle: i.cycle.map((t) => theme.cycle(theme.BORDER_GRAD, t)), pulse: i.pulse.map((f) => theme.pulse([0, 0, 0], [200, 100, 50], f)),
      agent_colors: i.agentTypes.map((t) => ({ type: t, rgb: theme.agentColor(t) })), dots: i.dots.map((f) => theme.dots(f)), waiting: i.waiting.map(([t, f]) => theme.waiting(t, f)),
      palette: { THEME: theme.THEME, ACCENT: theme.ACCENT, VIVID: theme.VIVID, WARM: theme.WARM, HEAT: theme.HEAT, SELECT_BG: theme.SELECT_BG, SELECT_FG: theme.SELECT_FG, BORDER_GRAD: theme.BORDER_GRAD, LOGO_GRAD: theme.LOGO_GRAD, AGENT_COLORS: theme.AGENT_COLORS, CLAUDE_SPIN: theme.CLAUDE_SPIN },
    };
    case 'highlight': return { lines: highlight(i.code, i.lang) };
    case 'langFromFile': return i.files.map((f) => ({ file: f, lang: langFromFile(f) }));
    case 'dialogs': {
      const p = picker.createPicker({ id: 'p', title: 't', items: ['alpha', 'beta', 'gamma', 'alphabet'].map((label) => ({ label, detail: label.toUpperCase(), value: label })) });
      const steps = [];
      const snap = (tag) => steps.push({ tag, query: p.query, choice: p.choice, offset: p.offset, items: picker.filteredItems(p).map((x) => x.label) });
      snap('initial');
      picker.typePicker(p, 'a'); snap('type a');
      picker.typePicker(p, 'l'); snap('type l');
      picker.movePicker(p, 1); snap('down');
      picker.movePickerEnd(p); snap('end');
      picker.movePickerHome(p); snap('home');
      picker.backspacePicker(p); snap('backspace');
      return {
        MODES: dialogs.MODES, VALID_MODES: dialogs.VALID_MODES, PLAN_OPTIONS: dialogs.PLAN_OPTIONS, OUTSIDE_READ_OPTIONS: dialogs.OUTSIDE_READ_OPTIONS, LOCAL_COMMANDS: dialogs.LOCAL_COMMANDS,
        normalizeMode: ['default', 'auto', 'plan', 'acceptEdits', 'bypassPermissions', 'dontAsk', 'bogus', '', null].map((m) => ({ m, v: dialogs.normalizeMode(m), name: dialogs.modeName(dialogs.normalizeMode(m)) })),
        permissionOptions: [{ tool: 'Bash', input: { command: 'ls' }, suggestions: [] }, { tool: 'Bash', input: { command: 'rm -rf x' }, suggestions: [{ type: 'addRules', rules: [{ toolName: 'Bash', ruleContent: 'rm:*' }] }] }, { tool: 'Write', input: { file_path: '/a' }, suggestions: [{ type: 'setMode', mode: 'acceptEdits' }] }].map((it) => dialogs.permissionOptions({ kind: 'permission', ...it })),
        questionState: dialogs.questionState([{ question: 'q', header: 'h', multiSelect: true, options: [{ label: 'a' }] }]),
        picker_steps: steps,
      };
    }
    default: throw new Error(i.op);
  }
};

// capture.js hook: split frame results into expected.json + expected.txt + expected.ansi.json.
exports.finish = (expected) => {
  if (!expected || !Array.isArray(expected.lines) || !Array.isArray(expected.ansi)) return { expected, files: {} };
  const { ansi, ...rest } = expected;
  return {
    expected: rest,
    files: {
      'expected.txt': expected.lines.join('\n') + '\n',
      'expected.ansi.json': { color_mode: 'truecolor', lines: ansi, encoded_truecolor: ansi.map((l) => sgr.encode(l, 'truecolor')) },
    },
  };
};
