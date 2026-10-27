# Changelog

Format: Keep a Changelog. Versioning: SemVer (pre-1.0: minor versions may break).

## [Unreleased]

### Security
- Release tag allowlist now fails closed: `scripts/check-release-tag.sh` exits non-zero when `.github/release-maintainers.txt` is missing or has no entries, unless repository variable `NEMO_ALLOW_ANY_VERIFIED_TAGGER` is exactly `true`. Tests: `scripts/test-check-release-tag.sh` (run in CI). Owner must add maintainer emails before the first tag.
- release.yml: `release` checkout no longer persists credentials; publishing moved to a separate `publish` job (environment approval) that re-checks the tag and the draft's `targetCommitish` before un-drafting.
- install.ps1 now requires PowerShell 7+ (it used `-SkipHttpErrorCheck`) and fails early with an install hint; docs no longer claim 5.1.
- Release gate: `scripts/check-release-tag.sh` also requires the tag to point at the built commit, the signed tag object to carry the tag's name and target that commit, and the API ref to match; it is re-run in the `release` job before and after creating the draft, which is now created with `--target $GITHUB_SHA`. Optional maintainer allowlist `.github/release-maintainers.txt` (empty = off); `Verified by GitHub` trust caveat documented.
- Installers fail closed when `gh` is missing (`NEMO_SKIP_ATTESTATION=1` to accept checksum-only; `NEMO_VERIFY` removed, now the default) and verify the attestation with `--source-ref refs/tags/v<version>`.
- ci.yml: checkouts no longer persist the token (`persist-credentials: false`).
- Release pipeline hardening: actions pinned to commit SHAs (enforced by `scripts/check-action-pins.mjs`), Dependabot, `release` environment, main-ancestor and signed-tag gate, per-job least-privilege permissions, no `${{ }}` of user data in `run:`, no build cache in release builds, SHA256SUMS and installers attested.
- Installers: https-only downloads, version allow-list, attestation verification of SHA256SUMS by default; repository slug from `packaging/REPO` with hard failure on the `OWNER` placeholder (`scripts/check-placeholders.mjs`).

## [0.1.0] - DRAFT, not released

### Added
- Rust core engine workspace: config, permissions, tools, terminal runtime, hooks, providers (Anthropic-compatible and OpenAI-compatible), MCP client, sessions, verification engine, browser runtime, OS sandbox backends (Seatbelt, Landlock, bubblewrap, Windows Job Object).
- Headless CLI `nemo` (`run`, `auth`, `models`, `doctor`) emitting JSONL events.
- API-key-only model access via `NEMO_*` environment variables (ADR-011).
- Sandbox controls `--sandbox enforce|best-effort|off`, `--network deny|loopback|allow`, `--sandbox-read`.
- Release engineering: 3-OS CI, MSRV, docs, cargo-deny/audit, Linux sandbox and browser jobs; tag-triggered release pipeline with checksums, provenance, SBOM, optional signing; npm platform packages (`nemo-acd`), install scripts, Homebrew/winget templates.

### Known limitations
- CI and the release pipeline have not run on GitHub; only macOS has been exercised.
- Windows has no filesystem or network sandbox; macOS Seatbelt is deprecated.
- Eval parity with the Node CLI is unmeasured; TUI, durable sessions and worktrees incomplete.
- `cargo audit` currently reports RUSTSEC-2026-0009 in `time 0.3.45` (fixed in >= 0.3.47); update `Cargo.lock` before release.
