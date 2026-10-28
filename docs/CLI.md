# CLI notes: shell prompt, approvals, permission flags

## `!` shell prompt (interactive app)
- Type `!` at the start of the prompt: the prompt shows `! bash mode`. Enter runs the command; Esc (or backspace on the empty prompt) leaves the mode; while a command runs, Esc cancels it and kills its process tree. Only a typed `!` starts a command: pasted text or a burst of characters starting with `!` is plain text.
- `!cmd` runs as a user-initiated `Bash` call: same terminal runtime, sandbox, `--network` mode and artifact log as the model's Bash tool; events carry `origin:"user"`. A plain `ask` is approved (you typed it); deny rules, protected paths and critical removals still block, with the reason shown. `PreToolUse` hooks run for it and only a deny is honoured (ask and allow are moot: you typed it).
- The command and its capped output are appended to the conversation as `<user-bash-input>` / `<user-bash-output exit=N truncated=B>` (data from a command the user ran). Then the model gets a turn (an implicit harness note, not shown as your text) to report success or failure, key lines and next steps. `!!cmd` records without a model turn.
- Settings (user `settings.json`): `nemo.shell.user_commands_model_turn` (default true), `nemo.shell.user_commands_unsandboxed` (default false: network is denied by default; use `--network loopback|allow`).

## Commands the agent runs (tests, builds)
The agent runs tests itself. A call that needs permission opens the approval dialog (allow-always is offered). Verification-gate checks (`cargo test`, `npm test`, ...) go through the same permission check as a `Bash` call. Without an approver (`nemo run`) the call is not run, the model is told which permission is missing and the task ends `blocked` (`approval_unavailable`); `task.ended.needed_permissions` lists the rules and `nemo run` prints, e.g.:

    nemo: blocked: approval needed for Bash(cargo test:*) — rerun with --allow 'Bash(cargo test:*)' or in the TUI

## `--allow RULE` / `--deny RULE` (repeatable, `nemo run` and the interactive app)
Settings rule syntax (`Bash(cargo test:*)`, `Edit`, `mcp__server__tool`). Flags are typed by you, so trusted. Deny wins over allow. `--allow` cannot lift critical-path or protected-path protections. A malformed rule is a usage error (exit 2).
