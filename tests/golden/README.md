# Golden fixtures (regression snapshots of the Node prototype)

Spec: `docs/specs/eval-and-golden.md` section B. The Node CLI (`~/Vibe/nemo-cli`, read-only) is the capture source, not an authority: these files are
regression snapshots of its behaviour, approved by the owner on 2026-10-29. A divergence is a decision to record (allow-list + reason), not automatically a Rust bug. Layout: `tests/golden/<area>/<case_id>/{input.json,expected.json,meta.json}`.

## Regenerate

```
node tools/golden-capture/capture.js [--out DIR] [--areas config,permissions,...] [--real-date]
NEMO_NODE_REF=/path/to/nemo-cli node tools/golden-capture/capture.js     # default ~/Vibe/nemo-cli
```

Plain Node >= 22, no npm deps. The tool only reads the reference; it never writes there. It sets `HOME`/`NEMO_HOME`/`CLAUDE_CONFIG_DIR`
to fresh temp dirs, `TERM=dumb`, `NO_COLOR=1`, `TZ=UTC`, removes `COLORTERM`/`NEMO_*`/`ANTHROPIC_*`/`OPENAI_*`/`CLAUDE_*`,
runs every case in its own sandbox, and exits non-zero if any expected output still contains a temp or real home path.
Determinism check: run twice (even with different `TMPDIR`) into two dirs and `diff -r` them. `captured_at` is the literal `<TS>`
unless `--real-date` is passed (then exclude `meta.json` from comparisons). Generated cases are rewritten per area on each run.

## Areas

| area | Node entry points | input.json `op` / content |
|---|---|---|
| config | `loadConfig`, `validateVersionedConfig`, `resolveModelDefaults`, `bestModelForFamily`, `parseModel`, `listModels`, `pickDefaultModel`, `detectProtocol` | `loadConfig`: `files`, `env`, `cwd`, `home`; others: literal args |
| permissions | `Permissions.check/suggestions/apply`, `isReadOnlyBash`, `isAutoSafeBash`, `isBroadAllow` | `check`: rules, modes, calls, fs layout (incl. symlinks); `predicates`, `broad`, `suggestions`, `apply` |
| transcripts | `history.itemsFromEntries`, `parser.applyEntry/sessionPhase/...`, file loaders | `replay`: JSONL `entries` + explicit `now`; helper and file ops |
| schemas | `toolSchemas(builtinTools())`, `agentToolSchemas` | `names`, `builtin`, `agent` |
| tools | `Read`, `Write`, `Edit`, `Glob`, `Grep` `.run` | `files`, `dirs`, ordered `steps` sharing one `readFiles` ctx; expected = per-step result + `tree_after` |
| keys | `keys.parseKeys` + `createKeyState` | `chunks`: successive stdin reads through one state; expected = events + pending buffer per chunk |
| sgr | `sgr.encode/to256/detectColorMode/applySgr` | per-op literal args |
| selection | `selectionTransition` and the pure geometry/text helpers, `copyToClipboard` (fake spawn) | per-op literal args |
| providers | `createAnthropicProvider`/`createOpenAIProvider` with fake `clientImpl`, `modelnorm`, `clientOptions`, `anthropicUsesBearer` | `stream`: normalized `req` + recorded `chunks` -> `wire_body` + `events`; others literal |
| render (95) | `render.renderFrame/tasksPanel/usagePanel/tasksUsageRow/heatLevel/layoutFor/fmtNum/fmtDur/bar/sessionsWindow`, `agentview.renderAgent/todoWindow`, `theme.*`, `highlight`, `picker`, `dialogs` | `frame`, `agent`, `tasks`, `usage`, `tasksUsage`, helper tables; see "Render" below |
| settings (56) | `claudecfg.loadWorkspace` (settings merge, `readUserConfig`, hooks sections, memory/agents/skills/commands/mcp), `loadNemoEnv`, `agentEnv`, `loadAutoModeConfig`, `loadBlockOutsideReads`, `isSecretEnvKey`, `sanitizePath`, `parseFrontmatter`, `expandCommand`, `resolveImportPath` | `workspace`: `files` (sandbox layout), `env`, `cwd`, optional `auto_memory`; others literal |
| hooks (36) | `Hooks.run` (matcher full-match semantics, dedupe, outcome merge, stdin contract, env stripping) | `config` (hooks section), `event`, `payload`, `target` (`matcherTarget`, `null` = none), `opts` (`sessionId`, `transcriptPath`, `cwd` default `<TMP>/ws`, `env`, `disabled`) |

## Render (areas `render`)

Frames are deterministic: `TZ=UTC`, a fixed `now` (2026-10-02T12:00:00Z) and `frame` counter in each input, theme defaults, no terminal. Each frame case writes
`expected.json` (`lines` = stripAnsi rows, `cols`/`rows` or `w`/`h`, hit rects for `frame` cases: `chatRect`, `sidebarRect`, `sessionRows`, ...),
`expected.txt` (the same rows, LF-joined; this is the plain-text golden) and `expected.ansi.json` (`color_mode: "truecolor"`, `lines` = the raw
SGR strings the renderer emits (always 24-bit; the renderers never consult the colour mode), `encoded_truecolor` = `sgr.encode(line, 'truecolor')`,
what `Screen` writes). 256-colour output is already covered by the `sgr` area. `renderFrame` clamps to 60x16, so the "40x20" cases are 60x20 frames.

Input state is JSON hydrated by the capture (a Rust test must build the equivalent state):
- `session`: `{id, cwd, model, gitBranch, startedAt, entries, set}`: `parser.createSession(id)`, then `parser.applyEntry` for each transcript entry, then `set` shallow-assigned (e.g. `todos`, `contextUsed`).
- `sessions` (explicit list) or `sessions_n` (`n` generated rows: `session-<i>-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa`, project `proj-<i>`, cwd `D:/workspace/project-<i>`, `mtime = now - i*1000`, `live {name, cwd}`, `phase idle`, `tokens 1000*(i+1)`, `ctxPct 10`).
- `history_gen {end, days, base}`: `days` daily entries ending at `end` (UTC); the entry `n` days ago has `tokens = n%7==0 ? 0 : base(1000)+n`, `output n%5`, `messages 3`, `tools 2`, `agents n%3==0 ? 1 : 0`, `sessions 1`.
- `agent`: duck-typed agent (`items`, `pending`, `mode`, `status`, `turnStart`, `turnTokens`, `model`, `subagents`, ...); a `pending` permission without `options` gets `dialogs.permissionOptions`/`PLAN_OPTIONS`, a `question` gets `questionState`. `editor.text` is inserted into a fresh `LineEditor`. `picker` = `{id,title,subtitle,items,set}` through `picker.createPicker`.

## Settings / hooks notes

`settings` expected output is the loadWorkspace result with built-in skills reduced to `{name, source}`, agents/commands/skills sorted by name (Node's readdir order is
platform dependent), and the sanitized cwd (`sanitizePath(cwd)`, tmpdir-dependent) replaced by `<SANITIZED_CWD>`. `hooks` runs real `sh -c` one-liners (`printf`, `cat`, `exit N`)
through the Bash tool's shell; `results[].ms` is dropped. Captured on darwin; must be re-captured/validated on Linux and Windows (Git Bash).

## Placeholders and normalisation

Inputs may contain `<TMP>` (sandbox root; layout `<TMP>/home`, `<TMP>/nemo_home`, `<TMP>/ws` = cwd, `<TMP>/outside`), `<HOME>`, `<NEMO_HOME>`.
A Rust test must expand them to its own temp dir (realpath it) before running and apply the same replacement, longest root first, to its
output before comparing. In `expected.json` UUIDs become `<UUID>` and ISO-8601 `...Z` timestamps `<TS>`. Objects have sorted keys, `Map` is an
object, `Set` a sorted array, `undefined` fields are absent, 2-space JSON, LF.

Error results are `{ "error": { name, code, message } }`, successful config/parse calls `{ "value": ... }`.

## How Rust tests load them

Walk `tests/golden/<area>/*/`, read `input.json`, dispatch on `op` (or the area default), build the Rust result, serialize with the same
normalisation, compare to `expected.json` as JSON values (not text). Every case is owner-reviewed (`reviewed_by` = `owner:DevOnChain101 2026-10-29 (regression snapshot, Node prototype not authoritative)`); `capture.js` preserves an existing `reviewed_by` per case on re-capture (a new case starts as `null` until reviewed). Fields in `meta.json`:
`source`, `ref_version`, `ref_tree_sha256` (over sorted relative paths + contents of `src/` and `bin/`), `node`, `platform`, `captured_at`, `reviewed_by`, optional `note`.
Cases with a `note` flag a known Node quirk or platform dependency.

Order-dependent leaks handled in the capture: Grep with several files returns lines in directory-walk order, so those steps are captured with
lines sorted (`sort_lines`); compare as a set. Glob order depends on mtimes, so the capture pins file mtimes to 1700000000+index seconds
(Rust must do the same: files in `input.json` order, oldest first).

## Not captured (no deterministic path)

- Live Bash tool / background jobs / BashOutput / KillShell (processes, timing, platform shells, process-tree kill).
- WebFetch / WebSearch (network), PDF text extraction, NotebookEdit (not yet), Skill/Task* tools' state (need agent runtime).
- TUI loop (`app.js` key handling, mouse, Screen diffing/resize repaint), live animation over time (only single frames at pinned `frame` values are captured), watcher-fed live session lists.
- Render: the topology/flow canvas is only captured inside whole `renderFrame` cases (no separate `flow.js` unit), the workspace box/`state.workspaces`, sub-agent docked beside the topology (`chatAgent` != main), the activity-panel click maps (`activityHits`), selection highlight overlay, image thumbnails (`thumb`), the 256-colour render of whole frames, and `pickers.js` (`open*Picker` build items from live agent/config/network state).
- Subagents, the transcript watcher (`SessionWatcher`, timers, fs events; `loadTranscriptSession` is captured with `agents:false`).
- Hooks: timeouts/kill-tree, spawn failures, `Hooks` wired into the agent loop (`native-hook-events`), `import-claude-hooks` conversion. Settings: `nemo import claude` writers, `agents`/`skills` edge parsing beyond the cases listed, `~/.codex`.
- Real SDK transport, SSE parsing, retries, auth/credential resolution (`auth.js`, keychain/plaintext `auth.json`).
- Windows/Linux outputs: everything here was captured on one platform (see `meta.json.platform`). Path-sensitive areas (config paths,
  permissions case-fold/`\`/UNC, Bash tool text in schemas) must be re-captured on win32/linux before being treated as the 3-OS spec.
