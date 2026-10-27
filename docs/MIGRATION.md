# Migrating from the Node CLI (parallel run)

The Node CLI (`~/Vibe/nemo-cli`) is frozen and stays the reference until cutover (ADR-010). Both can be installed at once.

## Coexistence

- **Command names.** The Rust binary is `nemo`; the Node CLI also uses `nemo`/`nemocode`. The npm package and shim for the Rust build are named `nemo-acd` (command `nemo-acd`) so installs never collide. `install.sh`/`install.ps1` take `NEMO_BIN_NAME` (e.g. `nemo-rs`).
- **PATH precedence.** Whichever directory comes first wins for plain `nemo`. The installers warn when another `nemo` is already earlier on PATH. Check with `which -a nemo` (`where nemo` on Windows).
- **Shared `NEMO_HOME`** (default `~/.nemo`). Both read `settings.json`, provider config and `projects/<cwd>/<session>.jsonl` transcripts. The Rust store adds `nemo.db` and `sessions/` next to them; the Node CLI ignores these. Use a separate home for experiments: `NEMO_HOME=/tmp/nemo-try nemo ...`.
- **`compat.write_node_transcripts`** (default ON until R9): the Rust session store also mirrors each session as a Node-format JSONL transcript so Node can resume/list it. Turn it off only once Node is retired, or if you do not want two copies.
- **Keys.** Both use `NEMO_*` variables only (ADR-011). The Rust build never reads `ANTHROPIC_*`, `OPENAI_*`, `CLAUDE_CONFIG_DIR` or `CODEX_HOME`. Import from `~/.claude` / `~/.codex` takes an explicit path.
- **Config and sessions.** Formats must read/write compatibly both ways during the parallel run (cutover gate). Config layering is a golden-tested faithful port (530 fixtures, `tests/golden`). Until the gate is checked off, do not assume a session written by one side resumes cleanly on the other.
- **Behaviour differences to expect.** Rust requires `--cwd`; sandbox defaults to `enforce` with network `deny` (the Node CLI had no OS sandbox); no plaintext `auth.json`.

## Cutover gates (design doc section 10, Phase 3)

Mark each with evidence (run link or output) before retiring Node:

- [ ] Rust eval >= Node baseline on the same models (`node tests/agent-evals/harness/run.js --agent rust|node`; needs API keys; the Node baseline with a real model is not recorded yet).
- [ ] Golden tests green on macOS, Linux and Windows CI (CI has never run on GitHub).
- [ ] Open audit items verified on real runs: ConPTY, legacy conhost, Git Bash quoting, clipboard/notification tools per OS.
- [ ] Session and config formats read/write compatible both ways (cross-resume test).
- [ ] Release pipeline produced signed, verified artifacts for the 6 targets (macOS and Windows signing, notarization).
- [ ] Linux sandbox job green with Landlock and bubblewrap on a real runner.
- [ ] Browser job green on all three OSes.
- [ ] Independent security re-review of the fixed audit findings (docs/STATUS.md).

## Rollback

Keep the Node CLI installed until the checklist is complete. To go back: remove the Rust install (docs/INSTALL.md), put the Node CLI first on PATH. `$NEMO_HOME` data written by Node is untouched by the Rust build; `nemo.db` and `sessions/` can be deleted safely.
