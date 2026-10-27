# Nemo

Nemo is a model-agnostic autonomous coding agent written in Rust. It runs a plan, act, observe, verify loop against your repository, talks directly to Anthropic-compatible and OpenAI-compatible APIs, and confines the commands it runs in an OS sandbox.

Status: **pre-1.0 (0.1.0 draft)**. See [Status and limitations](#status-and-limitations) before relying on it.

## Install

Pick one (details in [docs/INSTALL.md](docs/INSTALL.md)):

```sh
# macOS / Linux: checksum + provenance verified (needs gh, else refuses), no sudo, installs to ~/.local/bin
curl --proto '=https' --tlsv1.2 -fsSL https://github.com/OWNER/Nemo-ACD/releases/latest/download/install.sh | sh
# any OS with Node >= 18: native binary via platform packages
npm install -g nemo-acd        # installs the command `nemo-acd`
```

Windows: `packaging/install.ps1`. Release archives, SHA256SUMS and build provenance are on the GitHub Releases page. (`OWNER/Nemo-ACD` is replaced by `node scripts/check-placeholders.mjs --set <owner>/<repo>`; releases refuse to run until then.)

## Quick start

Nemo uses API keys only (ADR-011): no subscription login, no keychain, nothing written to disk. Keys come from `NEMO_*` environment variables (no other variable prefix is read; other names only via an explicit `{env:NAME}` reference in your provider config).

```sh
export NEMO_API_KEY=...             # see docs/specs/config.md for the full list
                                    # (NEMO_ANTHROPIC_API_KEY, NEMO_AUTH_TOKEN, NEMO_BASE_URL, NEMO_MODEL, ...)
nemo doctor --cwd .                 # sandbox backend, shell, browser, git, config, proxy
nemo models                         # configured models
nemo run --cwd . --model provider/model --prompt "fix the failing test in src/parser"
```

`nemo run` is headless: events are JSON lines on stdout (schema in `docs/specs/events.md`). `--cwd` is required; Nemo never uses an implicit working directory. Budgets: `--max-turns`, `--max-tool-calls`, `--max-cost-usd`, `--max-wall-secs`. `--permission-mode default|acceptEdits|plan|dontAsk`.

## Sandbox and network

Commands the agent runs are confined by the OS where possible.

| flag | values | default |
|---|---|---|
| `--sandbox` | `enforce` (refuse to run if the command cannot be confined), `best-effort`, `off` (loud warning) | `enforce` |
| `--network` | `deny`, `loopback` (dev server + tests in one sandbox), `allow` (loud warning) | `deny` |
| `--sandbox-read PATH` | extra read-only path, e.g. a toolchain under `$HOME` | none |

Backends: Seatbelt (macOS), Landlock or bubblewrap (Linux), Job Object only (Windows, no filesystem or network confinement). Most guarantees are **Weak**, meaning defence in depth, not a security boundary. Read [docs/SECURITY.md](docs/SECURITY.md) for the per-OS table.

## Security model in brief

Model output, repository files, web pages and tool output are untrusted data. A permission gate sits before every tool call. Project config, hooks, skills and MCP servers from a repository are untrusted until you trust them. Subprocess environments are filtered by an allowlist. Full detail and vulnerability reporting: [docs/SECURITY.md](docs/SECURITY.md).

## Coexisting with the Node CLI

During the parallel run both CLIs can be installed. See [docs/MIGRATION.md](docs/MIGRATION.md).

## Status and limitations

- Not yet run end to end on GitHub CI, Linux or Windows by the maintainers; release pipeline and signing are unverified until the first tag.
- Windows has no filesystem or network sandbox; macOS Seatbelt is deprecated by Apple and best-effort.
- Eval parity with the Node CLI against real models is not yet measured (cutover gate).
- TUI, durable sessions and worktrees are incomplete; see `docs/STATUS.md`.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --features nemo-cli/test-provider
node tests/agent-evals/harness/selftest.js
```

Design: `docs/Nemo_Solution_Concept_and_Technical_Design.md`; contracts: `docs/specs/`. Release process: [docs/RELEASE.md](docs/RELEASE.md). License: MIT.
