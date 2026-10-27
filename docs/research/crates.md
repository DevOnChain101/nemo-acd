# Candidate crates — verification (2026-10-26)

Source: researcher agent (crates.io/docs.rs/README via web). Versions marked ✔ were re-checked locally with `cargo info`; the rest are medium confidence. Feature/platform claims come from docs, not from running the code. Re-verify per-OS behavior with a spike before depending on it.

| Area | Crate | Version | MSRV | Verdict | Caveat |
|---|---|---|---|---|---|
| MCP client | rmcp | 3.7.0 ✔ | 1.88 | adopt w/ caveats | stdio + streamable HTTP; no legacy SSE client; 3.x migration churn |
| Anthropic API | (none official) | – | – | own thin client on reqwest + SSE | community `anthropic-ai-sdk` is unofficial; avoid in core |
| OpenAI API | async-openai | 0.42.2 | 1.75 | adopt w/ caveats | OpenAI side only; gateway quirks need handling; `api_base` field name unconfirmed |
| HTTP | reqwest | 0.13.5 ✔ | 1.85 | adopt | |
| Async | tokio | 1.54.1 ✔ | 1.85 | adopt | |
| TUI | ratatui | 0.30.2 ✔ | 1.88 | adopt | |
| Terminal | crossterm | 0.29.0 ✔ | 1.63 | adopt w/ caveats | last release 2025-04; Kitty keyboard on Windows unverified |
| PTY | portable-pty | 0.9.0 | – | adopt w/ caveats | last release 2025-02; ConPTY needs Windows 10 1809+; spike required |
| Process tree | process-wrap | 10.0.1 ✔ | 1.87 | adopt w/ caveats | group/Job Object tree-kill; Windows behavior unverified |
| Linux sandbox | landlock | 0.4.7 | 1.71 | adopt (Linux) | kernel ≥5.13; best-effort by default → mark features required |
| macOS sandbox | sandbox-exec | – | – | best-effort | deprecated, no Apple replacement; do not promise a guarantee |
| Linux sandbox alt | bubblewrap | – | – | adopt w/ caveats | Ubuntu 24.04 AppArmor blocks unprivileged userns by default |
| Windows sandbox | Job Object (+ AppContainer via `rappct`) | – | – | weak | Job Object = resource caps + tree kill, not FS/network confinement |
| Browser (Rust) | chromiumoxide 0.9.1 / headless_chrome 1.0.22 | – | 1.85 | spike (R6) | maintenance and Windows support unclear; a11y tree/console capture unconfirmed |
| Storage | rusqlite (bundled) | 0.41.0 ✔ | 1.89 | adopt | highest MSRV in the set |
| FS watch | notify | 8.2.1 | 1.85 | adopt | |
| Search | ignore | 0.4.x | 1.88 | adopt | |
| Schema | schemars | 1.2.3 | 1.74 | adopt | |
| Credentials | keyring | 4.2.0 ✔ | 1.88 | not needed | ADR-011: users manage keys via env vars; no keychain |
| Clipboard | arboard | 3.7.0 | 1.71 | adopt w/ caveats | Wayland and Windows UTF-16 unverified |
| Git | git2 | 0.21.0 | – | defer | shell out to `git` for worktrees initially |
| LSP | async-lsp | 0.2.4 | 1.66 | defer | 0.x churn |
| Syntax | tree-sitter | 0.27.1 | 1.90 | defer | not needed R2–R4 |

## Minimal dependency set
- R2 core: tokio, serde, serde_json, schemars, clap, rusqlite (bundled), notify, ignore, process-wrap.
- R3 providers/MCP: rmcp, reqwest, eventsource-stream, own Anthropic and OpenAI-compatible clients.
- R4/R5: ratatui, crossterm, arboard (or defer).
- Defer: browser, sandbox crates (add per OS at R7), tree-sitter, async-lsp, git2.
- Workspace `rust-version` must rise to the max MSRV in use (1.89 with rusqlite 0.41).

## Unverified / conflicts
- Whether Anthropic has announced an official Rust SDK (none found).
- rmcp README says it implements "MCP 2026-07-28" while crates.io is 3.7.0 — check its changelog before pinning.
- Not checked: unicode-width/segmentation, clap, nix/rustix/windows-sys, sqlx, Playwright sidecar, ratatui mouse/paste/truecolor behavior.
- Per-OS spikes needed before R2: process-wrap (Windows), arboard, ConPTY.
