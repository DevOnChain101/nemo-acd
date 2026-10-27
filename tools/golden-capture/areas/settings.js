'use strict';
// claudecfg.loadWorkspace (settings merge, readUserConfig, hook sections, memory/agents/skills/commands/mcp discovery) and the user-only loaders.
// Files are written into the sandbox (<HOME>, <TMP>/ws, ...); `env` is passed explicitly to loadWorkspace, `home` is always <HOME>.
const { ref } = require('../env');
const path = require('path');
const cc = require(path.join(ref, 'src/native/claudecfg.js'));

const source = 'src/native/claudecfg.js';
const out = [];
const add = (id, input, extra = {}) => out.push({ id, input, ...extra });
const H = '<HOME>';
const WS = '<TMP>/ws';
const nemo = (o) => ({ [`${H}/.nemo/settings.json`]: o });
const proj = (o) => ({ [`${WS}/.nemo/settings.json`]: o });
const cl = (o) => ({ [`${H}/.claude/settings.json`]: o });
const rc = { [`${H}/.nemo/nemo.json`]: { claude: { readUserConfig: true } } };
// op ws: loadWorkspace({cwd, home: <HOME>, env})
const ws = (id, files, o = {}) => add(id, { op: 'workspace', files, env: o.env || {}, cwd: o.cwd || WS, ...(o.dirs ? { dirs: o.dirs } : {}) }, o.note ? { note: o.note } : {});
const hook = (matcher, command) => ({ matcher, hooks: [{ type: 'command', command }] });

ws('no-files-defaults', {});
ws('merge-permissions-low-to-high-drops-env-and-apikeyhelper', {
  ...nemo({ permissions: { allow: ['Read'], deny: [], defaultMode: 'default' }, env: { ANTHROPIC_API_KEY: 'should-not-appear' }, apiKeyHelper: 'should-not-appear-either' }),
  ...proj({ permissions: { allow: ['Bash(git:*)'], deny: ['Bash(rm:*)'], defaultMode: 'acceptEdits' } }),
  [`${WS}/.claude/settings.json`]: { permissions: { allow: ['Write'] } },
});
ws('permission-lists-deduped-additional-dirs-concatenated', {
  ...nemo({ permissions: { allow: ['Read', 'Read', 'Grep'], ask: ['Bash'], additionalDirectories: ['/a', '/b'] } }),
  ...proj({ permissions: { allow: ['Grep', 'Glob'], ask: ['Bash', 'Edit'], additionalDirectories: ['/b', '/c'] } }),
});
ws('malformed-json-warns-and-defaults', { [`${WS}/.nemo/settings.json`]: '{ not valid json' });
ws('malformed-user-file-does-not-block-project', { [`${H}/.nemo/settings.json`]: '{{', ...proj({ permissions: { allow: ['Read'] } }) });
ws('top-level-non-object-ignored', { ...nemo([1, 2]), ...proj('str') });
ws('permissions-wrong-types-ignored', nemo({ permissions: { allow: 'Read', deny: { a: 1 }, ask: 5, defaultMode: 7, additionalDirectories: null } }));
ws('hooks-concatenated-across-files-keep-source', {
  ...nemo({ hooks: { PreToolUse: [hook('Bash', 'echo user')], Stop: [{ hooks: [{ type: 'command', command: 'echo stop' }] }] } }),
  ...proj({ hooks: { PreToolUse: [hook('Bash', 'echo project')] } }),
});
ws('hooks-non-array-event-and-non-array-hooks-tolerated', {
  ...nemo({ hooks: { PreToolUse: 'nope', PostToolUse: [{ matcher: 'x', hooks: 'bad' }, { matcher: 'y' }], Stop: [] } }),
});
ws('disable-all-hooks-last-boolean-wins', { ...nemo({ disableAllHooks: true }), ...proj({ disableAllHooks: 'yes' }) });
ws('disable-all-hooks-project-can-reenable', { ...nemo({ disableAllHooks: true }), ...proj({ disableAllHooks: false }) });
ws('mcp-approval-lists-and-flag', { ...nemo({ enabledMcpjsonServers: ['a', 'b'], disabledMcpjsonServers: ['c'], enableAllProjectMcpServers: true }), ...proj({ enabledMcpjsonServers: ['b', 'd'], enableAllProjectMcpServers: false }) });
for (const bad of ['auto', 'bypassPermissions']) {
  ws(`project-default-mode-${bad}-ignored-user-plan-kept`, { ...nemo({ permissions: { defaultMode: 'plan' } }), ...proj({ permissions: { defaultMode: bad } }) });
}
ws('project-default-mode-accept-edits-applies', { ...nemo({ permissions: { defaultMode: 'plan' } }), ...proj({ permissions: { defaultMode: 'acceptEdits' } }) });
ws('project-default-mode-manual-passed-through', proj({ permissions: { defaultMode: 'manual' } }));
ws('user-default-mode-auto-keeps-working', nemo({ permissions: { defaultMode: 'auto' } }));
ws('project-forbidden-mode-only-ignored-when-no-user-mode', proj({ permissions: { defaultMode: 'auto' } }));
ws('claude-user-settings-not-read-by-default', { ...cl({ permissions: { allow: ['ClaudeOnly'] }, hooks: { Stop: [hook(undefined, 'echo c')] } }), ...nemo({ permissions: { allow: ['Read'] } }) });
ws('read-user-config-true-merges-claude-settings-first', { ...rc, ...cl({ permissions: { allow: ['ClaudeOnly'], defaultMode: 'acceptEdits' }, hooks: { Stop: [hook(undefined, 'echo c')] } }), ...nemo({ permissions: { allow: ['Read'], defaultMode: 'plan' } }), ...proj({ hooks: { Stop: [hook(undefined, 'echo p')] } }) });
ws('read-user-config-claude-project-mode-not-forbidden', { ...rc, ...cl({ permissions: { defaultMode: 'bypassPermissions' } }) });
ws('read-user-config-truthy-string-is-not-true', { [`${H}/.nemo/nemo.json`]: { claude: { readUserConfig: 'true' } }, ...cl({ permissions: { allow: ['ClaudeOnly'] } }) });
ws('read-user-config-malformed-nemo-json-warns', { [`${H}/.nemo/nemo.json`]: '{oops', ...cl({ permissions: { allow: ['ClaudeOnly'] } }) });
ws('read-user-config-claude-dir-from-env', { ...rc, '<TMP>/outside/claude/settings.json': { permissions: { allow: ['FromEnvDir'] } }, ...cl({ permissions: { allow: ['HomeClaude'] } }) }, { env: { CLAUDE_CONFIG_DIR: '<TMP>/outside/claude' } });
ws('nemo-home-from-env-overrides-home-dot-nemo', { '<NEMO_HOME>/settings.json': { permissions: { allow: ['FromNemoHome'] } }, ...nemo({ permissions: { allow: ['DotNemo'] } }) }, { env: { NEMO_HOME: '<NEMO_HOME>' } });
ws('project-root-found-from-subdir-via-dot-nemo', { [`${WS}/.nemo/settings.json`]: { permissions: { allow: ['Root'] } }, [`${WS}/pkg/sub/keep`]: '' }, { cwd: `${WS}/pkg/sub` });
ws('project-root-found-via-dot-git', { [`${WS}/.git/KEEP`]: '', [`${WS}/pkg/AGENTS.md`]: 'pkg memory', [`${WS}/AGENTS.md`]: 'root memory' }, { cwd: `${WS}/pkg` });
ws('memory-user-then-project-ancestors-claude-md-ignored', { ...{ [`${H}/.nemo/AGENTS.md`]: 'user nemo memory', [`${H}/.claude/CLAUDE.md`]: 'user claude memory' }, [`${WS}/.git/KEEP`]: '', [`${WS}/AGENTS.md`]: 'root memory', [`${WS}/CLAUDE.md`]: 'root claude memory', [`${WS}/pkg/AGENTS.md`]: 'sub memory' }, { cwd: `${WS}/pkg` });
ws('memory-imports-relative-cycle-fence-missing', { [`${WS}/AGENTS.md`]: 'top\n@./a.md\n```\n@./skipped.md\n```\n@./missing.md', [`${WS}/a.md`]: 'A text\n@./AGENTS.md', [`${WS}/skipped.md`]: 'SKIPPED' });
add('memory-auto-dir-memory-md-loaded-nemo-home', { op: 'workspace', files: {}, auto_memory: { base: '<HOME>/.nemo', content: 'auto memory from nemo' }, env: {}, cwd: WS }, { note: 'autoMemoryDir/path contain sanitizePath(cwd) which is replaced by <SANITIZED_CWD>' });
add('memory-auto-dir-claude-home-when-read-user-config', { op: 'workspace', files: { ...rc }, auto_memory: { base: '<HOME>/.claude', content: 'auto memory from claude' }, env: {}, cwd: WS }, { note: 'autoMemoryDir/path contain sanitizePath(cwd) which is replaced by <SANITIZED_CWD>' });
ws('agents-frontmatter-project-overrides-user', { [`${H}/.nemo/agents/tester.md`]: '---\ndescription: user tester\ntools: Bash, Read\nmodel: inherit\n---\nUser prompt body', [`${WS}/.nemo/agents/tester.md`]: '---\ndescription: project tester\ntools: [Read, Write]\nmodel: opus\n---\nProject prompt body', [`${WS}/.nemo/agents/other.md`]: 'no frontmatter body' });
ws('agents-claude-dir-only-with-read-user-config', { ...rc, [`${H}/.claude/agents/cl.md`]: '---\ndescription: from claude\n---\nbody' });
ws('skills-project-skill-md-and-claude-skills-gated', { [`${WS}/.nemo/skills/scout/SKILL.md`]: '---\ndescription: finds things\nallowed-tools: Read, Grep\n---\nSkill body here', [`${H}/.claude/skills/cl/SKILL.md`]: '---\ndescription: claude skill\n---\nb' });
ws('commands-subdir-joined-with-colon', { [`${WS}/.nemo/commands/review.md`]: '---\ndescription: Reviews code\nargument-hint: [file]\n---\nReview $1 carefully. $ARGUMENTS', [`${WS}/.nemo/commands/git/commit.md`]: 'Commit everything.\nNo placeholder here.' });
const MJ = { mcpServers: { alpha: { command: 'node', args: ['a.js'], env: { SECRET: 'x' } }, remote: { type: 'http', url: 'https://user:pw@h.example.test/mcp?token=abc', headers: { Authorization: 'Bearer z' } } } };
ws('mcp-project-mcp-json-ignored-without-read-user-config', { [`${WS}/.mcp.json`]: MJ });
ws('mcp-project-mcp-json-needs-approval', { ...rc, [`${WS}/.mcp.json`]: MJ });
ws('mcp-project-mcp-json-approved-by-settings-and-url-stripped', { ...rc, [`${WS}/.mcp.json`]: MJ, ...nemo({ enabledMcpjsonServers: ['alpha'], disabledMcpjsonServers: ['remote'] }) });
ws('mcp-project-mcp-json-enable-all', { ...rc, [`${WS}/.mcp.json`]: MJ, ...proj({ enableAllProjectMcpServers: true }) });
ws('mcp-claude-json-user-local-scopes-and-disabled-names', { ...rc, [`${H}/.claude.json`]: { mcpServers: { u1: { command: 'u' } }, projects: { '<TMP>/ws': { mcpServers: { l1: { command: 'l', args: [1, 2] } }, disabledMcpServers: ['u1'], enabledMcpjsonServers: ['alpha'] } } }, [`${WS}/.mcp.json`]: MJ });
ws('mcp-nemo-json-user-and-projects-read-always', { [`${H}/.nemo/nemo.json`]: { mcpServers: { nu: { url: 'https://n.example.test/x#frag' } }, projects: { '<TMP>/ws': { mcpServers: { nl: { command: 'x' } }, disabledMcpServers: ['nu'] } } } });
ws('mcp-nemo-project-config-opencode-and-claude-shapes', { [`${WS}/.nemo/config.json`]: { mcp: { oc: { type: 'local', command: ['bin', 'a', 'b'] }, off: { type: 'local', command: ['x'], enabled: false }, rem: { type: 'remote', url: 'https://r.example.test/m?k=1' }, flat: { command: 'f' } }, mcpServers: { extra: { type: 'sse', url: 'https://s.example.test/sse' } } } });
ws('mcp-later-scope-overrides-same-name', { ...rc, [`${H}/.claude.json`]: { mcpServers: { dup: { command: 'user' } } }, [`${H}/.nemo/nemo.json`]: { claude: { readUserConfig: true }, mcpServers: { dup: { command: 'nemo-user' } } }, [`${WS}/.nemo/config.json`]: { mcp: { dup: { command: 'nemo-project' } } } });

// ---- user-only loaders and pure helpers ----
const nh = (o) => ({ '<NEMO_HOME>/settings.json': o });
add('nemo-env-block-secrets-refused-both-scopes', { op: 'nemoEnv', files: { ...nh({ env: { FOO: 'bar', N: 5, ANTHROPIC_API_KEY: 'x', MY_TOKEN: 't', DB_PASSWORD: 'p', OPENAI_BASE_URL: 'u' } }), [`${WS}/.nemo/settings.json`]: { env: { PROJ: '1', FOO: 'override', AWS_SECRET_ACCESS_KEY: 's' } } }, scope: null });
add('nemo-env-block-scope-user', { op: 'nemoEnv', files: { ...nh({ env: { A: '1' } }), [`${WS}/.nemo/settings.json`]: { env: { B: '2' } } }, scope: 'user' });
add('nemo-env-block-scope-project', { op: 'nemoEnv', files: { ...nh({ env: { A: '1' } }), [`${WS}/.nemo/settings.json`]: { env: { B: '2' } } }, scope: 'project' });
add('agent-env-project-wins-unless-real-env', { op: 'agentEnv', files: { ...nh({ env: { A: 'user', B: 'user' } }), [`${WS}/.nemo/settings.json`]: { env: { A: 'proj', B: 'proj', C: 'proj' } } }, env: { A: 'user', B: 'real', KEEP: 'k' } });
add('auto-mode-user-only-filters-types', { op: 'autoMode', files: { ...nh({ autoMode: { environment: ['e', 1], allow: ['a'], soft_deny: 'x', hard_deny: ['h'], classifyAllShell: true, useAutoModeDuringPlan: 'no', extra: 1 } }), [`${WS}/.nemo/settings.json`]: { autoMode: { allow: ['PROJECT'] } } } });
add('auto-mode-absent', { op: 'autoMode', files: {} });
add('block-outside-reads-user-level-only', { op: 'blockOutside', files: { ...nh({ permissions: { blockReadsOutsideWorkingDirectories: true } }) } });
add('block-outside-reads-project-cannot-set', { op: 'blockOutside', files: { [`${WS}/.nemo/settings.json`]: { permissions: { blockReadsOutsideWorkingDirectories: true } }, ...nh({ permissions: { blockReadsOutsideWorkingDirectories: 'true' } }) } });
add('secret-env-key-table', { op: 'secretKeys', keys: ['PATH', 'HOME', 'ANTHROPIC_API_KEY', 'OPENAI_API_KEY', 'NEMO_API_KEY', 'NEMO_AUTH_TOKEN', 'GITHUB_TOKEN', 'MY_SECRET', 'DB_PASSWORD', 'AWS_SECRET_ACCESS_KEY', 'api_key', 'Token', 'FOO', 'EDITOR', 'NODE_ENV', 'KEYBOARD', 'MONKEY'] });
add('sanitize-path-table', { op: 'sanitize', paths: ['/Users/a/b', 'C:\\Users\\a b\\proj', '/tmp/x.y_z', '', '日本語/x', '/a//b/'] });
add('parse-frontmatter-table', { op: 'frontmatter', texts: ['---\nname: tester\ndescription: "quoted: yes"\ntools: Bash, Read\nlist: [a, b, c]\nitems:\n  - one\n  - two\nflag: true\nn: 5\nempty:\n---\nbody text here', '---\nliteral: |\n  line one\n  line two\nfolded: >\n  wrapped\n  text\n---\nbody', 'just a plain file\nwith no frontmatter', '---\nunterminated: yes\nbody', '---\r\nname: crlf\r\n---\r\nbody\r\n', ''] });
add('expand-command-table', { op: 'expandCommand', cases: [{ body: 'Review $1 carefully. $ARGUMENTS', args: 'foo.js bar' }, { body: 'Commit.\nNo placeholder.', args: 'extra text' }, { body: 'Commit.', args: '' }, { body: '$1 $2 $3', args: 'a' }, { body: '$ARGUMENTS and $ARGUMENTS', args: 'x y' }, { body: 'Plain', args: null }] });
add('import-path-resolution', { op: 'importPath', cases: [['~/x.md', '<TMP>/ws', '<HOME>'], ['./a.md', '<TMP>/ws/sub', '<HOME>'], ['../b.md', '<TMP>/ws/sub', '<HOME>'], ['/abs/c.md', '<TMP>/ws', '<HOME>'], ['d/e.md', '<TMP>/ws', '<HOME>']] });

exports.source = source;
exports.cases = () => out;

const fs = require('fs');
const byName = (a, b) => (a.name < b.name ? -1 : 1); // readdir order is platform dependent: compare agents/skills/commands as sets
const slim = (w) => ({ ...w, agents: [...w.agents].sort(byName), commands: [...w.commands].sort(byName), skills: w.skills.map((s) => (s.source === 'built-in' ? { name: s.name, source: s.source } : s)).sort(byName) });
exports.run = async (i, sb) => {
  sb.setup({ files: i.files || {}, dirs: i.dirs || [] });
  switch (i.op) {
    case 'workspace': {
      const san = cc.sanitizePath(path.resolve(i.cwd));
      if (i.auto_memory) {
        const d = path.join(i.auto_memory.base, 'projects', san, 'memory');
        fs.mkdirSync(d, { recursive: true });
        fs.writeFileSync(path.join(d, 'MEMORY.md'), i.auto_memory.content);
      }
      const w = slim(cc.loadWorkspace({ cwd: i.cwd, home: sb.home, env: i.env }));
      return JSON.parse(JSON.stringify(w).split(san).join('<SANITIZED_CWD>'));
    }
    case 'nemoEnv': return cc.loadNemoEnv({ cwd: sb.ws, home: sb.nemoHome, env: {}, scope: i.scope || undefined });
    case 'agentEnv': return cc.agentEnv({ cwd: sb.ws, home: sb.nemoHome, env: i.env });
    case 'autoMode': return cc.loadAutoModeConfig({ nemoHome: sb.nemoHome });
    case 'blockOutside': return { value: cc.loadBlockOutsideReads({ nemoHome: sb.nemoHome }) };
    case 'secretKeys': return i.keys.map((k) => ({ key: k, secret: cc.isSecretEnvKey(k) }));
    case 'sanitize': return i.paths.map((p) => ({ path: p, out: cc.sanitizePath(p) }));
    case 'frontmatter': return i.texts.map((t) => cc.parseFrontmatter(t));
    case 'expandCommand': return i.cases.map((c) => ({ ...c, out: cc.expandCommand({ body: c.body }, c.args) }));
    case 'importPath': return i.cases.map(([t, b, h]) => ({ token: t, base: b, out: cc.resolveImportPath(t, b, h) }));
    default: throw new Error(i.op);
  }
};
