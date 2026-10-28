# Importing a Claude Code setup (`nemo import claude`)

`nemo import claude` COPIES and CONVERTS your Claude Code configuration into Nemo-owned files so
Nemo runs in an equivalent environment. Nemo never reads `~/.claude` at runtime (ADR-011, only
`NEMO_*` settings); after the import the Nemo files are yours and the Claude ones are untouched.

```
nemo import claude [--scope user|project|all] [--project-dir DIR] [--from DIR]
                   [--dry-run | --apply] [--only LIST]
                   [--merge | --overwrite | --skip-existing]
                   [--accept-broad-allow] [--accept-hooks]
```

- Default is a **dry run**: a per-item plan (`source -> target : create|update|unchanged|skip`,
  warnings, a summary and a checklist of manual steps). `--apply` writes.
- `--from` defaults to `$HOME/.claude`; `~/.claude.json` is read from its parent directory.
  `--project-dir` is required for the project scope (`all` without it imports the user scope only).
  Targets are `$NEMO_HOME` (default `~/.nemo`) and `<project>/.nemo`. All paths derive from the
  injected environment, so `HOME=/tmp/x NEMO_HOME=/tmp/x/.nemo nemo import claude --from /tmp/x/.claude`
  never touches the real home.
- `--only` takes `instructions,permissions,hooks,env,agents,skills,commands,mcp,memory`
  (default: all but `memory`).

## What goes where

| Claude source | Nemo target | Notes |
|---|---|---|
| `~/.claude/CLAUDE.md`, `~/.claude/rules/*.md` | `$NEMO_HOME/NEMO.md`, `rules/` | header `<!-- imported from ... on DATE by nemo import claude -->`, content verbatim; relative `@imports` are copied to `imports/` and rewritten; path-scoped rules (`paths:`) load for every task (warned) |
| project `CLAUDE.md`, `CLAUDE.local.md`, `.claude/CLAUDE.md` | nothing | Nemo already loads them from the project. Only when `.claude/rules/*.md` exist: `.nemo/rules/*` plus a project `NEMO.md` (base text + `@.nemo/rules/..` lines). A project `NEMO.md`/`AGENTS.md` wins over `CLAUDE.md` (reported) |
| `settings.json` (`.claude/settings.local.json` for the project) | `settings.json` (`.nemo/settings.local.json`, listed in `.nemo/.gitignore`) | `permissions.allow/ask/deny/additionalDirectories/defaultMode`, `hooks`, `env` |
| `agents/*.md`, `skills/*/` (whole directory), `commands/**/*.md` | `agents/`, `skills/`, `commands/` | frontmatter kept; tool names translated; unsupported fields listed. Nemo has no slash-command loader yet: commands are staged only |
| `~/.claude.json` `mcpServers` | `$NEMO_HOME/nemo.json` `mcpServers` (file gets `$schema` if new) | see MCP below |
| `~/.claude.json` `projects.<dir>.mcpServers` | `nemo.json` `projects.<dir>.mcpServers` | |
| project `.mcp.json` | untouched | Nemo reads it itself; the plan prints `--approve-mcp NAME@HASH` |
| `~/.claude/projects/<dir>/memory/*.md` (`--only memory`) | `$NEMO_HOME/projects/<dir>/memory/` | staged; Nemo does not load auto-memory yet |

Tool names: `Agent->Task`, `MultiEdit->Edit`, `LS->Glob`, `KillBash->KillShell`; `Bash`, `Read`,
`Edit`, `Write`, `Glob`, `Grep`, `WebFetch`, `WebSearch`, `Task`, `Skill`, `mcp__*` and
`Bash(spec)` forms are kept. Tools Nemo lacks (`PowerShell`, `Monitor`, ...) are dropped with a
warning; an agent whose tools all vanish gets an empty `tools:` (no tools), never "all tools".

## Deliberately NOT imported (security rationale)

- **Credentials**: `apiKeyHelper`, OAuth accounts, tokens. Nemo never writes credentials.
- **`defaultMode: bypassPermissions` or `auto`**: mapped to `default` with a warning.
- **Broad allow rules** (`Bash`, `Bash(*)`, `Write`, `Edit`, `WebFetch`, `*`): skipped unless `--accept-broad-allow`.
- **Rules Nemo cannot represent** (unknown tool, malformed): dropped with a warning. A dropped `deny` rule is flagged loudly: that restriction is NOT carried over.
- **User-scope hooks**: printed (credential-looking words masked) and written only with `--accept-hooks`. Unsupported events (e.g. `SubagentStart`) are skipped. Project hooks are written but stay inert until the project is trusted (`--trust-project`). Treat imported hooks as unreviewed; `nemo doctor` shows the count.
- **`env`**: secret-shaped names (`*KEY*`, `*TOKEN*`, `*SECRET*`, `*PASSWORD*`, ...) and `ANTHROPIC_/OPENAI_/CLAUDE_/CODEX_` prefixes are skipped; only names are listed.
- `model` (noted only; use `NEMO_MODEL`), `statusLine`, `enabledPlugins`, theme and other keys (listed by name).

## MCP servers

Entries are fitted to Nemo's closed `config.json/v1` schema (unknown keys dropped by name,
`disabled` becomes `enabled`, unknown `type` with a `url` becomes `http`). **Literal secrets are
never copied**: a header/env/flag whose name is secret-shaped, a known token prefix (`ghp_`, `sk-`,
`xox*-`, `AKIA`, JWT...), a long random string, URL userinfo or a secret-shaped query parameter is
replaced by `${SERVER_KEY}` (`Authorization: Bearer x` becomes `Bearer ${SERVER_TOKEN}`).
Existing `${VAR}` references are kept. The checklist lists every variable you must export and add
to `nemo.mcp.env_passthrough` in the user `settings.json` (the import does not widen that list
for you). OAuth servers are imported as-is with a warning; the login is left to the provider package.
The plan shows each definition hash. Project servers need hash approval as always; servers in
your own `nemo.json` start without it (it is your file, and you just reviewed the plan).

## Safety properties

- All sources and targets are parsed before the first write: a malformed Claude or Nemo file is a
  clear error (exit 2, no content echoed) and nothing is written.
- Each changed file is backed up first as `NAME.bak-YYYYmmdd-HHMMSS`, then replaced atomically
  (temp file + rename), mode 0600 on Unix.
- Idempotent: a second run reports `unchanged` and writes nothing (no new backup).
- `--merge` (default): lists are unioned, scalars and existing servers keep your value, files
  Nemo did not write itself are kept (reported). `--overwrite` replaces what the import manages.
  `--skip-existing` never touches an existing file.
- Copied files are tracked in `<root>/imports/claude.json`; `nemo doctor` prints its counts and which instruction files apply.

## Example

```
$ nemo import claude --project-dir ~/code/app            # plan only
$ nemo import claude --project-dir ~/code/app --apply    # write
$ nemo import claude --apply --accept-hooks --only hooks # after reviewing the hooks
```

## Undo

Restore a file from its backup (`mv ~/.nemo/settings.json.bak-20261029-101500 ~/.nemo/settings.json`)
and delete files that were `create`d (listed in `imports/claude.json`). Nothing under `~/.claude`
is ever modified.
