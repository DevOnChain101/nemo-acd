# Nemo-ACD status

Goal: R1–R9, hand over the full Nemo autonomous coding agent. NOT complete. Nothing is committed (repo has no commits, no remote); CI has never run; only macOS (x86_64) was exercised.

## Verified (final run, macOS x86_64 only; tester report)
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` (also nemo-cli with `--features test-provider`): clean.
- `cargo test --workspace --no-fail-fast`: 670 passed, 0 failed, 0 ignored (63 test binaries + 21 doc-test sections), identical outcome lists on two runs; `cargo test -p nemo-cli --features test-provider`: 53 passed; selftest of the eval harness: 6/6; five-crate loop (browser, workspace, mcp, tui, tui-base) 5x: 210 passed each, no intermittent failures.
- Counting method (to compare future runs): `grep -E '^test result' log | awk '{p+=$4; f+=$6; i+=$8}'`.
- Golden: 717 cases, capture byte-identical on re-run; all `reviewed_by` are null.
- Release binary: builds, `--version`, `doctor`, no `test-provider` in the binary; `nemo run` with no credentials exits 2 (fails closed). `scripts/test-check-release-tag.sh` 9/9; action pins pass; placeholder check fails by design until the owner sets the repo.
- Security reviews: five passes (F/L/M series, #1-#19, pass-5 #1-#9); last pass had no critical/high findings; its fixes are in. Residual risks are listed in docs/SECURITY.md.

## Built (R1–R9 status)
- R1 contracts: docs/specs/*, eval harness (8 scenarios, oracle/noop/node/rust adapters), golden capture, Cargo workspace.
- R2 core: nemo-config, nemo-terminal (+sandbox integration, limits), nemo-hooks, nemo-tools (fs, exec, terminal_*, browser_*, verify_run, artifact_read, web, notebook, skill, pdf), nemo-permissions (hardened).
- R3: nemo-providers (Anthropic + OpenAI-compatible, own thin clients), nemo-mcp (no legacy SSE).
- R4: nemo-harness, nemo-verification, nemo-settings, nemo-context, apps/cli (`nemo run/auth/models/doctor/tui`).
- R5: nemo-tui-base, nemo-tui-render (95/95 render goldens), nemo-tui (interactive; limits listed in its report).
- R6: nemo-browser (own CDP client, pipe transport, URL policy on all targets).
- R7: nemo-sandbox (Seatbelt tested on macOS; Landlock/bwrap compile-only; Windows = Job Object reporting only).
- R8: nemo-session (SQLite store, recovery, Node compat), nemo-workspace (hardened git worktrees, checkpoint restore).
- R9a: CI + release pipeline + npm/installers/docs written, never run on GitHub.

## Decisions on record
ADR-010 Rust core; ADR-011 API keys only, user-managed via NEMO_* (standard vars per their standards; no other products' prefixes); no keychain; sandbox Enforce by default; network Deny by default (Loopback opt-in; macOS Loopback exposes host loopback services).

## Blocked / not done (needs the owner)
- Real-model eval baseline vs Node (needs API keys via NEMO_*): cutover gate 1 unmet.
- Linux/Windows: only per-crate compile checks for C-free crates; whole-workspace cross-check impossible on macOS (aws-lc-sys). Need CI on real runners; ConPTY, legacy conhost, Git Bash quoting, Landlock/bwrap enforcement, clipboard/notification tools unverified.
- Repo remote, branch protection (`ci-ok`), action SHA pinning (docs/RELEASE.md), signing secrets, npm names, OWNER placeholders.
- Specs and golden cases still `draft` / unreviewed by a human.
- Nodes C and F2 in the Node reference repo (legacyEnvWarnings test; copy `●` glyph) undecided.
- Security: third review in progress (see its report); known residual risks in docs/SECURITY.md.
- Cutover: parallel run, then retire Node (docs/MIGRATION.md checklist).
