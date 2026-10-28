# CLI notes: shell prompt, approvals, permission flags

## `!` shell prompt (interactive app)
- Type `!` at the start of the prompt: the prompt shows `! bash mode`. Enter runs the command; Esc (or backspace on the empty prompt) leaves the mode; while a command runs, Esc cancels it and kills its process tree. Only a typed `!` starts a command: pasted text or a burst of characters starting with `!` is plain text.
- `!cmd` runs as a user-initiated `Bash` call: same terminal runtime, sandbox, `--network` mode and artifact log as the model's Bash tool; events carry `origin:"user"`. A plain `ask` is approved (you typed it); deny rules, protected paths and critical removals still block, with the reason shown. `PreToolUse` hooks run for it and only a deny is honoured (ask and allow are moot: you typed it).
- The command and its capped output are appended to the conversation as `<user-bash-input>` / `<user-bash-output exit=N truncated=B>` (data from a command the user ran). Then the model gets a turn (an implicit harness note, not shown as your text) to report success or failure, key lines and next steps. `!!cmd` records without a model turn.
- Settings (user `settings.json`): `nemo.shell.user_commands_model_turn` (default true), `nemo.shell.user_commands_unsandboxed` (default false: network is denied by default; use `--network loopback|allow`).

## Commands the agent runs (tests, builds)
The agent runs tests itself. A call that needs permission opens the approval dialog (allow-always is offered). Verification-gate checks (`cargo test`, `npm test`, ...) go through the same permission check as a `Bash` call. Without an approver (`nemo run`) the call is not run, the model is told which permission is missing and the task ends `blocked` (`approval_unavailable`); `task.ended.needed_permissions` lists the rules and `nemo run` prints, e.g.:

    nemo: blocked: approval needed for Bash(cargo test:*) — rerun with --allow 'Bash(cargo test:*)' or in the TUI

## Permission modes (`--permission-mode`, Shift+Tab, `/mode`)
`nemo run --permission-mode default|acceptEdits|plan|dontAsk|auto|bypassPermissions`. `nemo run` never prompts (no stdin): asks are decided by the mode and the `--allow` / settings allowlist; an ask nobody can grant is denied.

| mode | `nemo run` behaviour |
|---|---|
| `default` | asks are auto-denied unless allowed by `--allow`/settings |
| `acceptEdits` | edits inside the workspace are allowed; other asks (Bash, MCP, ...) are auto-denied |
| `plan` | read-only; writes are denied |
| `dontAsk` | an ask becomes a deny |
| `auto` | a judge model call decides each open call (allow / deny / ask); ask, unclear answer, error or timeout = denied |
| `bypassPermissions` | everything is allowed except hard stops (deny rules, critical `rm`, destructive MCP asks); loud notice at start. Only from the CLI flag: never from project settings, `.mcp.json` or the environment |

A denied write or command (Write, Edit, Bash, MCP write tools, ...) makes the task end `blocked` / `approval_unavailable` with `needed_permissions`, never `completed`. Typical CI use:

    nemo run --permission-mode acceptEdits --allow 'Bash(cargo test:*)' --prompt "fix the failing test"

**Auto mode judge.** The judge uses the session provider and model; set another model of the same provider with `autoMode.model` in the user `settings.json` (with `autoMode.environment|allow|soft_deny|hard_deny` to steer it). It sees the task, earlier user messages, the workspace and the pending call, all as data (tool output and repo text are never sent). At most 50 judge calls per task (identical calls are cached); their cost counts in the task. Every decision is logged as a `permission.auto_verdict` event and shown in the TUI. Hard stops (deny rules, critical `rm`, protected paths, destructive MCP asks) are decided before it and it cannot relax them.

**Interactive app.** Shift+Tab cycles `default -> acceptEdits -> plan -> auto -> default`; the footer shows the current mode. `/mode` opens a picker with all modes; `/mode bypass` (or the last picker entry) opens the "Bypass permissions?" dialog: it lists what is turned off and what still stops (deny rules, critical removals, destructive MCP asks); only "Yes" switches (Esc or "No" keeps the mode). One Shift+Tab leaves bypass. Bypass is never saved, never restored by `--resume`, and `--permission-mode bypassPermissions` is refused by the TUI. A switch applies to tasks started after it (a running task keeps its mode). Every change emits `session.mode_changed`; hosts call `Harness::set_mode(mode, confirmed_bypass)`, which refuses an unconfirmed bypass in the core.

## `--allow RULE` / `--deny RULE` (repeatable, `nemo run` and the interactive app)
Settings rule syntax (`Bash(cargo test:*)`, `Edit`, `mcp__server__tool`). Flags are typed by you, so trusted. Deny wins over allow. `--allow` cannot lift critical-path or protected-path protections. A malformed rule is a usage error (exit 2).
