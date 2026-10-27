'use strict';
const { ref } = require('../env');
const path = require('path');
const perm = require(path.join(ref, 'src/native/permissions.js'));

const source = 'src/native/permissions.js';
const MODES = ['default', 'acceptEdits', 'plan', 'auto', 'dontAsk', 'bypassPermissions'];
const WS = '<TMP>/ws';
const OUT = '<TMP>/outside';
const out = [];
const slug = (s) => String(s).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 40);
const add = (id, input, extra = {}) => out.push({ id, input, ...extra });
const FS = { dirs: [`${WS}/sub`] };

// a "check" case: every call is evaluated under every mode in `modes` with one Permissions per (call, mode)
const chk = (id, calls, o = {}) => add(id, { op: 'check', rules: o.rules || {}, modes: o.modes || MODES, cwd: WS, home: '<HOME>', autoMode: o.autoMode || {}, flags: o.flags || {}, fs: o.fs || FS, calls, suggestions: !!o.suggestions }, o.extra);

// ---- Bash table (permission-modes.test.js TABLE) ----
const BASH = [
  'ls -la', 'git status && cat a.txt | head', 'ls; rm x', 'mkdir newdir', 'touch a.txt', 'mv a.txt b.txt', 'cp a.txt sub/b.txt',
  'sed -i s/a/b/ a.txt', 'sed -i s/a/b/e a.txt', 'sed -n 1p a.txt', 'LANG=C mkdir x', 'timeout 5 rm x', 'nice -n 5 touch x', 'cd sub && touch x',
  'rm -rf ./build/*.o', 'rm *.o', 'rm -rf src/*', 'npm test', 'git add .', 'curl http://example.com', 'node script.js', 'touch .git/hooks/x',
  'touch .nemo/settings.json', 'mkdir x && touch .husky/pre-commit', `touch ${OUT}/x`, `mv a.txt ${OUT}/a.txt`, `cp ${OUT}/a.txt .`, 'rm $HOME/x', 'touch "$DIR"/x',
  'rm -rf ~', 'rm -rf /', 'rm -rf /usr', 'rm -rf "$DIR"/*', "bash -c 'rm -rf ~'", 'sh -c "rm -rf ~"', '(rm -rf ~)', '{ rm -rf ~; }',
  'rm -rf *', 'rm -rf */', 'rm -rf .*', 'rm -rf ./*', 'rm -rf ./*/', 'rm -rf [a-z]*', 'rm -rf .', 'rm -rf ..', 'rm -rf $(echo /)', 'LANG=C rm -rf ~', `rm -rf ${WS}`,
];
BASH.forEach((c, i) => chk(`bash-${String(i).padStart(2, '0')}-${slug(c)}`, [{ tool: 'Bash', input: { command: c } }]));

// ---- edit tools / protected paths (permission-modes.test.js) ----
[`${WS}/a.txt`, `${WS}/sub/deep.txt`, `${OUT}/a.txt`, `${WS}/.git/config`, `${WS}/.nemo/settings.json`, `${WS}/sub/.vscode/tasks.json`, `${WS}/.zshrc`, `${WS}/.mcp.json`, 'rel/new.txt', '../escape.txt']
  .forEach((p, i) => chk(`write-${String(i).padStart(2, '0')}-${slug(p.replace('<TMP>', ''))}`, [{ tool: 'Write', input: { file_path: p } }, { tool: 'Edit', input: { file_path: p } }], { suggestions: true }));

// ---- other tools ----
chk('webfetch-and-websearch', [{ tool: 'WebFetch', input: { url: 'https://example.com/x' } }, { tool: 'WebSearch', input: { query: 'q' } }], { suggestions: true });
chk('read-only-tools-and-dialogs', ['Read', 'Glob', 'Grep', 'LS', 'BashOutput', 'TodoWrite', 'TaskCreate', 'Skill', 'Task', 'Agent', 'AskUserQuestion', 'ExitPlanMode'].map((tool) => ({ tool, input: {} })));
chk('unknown-and-mcp-tools', [{ tool: 'mcp__srv__do', input: { a: 1 } }, { tool: 'Bogus', input: {} }], { suggestions: true });
chk('read-outside-workspace', [{ tool: 'Read', input: { file_path: `${OUT}/f.txt` } }, { tool: 'Grep', input: { pattern: 'x', path: OUT } }, { tool: 'Glob', input: { pattern: '*.txt', path: OUT } }, { tool: 'Glob', input: { pattern: `${OUT}/*.txt` } }, { tool: 'Read', input: { file_path: `${WS}/in.txt` } }]);
chk('read-outside-workspace-answered', [{ tool: 'Read', input: { file_path: `${OUT}/f.txt` } }], { flags: { outsideReadsAnswered: true } });
chk('read-outside-workspace-blocked-by-setting', [{ tool: 'Read', input: { file_path: `${OUT}/f.txt` } }, { tool: 'Read', input: { file_path: `${WS}/ok.txt` } }], { flags: { blockOutsideReads: true } });
chk('read-network-paths', [{ tool: 'Read', input: { file_path: '//server/share/x.txt' } }, { tool: 'Read', input: { file_path: '\\\\server\\share\\x.txt' } }]);

// ---- rules ----
const R = (id, rules, calls, o = {}) => chk(id, calls, { rules, ...o });
const B = (command) => ({ tool: 'Bash', input: { command } });
R('rules-deny-wins-everywhere', { deny: ['Bash(rm:*)'], allow: ['Bash'] }, [B('rm -rf x'), B('ls'), B('echo hi; rm y')]);
R('rules-deny-subcommand-in-compound', { deny: ['Bash(git push:*)'] }, [B('ls && git push origin main'), B('git status')]);
R('rules-ask-rule-beats-allow-and-readonly', { ask: ['Bash(ls:*)'], allow: ['Bash(ls:*)'] }, [B('ls -la')]);
R('rules-allow-bash-prefix', { allow: ['Bash(npm test:*)', 'Bash(git commit:*)'] }, [B('npm test -- --watch'), B('npm install'), B('git commit -m x'), B('npm test && rm -rf x')]);
R('rules-allow-bash-exact-and-star', { allow: ['Bash(make build)', 'Bash(echo *)'] }, [B('make build'), B('make build2'), B('echo hello'), B('echo a; make clean')]);
R('rules-allow-tool-wide', { allow: ['Write', 'Edit'] }, [{ tool: 'Write', input: { file_path: `${OUT}/x` } }, { tool: 'Edit', input: { file_path: `${WS}/x` } }]);
R('rules-path-globs-edit', { allow: ['Edit(src/**)', 'Write(/abs-never/**)'], deny: ['Edit(**/*.secret)'] }, [{ tool: 'Edit', input: { file_path: `${WS}/src/a/b.js` } }, { tool: 'Edit', input: { file_path: `${WS}/src/k.secret` } }, { tool: 'Edit', input: { file_path: `${WS}/lib/a.js` } }]);
R('rules-path-glob-home-tilde', { deny: ['Read(~/.ssh/**)'] }, [{ tool: 'Read', input: { file_path: '<HOME>/.ssh/id_rsa' } }, { tool: 'Read', input: { file_path: '<HOME>/.bashrc' } }]);
R('rules-webfetch-domain', { allow: ['WebFetch(domain:example.com)'], deny: ['WebFetch(domain:evil.test)'] }, [{ tool: 'WebFetch', input: { url: 'https://example.com/a' } }, { tool: 'WebFetch', input: { url: 'https://evil.test/a' } }, { tool: 'WebFetch', input: { url: 'https://other.test' } }]);
R('rules-mcp-tool-allow', { allow: ['mcp__srv__do', 'mcp__srv2'] }, [{ tool: 'mcp__srv__do', input: {} }, { tool: 'mcp__srv__other', input: {} }, { tool: 'mcp__srv2__x', input: {} }]);
R('rules-allow-never-approves-critical-or-protected', { allow: ['Bash(rm:*)', 'Bash(touch:*)', 'Edit(.git/**)', 'Write'] }, [B('rm -rf ~'), B(`rm -rf ${WS}`), B('touch .git/x'), B('rm x'), { tool: 'Edit', input: { file_path: `${WS}/.git/config` } }, { tool: 'Write', input: { file_path: `${WS}/.git/config` } }]);
R('rules-deny-beats-bypass', { deny: ['Bash(rm:*)'] }, [B('rm -rf ~'), B('rm -rf x')]);
R('rules-additional-directories', { additionalDirectories: [`${OUT}/extra`] }, [B(`touch ${OUT}/extra/x`), { tool: 'Write', input: { file_path: `${OUT}/extra/x` } }, B(`rm -rf ${OUT}/*`), { tool: 'Read', input: { file_path: `${OUT}/extra/r.txt` } }, { tool: 'Read', input: { file_path: `${OUT}/other.txt` } }], { fs: { dirs: [`${WS}/sub`, `${OUT}/extra`] } });
R('rules-auto-mode-classify-all-shell', { allow: ['Bash(ls:*)'] }, [B('ls'), B('npm test'), B('git status')], { autoMode: { classifyAllShell: true } });
R('rules-plan-without-auto-classifier', {}, [B('npm test'), B('ls'), B('rm x'), { tool: 'Write', input: { file_path: `${WS}/a` } }], { autoMode: { useAutoModeDuringPlan: false } });
R('rules-malformed-rules-ignored', { allow: ['', 'Bash(', '(x)', 'Bash()'], deny: [' '] }, [B('ls'), B('touch a')]);

// ---- symlink escape (junction/symlink out of cwd) ----
chk('symlink-escape-bash-and-edit', [B('touch escape/new.txt'), B('rm escape/victim.txt'), B('mv a.txt escape/a.txt'), B('sed -i s/x/y/ escape/victim.txt'), { tool: 'Write', input: { file_path: `${WS}/escape/new.txt` } }, { tool: 'Read', input: { file_path: `${WS}/escape/victim.txt` } }],
  { fs: { dirs: [`${WS}/sub`], files: { [`${OUT}/victim.txt`]: 'x' }, symlinks: { [`${WS}/escape`]: OUT } }, modes: ['acceptEdits', 'auto', 'default'] });
chk('symlink-inside-stays-inside', [B('touch inlink/new.txt'), { tool: 'Write', input: { file_path: `${WS}/inlink/new.txt` } }], { fs: { dirs: [`${WS}/real`], symlinks: { [`${WS}/inlink`]: `${WS}/real` } }, modes: ['acceptEdits', 'auto'] });

// ---- bash predicates ----
const CMDS = {
  'read-only': ['ls', 'cat a | head', 'cd src && ls', 'git status', 'git branch --list', 'node --version', 'npm ls', 'env', 'echo hi 2>&1', 'git log --oneline -n 5', 'git diff HEAD~1', 'find . -name "*.js"', 'grep -rn foo src | wc -l', 'echo a > /dev/null', 'pwd; whoami', 'cat a.txt &>/dev/null'],
  'build-test': ['npm test', 'npm run lint', 'node --test x', 'node --check x', 'npx tsc --noEmit', 'cargo test', 'cargo check'],
  'unsafe': ['rm x', 'ls > out', 'echo $(id)', 'find . -delete', 'git push', 'env FOO=1 ls', 'ls `id`', 'cat <(ls)', "echo 'unbalanced", 'cat <<EOF\nx\nEOF', 'ls >> log', 'find . -exec rm {} ;', 'git branch -D x', 'npm run deploy', 'curl http://x', 'ls | xargs rm', 'sed -i s/a/b/ f', 'echo hi > /dev/null; rm x', 'ls &', ''],
};
for (const [group, commands] of Object.entries(CMDS)) add(`predicates-${group}`, { op: 'predicates', commands }, { source: 'isReadOnlyBash/isAutoSafeBash/isSafeBashCommand in src/native/permissions.js' });

add('broad-allow-rules', { op: 'broad', rules: ['Task', 'Agent', '*', 'Bash', 'Bash(*)', 'Bash(ls)', 'Bash(ls:*)', 'Bash(python:*)', 'Bash(python3 script.py)', 'Bash(py*)', 'Bash(p*)', 'Bash(npm run:*)', 'Bash(npm test:*)', 'Bash(npm:*)', 'Bash(yarn dlx:*)', 'Bash(bash -c:*)', 'Bash(sudo:*)', 'Bash(git status:*)', 'Bash(:*)', 'Read', 'Write', 'WebFetch(domain:x.test)'] }, { source: 'isBroadAllow in src/native/permissions.js' });
add('broad-allow-rules-classify-all-shell', { op: 'broad', classifyAllShell: true, rules: ['Bash', 'Bash(ls:*)', 'Bash(ls)', 'Read', 'Task'] }, { source: 'isBroadAllow in src/native/permissions.js' });

// ---- suggestions and apply ----
add('suggestions-bash-subcommand-tools', { op: 'suggestions', calls: [B('git commit -m x'), B('npm run build'), B('ls -la'), B('  docker   compose up '), B(''), B('cargo test --all'), B('python3 -m pytest')] });
add('suggestions-other-tools', { op: 'suggestions', calls: [{ tool: 'Write', input: {} }, { tool: 'MultiEdit', input: {} }, { tool: 'WebFetch', input: { url: 'https://docs.example.com/a?b' } }, { tool: 'WebFetch', input: { url: 'nope' } }, { tool: 'Read', input: {} }, { tool: 'mcp__a__b', input: {} }] });
add('apply-suggestions-mutates-rules-and-mode', { op: 'apply', rules: { allow: ['Bash(ls:*)'] }, suggestions: [{ type: 'addRules', rules: [{ toolName: 'Bash', ruleContent: 'ls:*' }, { toolName: 'Bash', ruleContent: 'git status:*' }, { toolName: 'Read', ruleContent: null }], behavior: 'allow', destination: 'session' }, { type: 'addRules', rules: [{ toolName: 'Bash', ruleContent: 'rm:*' }], behavior: 'deny' }, { type: 'addRules', rules: [{ toolName: 'Edit', ruleContent: 'a/**' }], behavior: 'ask' }, { type: 'setMode', mode: 'acceptEdits' }], then: [B('git status'), B('rm x'), { tool: 'Edit', input: { file_path: `${WS}/a/x` } }] });
add('mode-unknown-falls-back-to-current', { op: 'check', rules: {}, modes: ['default', 'bogus'], cwd: WS, home: '<HOME>', autoMode: {}, flags: {}, fs: FS, calls: [B('npm test')], suggestions: false });

exports.source = source;
exports.cases = () => out;
exports.run = async (i, sb) => {
  if (i.op === 'predicates') return i.commands.map((c) => ({ command: c, readOnly: perm.isReadOnlyBash(c), autoSafe: perm.isAutoSafeBash(c), isSafeBashCommandIsAutoSafe: perm.isSafeBashCommand(c) === perm.isAutoSafeBash(c) }));
  if (i.op === 'broad') return i.rules.map((r) => ({ rule: r, broad: perm.isBroadAllow(r, !!i.classifyAllShell) }));
  sb.setup(i.fs || {});
  const make = (mode) => {
    const p = new perm.Permissions(i.rules || {}, { cwd: i.cwd || sb.ws, home: i.home || sb.home, mode, autoMode: i.autoMode || {} });
    Object.assign(p, i.flags || {});
    return p;
  };
  if (i.op === 'suggestions') {
    const p = make('default');
    return i.calls.map((c) => ({ tool: c.tool, input: c.input, suggestions: p.suggestions(c.tool, c.input) }));
  }
  if (i.op === 'apply') {
    const p = make('default');
    const mode = p.apply(i.suggestions);
    return { returned_mode: mode, mode: p.mode, allow: p.allow, deny: p.deny, ask: p.ask, then: i.then.map((c) => ({ tool: c.tool, input: c.input, result: p.check(c.tool, c.input) })) };
  }
  return i.calls.map((c) => {
    const by_mode = {};
    for (const m of i.modes) by_mode[m] = make(m).check(c.tool, c.input);
    return { tool: c.tool, input: c.input, by_mode, ...(i.suggestions ? { suggestions: make('default').suggestions(c.tool, c.input) } : {}) };
  });
};
