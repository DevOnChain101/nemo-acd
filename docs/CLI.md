# CLI notes: shell prompt, approvals, permission flags

## `!` shell prompt (interactive app)
- Type `!` at the start of the prompt: the prompt shows `! bash mode`. Enter runs the command; Esc (or backspace on the empty prompt) leaves the mode; while a command runs, Esc cancels it and kills its process tree. Only a typed `!` starts a command: pasted text or a burst of characters starting with `!` is plain text.
- `!cmd` runs as a user-initiated `Bash` call: same terminal runtime, sandbox, `--network` mode and artifact log as the model's Bash tool; events carry `origin:"user"`. A plain `ask` is approved (you typed it); deny rules, protected paths and critical removals still block, with the reason shown. `PreToolUse` hooks run for it and only a deny is honoured (ask and allow are moot: you typed it).
- The command and its capped output are appended to the conversation as `<user-bash-input>` / `<user-bash-output exit=N truncated=B>` (data from a command the user ran). Then the model gets a turn (an implicit harness note, not shown as your text) to report success or failure, key lines and next steps. `!!cmd` records without a model turn.
- Settings (user `settings.json`): `nemo.shell.user_commands_model_turn` (default true), `nemo.shell.user_commands_unsandboxed` (default false: network is denied by default; use `--network loopback|allow`).

## Commands the agent runs (tests, builds)
The agent runs tests itself. A call that needs permission opens the approval dialog (allow-always is offered). Verification-gate checks (`cargo test`, `npm test`, ...) go through the same permission check as a `Bash` call. Without an approver (`nemo run`) the call is not run, the model is told which permission is missing and the task ends `blocked` (`approval_unavailable`); `task.ended.needed_permissions` lists the rules and `nemo run` prints, e.g.:

    nemo: blocked: approval needed for Bash(cargo test:*) — rerun with --allow 'Bash(cargo test:*)' or in the TUI

## Permission modes (`--permission-mode`, Shift+Tab)
`nemo run --permission-mode default|acceptEdits|plan|dontAsk|auto|bypassPermissions`. `nemo run` never prompts (no stdin): asks are decided by the mode and the `--allow` / settings allowlist; an ask nobody can grant is denied.

| mode | `nemo run` behaviour |
|---|---|
| `default` | asks are auto-denied unless allowed by `--allow`/settings |
| `acceptEdits` | edits inside the workspace are allowed; other asks (Bash, MCP, ...) are auto-denied |
| `plan` | read-only; writes are denied |
| `dontAsk` | an ask becomes a deny |
| `auto` | a judge model call decides each open call (allow / deny / ask); ask, unclear answer, error or timeout = denied |
| `bypassPermissions` | skips approval prompts; still stops: deny rules, ask rules (they still ask, so an unattended run denies them), critical `rm`, destructive MCP asks. Protected paths (`.git`, `.nemo`, `.ssh`, ...) are NOT stopped by plain writes, as in Claude Code; the sandbox still confines commands. Loud notice at start. Only from the CLI flag: never from project settings, `.mcp.json` or the environment |

A denied write or command (Write, Edit, Bash, MCP write tools, ...) makes the task end `blocked` / `approval_unavailable` with `needed_permissions`, never `completed`. Typical CI use:

    nemo run --permission-mode acceptEdits --allow 'Bash(cargo test:*)' --prompt "fix the failing test"

**Auto mode judge.** The judge uses the session provider and model; set another model of the same provider with `autoMode.model` in the user `settings.json` (with `autoMode.environment|allow|soft_deny|hard_deny` to steer it). It sees the task, earlier user messages, the workspace and the pending call, all as data (tool output and repo text are never sent). At most 50 judge calls per task (identical calls are cached); their cost counts in the task. Every decision is logged as a `permission.auto_verdict` event and shown in the TUI. Hard stops (deny rules, critical `rm`, protected paths, destructive MCP asks) are decided before it and it cannot relax them.

**Interactive app.** Shift+Tab is the only way to switch modes (there is no `/mode` command): it cycles `default -> acceptEdits -> plan -> auto -> bypassPermissions -> default`; the footer shows the current mode. Reaching `bypassPermissions` opens the "Bypass permissions?" dialog: it lists what is turned off and what still stops (deny rules, ask rules, critical removals, destructive MCP asks); "No" is the default, and Esc or "No" stays in the previous mode; only "Yes" switches. The next Shift+Tab leaves bypass. `dontAsk` is reachable only with `--permission-mode`. Bypass is never saved, never restored by `--resume`, and `--permission-mode bypassPermissions` is refused by the TUI. A switch applies to tasks started after it (a running task keeps its mode). Every change emits `session.mode_changed`; hosts call `Harness::set_mode(mode, confirmed_bypass)`, which refuses an unconfirmed bypass in the core.

## Selecting and copying text (interactive app)
Mouse reporting is on, so the app does the selection itself.
- Drag over the chat to select (the highlight follows the pointer; a drag may start on a blank cell and run in either direction). On release the text is copied to the system clipboard and a short "Copied N chars" notice shows. A plain click starts afresh; Shift/Alt+click extends a kept selection.
- Double-click selects a word, triple-click the whole line (a wrapped paragraph counts as one line); both copy at once.
- Ctrl+C copies a highlighted selection and clears it; with no selection it keeps its meaning (clear the prompt, cancel the task, press again to quit). Terminals keep Cmd+C for themselves and never pass it to the app, so use Ctrl+C or just release the mouse.
- Copied text leaves out gutters and decorations (the `│` of thinking, diff line numbers, bullets, indents) and joins wrapped rows into one line. Wide characters (CJK) are copied whole.
- The clipboard is written with `pbcopy` (macOS), `clip.exe` (Windows), `wl-copy` / `xclip` (Linux, when `WAYLAND_DISPLAY` / `DISPLAY` is set), then OSC 52 (also works over ssh when the terminal allows it). If none works the notice says "copy failed".
- To use the terminal's own selection instead, hold the bypass key while dragging: Shift in most terminals (xterm, GNOME Terminal, kitty, Windows Terminal), Option on macOS (iTerm2: Option; some terminals need Fn or Option+Cmd). Text selected that way is copied by the terminal (Cmd+C / Ctrl+Shift+C) and may include the gutters.
- Terminals without SGR mouse reporting (mode 1006) are not supported for in-app selection; use the native selection above.

## `--allow RULE` / `--deny RULE` (repeatable, `nemo run` and the interactive app)
Settings rule syntax (`Bash(cargo test:*)`, `Edit`, `mcp__server__tool`). Flags are typed by you, so trusted.
- `deny: ["mcp__*"]` removes every MCP tool, `mcp__server__*` (or `mcp__server`) one server; `mcp__*` is never honoured as an allow rule.
- A `Read(path)` deny/ask rule also covers Grep, Glob, Write/Edit of that path (denied, not asked), and Bash readers of it (`cat`, `head`, `tail`, `less`, `grep`, `sed`, `awk`, `cp`, `< file` ...). It is checked on the command's own operands only; a command that builds the path in the shell (`cat "$F"`) cannot be matched, which is what the sandbox and the other rules are for.
- An ask rule prompts in every mode but `dontAsk` (denied there), `bypassPermissions` included; `nemo run` has nobody to ask, so it is denied.
- A `settings.json` that exists but is malformed (a trailing comma, a missing brace) or unreadable stops `nemo run` with exit 2 and `settings error: ...`, and the interactive app does not start, because the deny rules in it could not apply. Fix or remove the file.
- Memory files: per directory, the first non-empty of `NEMO.md`, `AGENTS.md`, `CLAUDE.md` (then `.claude/CLAUDE.md`) is loaded, root to cwd, 32 KiB combined; `CLAUDE.local.md` is added next to it.
- Hooks: `PreToolUse` and `UserPromptSubmit` fail closed and time out after 60 s by default; every other event defaults to 600 s. A per-hook `timeout` (seconds) overrides either.
- Read: a whole-file read over 256 KB, or a result over about 25k tokens, is refused with the offset/limit to use instead.
- Bash timeout: `nemo run` kills the command; the interactive app keeps it running in the background and tells the model its `bash_id` (BashOutput / KillShell).
- Windows: no OS backend can confine commands, so without `--sandbox` the default is `best-effort` with a notice at start (commands run unconfined); pass `--sandbox enforce` to refuse them instead. macOS and Linux default to `enforce`. Deny wins over allow. `--allow` cannot lift critical-path or protected-path protections. A malformed rule is a usage error (exit 2).
