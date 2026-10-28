# Migrating from the Node CLI

The Node CLI (`~/Vibe/nemo-cli`) is frozen and retired: the owner no longer uses it (2026-10-28), so a parallel run is NOT required for cutover. It stays read-only as a behaviour/fixture reference (ADR-010). If you still have it installed, both can coexist as described below.

## Coexistence

- **Command names.** The Rust binary is `nemo`; the Node CLI also uses `nemo`/`nemocode`. The npm package for the Rust build is `@devonchain101/nemo` (command `nemo`); pass `--bin-names` to `scripts/package-npm.mjs` for another command name if you must coexist. `install.sh`/`install.ps1` take `NEMO_BIN_NAME` (e.g. `nemo-rs`).
- **PATH precedence.** Whichever directory comes first wins for plain `nemo`. The installers warn when another `nemo` is already earlier on PATH. Check with `which -a nemo` (`where nemo` on Windows).
- **Shared `NEMO_HOME`** (default `~/.nemo`). Both read `settings.json`, provider config and `projects/<cwd>/<session>.jsonl` transcripts. The Rust store adds `nemo.db` and `sessions/` next to them; the Node CLI ignores these. Use a separate home for experiments: `NEMO_HOME=/tmp/nemo-try nemo ...`.
- **`compat.write_node_transcripts`** (default ON): the Rust session store also mirrors each session as a Node-format JSONL transcript so Node can resume/list it. Node is retired, so turn it off unless you still use Node.
- **Keys.** Both use `NEMO_*` variables only (ADR-011). The Rust build never reads `ANTHROPIC_*`, `OPENAI_*`, `CLAUDE_CONFIG_DIR` or `CODEX_HOME`. Import from `~/.claude` / `~/.codex` takes an explicit path.
- **Config and sessions.** Formats stay compatible for anyone still running Node (no longer a cutover gate). Config layering is a golden-tested faithful port (530 fixtures, `tests/golden`). Do not assume a session written by one side resumes cleanly on the other.
- **Behaviour differences to expect.** Rust requires `--cwd`; sandbox defaults to `enforce` with network `deny` (the Node CLI had no OS sandbox); no plaintext `auth.json`.

## Cutover gates (design doc section 10, Phase 3)

Mark each with evidence (run link or output) before cutover. Node is already retired, so no parallel run or Node baseline is needed:

- [ ] Rust eval passes on a real model (`--agent rust`; needs API keys). A Node baseline comparison is optional.
- [ ] Golden tests green on macOS, Linux and Windows CI (CI has never run on GitHub).
- [ ] Open audit items verified on real runs: ConPTY, legacy conhost, Git Bash quoting, clipboard/notification tools per OS.
- [ ] Release pipeline produced signed, verified artifacts for the 6 targets (macOS and Windows signing, notarization).
- [ ] Linux sandbox job green with Landlock and bubblewrap on a real runner.
- [ ] Browser job green on all three OSes.
- [ ] Independent security re-review of the fixed audit findings (docs/STATUS.md).

## Rollback

If you kept a Node install, to go back: remove the Rust install (docs/INSTALL.md), put the Node CLI first on PATH. `$NEMO_HOME` data written by Node is untouched by the Rust build; `nemo.db` and `sessions/` can be deleted safely.
