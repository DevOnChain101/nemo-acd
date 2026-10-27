# Releasing

Everything here is UNVERIFIED on GitHub: the workflows were validated locally only for YAML syntax and the individual commands (see the end of this file).

## Steps

1. Make sure `main` is green (`ci.yml`, including cargo-deny and cargo-audit).
2. Bump `[workspace.package] version` in `Cargo.toml`, update `Cargo.lock` (`cargo update -w`), move CHANGELOG entries from Unreleased, commit, merge.
3. Tag and push: `git tag -s vX.Y.Z -m vX.Y.Z && git push origin vX.Y.Z`. The tag must equal the workspace version, point at a commit on `main`, be annotated, and GitHub must report its signature as verified (all checked by the `gate` job via `scripts/check-release-tag.sh`; upload your signing key to GitHub first, as a signing key). The script also requires `refs/tags/<tag>^{commit}` to equal the commit the workflow runs on, the API tag object to be named exactly the tag and to target that commit, and the API ref to resolve to that tag object. **Trust caveat:** "Verified by GitHub" only says the signature matches a key of some GitHub account; any account that can push tags passes. To restrict who may release, list maintainer tagger emails (one per line) in `.github/release-maintainers.txt`; the file is read from the tagged commit (so changing it needs a reviewed merge to `main`). **The policy fails closed:** a missing file or one with no entries makes `scripts/check-release-tag.sh` fail, unless the repository variable `NEMO_ALLOW_ANY_VERIFIED_TAGGER` is exactly `true` (an explicit owner decision: any GitHub account that can push a `v*` tag with a verified signature can then release; passed from `vars.` into the scripts' env in `gate`, `release` and `publish`). The GitHub API does not expose the signer's login, so the allowlist matches the tagger email in the signed tag, which GitHub verified the signature against (GPG: a verified email of the key's account). Pair it with tag protection rules / a ruleset restricting who may create `v*` tags.
4. `release.yml` first runs `gate` (ancestor of main, signed annotated tag, `scripts/check-action-pins.mjs`, `scripts/check-placeholders.mjs --expect <this repo>`), then re-runs the whole CI on the tagged commit (any red job blocks the build), builds 7 targets (musl x86_64 and windows-arm64 are `experimental`: may fail without blocking), proves `test-provider` is absent (`scripts/check-no-test-provider.sh`), smoke-tests (`--version`, `doctor`), signs if secrets exist, packages, makes SBOM, npm tarballs, SHA256SUMS (stamping the installers with the slug from `packaging/REPO`), a provenance attestation covering archives, npm tarballs, SHA256SUMS and the installers (job `attest`, the only one with `id-token`), and creates a DRAFT release (job `release`, the only one with `contents: write`). The `release` job re-runs `scripts/check-release-tag.sh` right before `gh release create ... --verify-tag --target "$GITHUB_SHA"` (approval can take days, the tag may have been moved meanwhile) and once more after creation; if the tag moved, it deletes the draft and fails. Jobs `build`, `attest`, `release` and `publish` run in the `release` environment and wait for its reviewers. The draft is NOT public until the small final `publish` job (separate approval) re-runs `scripts/check-release-tag.sh`, checks the release is still a draft whose `targetCommitish` equals `$GITHUB_SHA` (`gh release view --json isDraft,targetCommitish`), and only then runs `gh release edit --draft=false`. The `release` and `publish` checkouts use `persist-credentials: false`; the script passes `GH_TOKEN` as a one-off `http.extraheader` for its tag fetch only.
5. Review the draft: 5 required archives present, SHA256SUMS, SBOM. Download one archive per OS, run `gh attestation verify <file> --repo OWNER/Nemo-ACD` (also for `SHA256SUMS`), run `nemo --version`/`nemo doctor`.
6. Approve the `publish` job (it publishes the draft after re-verifying tag and target; do not publish by hand unless that job is unavailable). Then (manual, irreversible): `npm publish` each tarball from the release (platform packages first, `nemo-acd` last; use `--provenance` from CI or `--access public`), fill and submit the Homebrew formula and winget manifests from `packaging/*.tmpl` with the hashes in SHA256SUMS.

## Secrets (ENVIRONMENT secrets of the `release` environment; all optional, steps skip with a log line when absent)

Do NOT store these as repository or organisation secrets: those are readable by any workflow job, including ones from branches. Environment secrets are only released to jobs that declare `environment: release` and pass its protection rules.

| Secret | Purpose |
|---|---|
| `MACOS_CERTIFICATE` | base64 of a "Developer ID Application" .p12 |
| `MACOS_CERTIFICATE_PASSWORD` | its password |
| `MACOS_SIGN_IDENTITY` | e.g. `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` | notarytool credentials (app-specific password) |
| `WINDOWS_CERTIFICATE` | base64 of the Authenticode .pfx |
| `WINDOWS_CERTIFICATE_PASSWORD` | its password |

An unsigned release is still produced when they are absent. Nothing needs a token beyond the default `GITHUB_TOKEN`. Bare Mach-O binaries cannot be stapled; Gatekeeper checks the notarization ticket online. Consider a signed .pkg/.dmg later.

## One-time owner setup

- Repository slug, the one place to edit: run `node scripts/check-placeholders.mjs --set <owner>/<repo>`. It writes `packaging/REPO` (the single source read by `scripts/package-npm.mjs` and stamped into the released `install.sh`/`install.ps1`) and replaces `OWNER/Nemo-ACD` in README.md and docs/INSTALL.md. The Homebrew/winget templates use `@REPO@`/`@REPO_OWNER@` tokens you fill at submission. Until this is done the release `gate` fails, `package-npm.mjs` aborts, and unstamped installers refuse to run.
- Create the `release` environment (Settings, Environments): required reviewers (at least one person other than the tag pusher; enable "prevent self-review"), "Deployment branches and tags" restricted to the tag pattern `v*`, then add the signing secrets above as environment secrets. Remove any signing secrets from repository level.
- Upload your tag-signing key (GPG or SSH, as a signing key) to GitHub so tags show "Verified". Without it the `gate` job rejects every tag. Fill `.github/release-maintainers.txt` with the tagger emails BEFORE the first tag (the gate refuses otherwise), or set the repository variable `NEMO_ALLOW_ANY_VERIFIED_TAGGER=true` (Settings, Secrets and variables, Actions, Variables) and accept that any verified tagger passes.
- Tag protection: add a ruleset for `refs/tags/v*` (restrict creation to maintainers, block update and deletion so a tag cannot be moved), and enable immutable releases (Settings, General) so a published release and its tag cannot be altered. The `release` environment must have required reviewers (they gate `build`, `attest`, `release` and `publish`).
- Branch protection on `main` (required reviews, required `ci-ok`): the gate trusts that anything on `main` was reviewed, and runs the check scripts from the tagged commit.
- Settings, Actions, General: "Workflow permissions" read-only by default; require approval for outside-collaborator PR workflows.
- Repository settings: enable private vulnerability reporting; branch protection requiring the `ci-ok` check; allow Actions to create attestations (public repos, or a plan that supports them).
- Reserve npm names `nemo-acd` and `nemo-acd-*`.
- Runner labels used: `ubuntu-22.04-arm` and `windows-11-arm` (free for public repos only); `macos-latest` builds both darwin targets. Private repos need paid or self-hosted runners.
- Linux bwrap on `ubuntu-24.04`: the CI sandbox job runs `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0`; without it bubblewrap fails with "setting up uid map: Permission denied". The job also fails (instead of silently skipping) if the runner kernel lacks Landlock.

## Action pinning

Every third-party `uses:` in `ci.yml` and `release.yml` is pinned to a full commit SHA with the tag in a trailing comment. Each SHA was resolved on 2026-10-27 from the GitHub API (`GET /repos/<owner>/<repo>/git/ref/tags/<major-tag>`, annotated tags dereferenced), not from memory; I did NOT audit the action source at those commits. The major tags move, so the SHAs are whatever the tag pointed at that day.

| Action | Tag | Pinned SHA |
|---|---|---|
| actions/checkout | v4 | 11d5960a326750d5838078e36cf38b85af677262 |
| actions/setup-node | v4 | 49933ea5288caeca8642d1e84afbd3f7d6820020 |
| actions/setup-python | v5 | a26af69be951a213d495a4c3e4e4022e16d87065 |
| actions/upload-artifact | v4 | ea165f8d65b6e75b540449e92b4886f43607fa02 |
| actions/download-artifact | v4 | d3f86a106a0bac45b974a628896c90dbdf5c8093 |
| actions/attest-build-provenance | v2 | e8998f949152b193b063cb0ec769d69d929409be |
| Swatinem/rust-cache | v2 | d1527c8f67ef9af357290475d4af4b0fab8f12e0 (CI only; removed from release builds) |
| browser-actions/setup-chrome | v1 | c785b87e244131f27c9f19c1a33e2ead956ab7ce (`chrome-version: stable`; pin a concrete version for reproducibility) |
| EmbarkStudios/cargo-deny-action | v2 | 3c6349835b2b7b196a839186cb8b78e02f7b5f25 |
| taiki-e/install-action | v2 | 5488ac5a951645ce95062214902ac4b6bc1a1037 |
| raven-actions/actionlint | v2 | 3d39aea434753780c3b3d4a1a31c854b4dbf49d7 |

None are left unpinned. If one ever is (a new action, a manual edit), `release.yml` fails in `gate` via `scripts/check-action-pins.mjs`; ordinary CI does not. To resolve: `gh api repos/<owner>/<repo>/git/ref/tags/<tag> --jq .object` (if `.type` is `tag`, follow `.url` and take `.object.sha`), put `@<sha> # <tag>` in the workflow, rerun the script. `.github/dependabot.yml` (github-actions, cargo) proposes bumps weekly; review each. Dependabot has no npm entry because no npm manifest with dependencies exists.

Also unpinned by design: `rust-toolchain.toml` uses `channel = "stable"`; MSRV is checked separately at 1.89.

## Rollback

- Unpublished draft: delete the draft release and the tag (`git push --delete origin vX.Y.Z`).
- Published release with a bad binary: mark it pre-release or delete the release, publish a fixed patch version; do not move tags. Homebrew/winget: revert the PR or ship the fix version.
- npm: `npm deprecate nemo-acd@X.Y.Z "reason"` (unpublish is only possible within 72 hours and discouraged); publish X.Y.Z+1. Users pin back with `npm i -g nemo-acd@<good>`.
- Installers always take `NEMO_VERSION=<good>`.

## Validated locally (macOS x86_64) vs not

Validated: `scripts/test-check-release-tag.sh` (also run in ci.yml `shell-scripts`: empty list fails, missing file fails, flag `true` passes, flag not exactly `true` fails, listed email passes, unlisted fails even with the flag, token-header fetch); both workflow files parse as YAML; `cargo build --release --locked -p nemo-cli`; `scripts/check-no-test-provider.sh` passes on the release binary and fails on a `test-provider` debug build; `scripts/package-npm.mjs` + `npm pack` + install into a temp dir + `nemo-acd --version` / `doctor`; `install.sh` against a local archive (success; refusals for bad checksum, http base, bad version, OWNER/unstamped/malformed repo; a stub `gh` for pass, wrong `--source-ref`, no `gh` (refuses, exit 1) and `NEMO_SKIP_ATTESTATION=1` (proceeds with a warning)); `scripts/check-release-tag.sh` against a local origin with a stub `gh` (pass; lightweight tag; tag/GITHUB_SHA mismatch; unverified; wrong tag name; API object sha differing; API ref differing; allowlist empty/match/miss); `check-action-pins.mjs` and `check-placeholders.mjs` pass and fail cases; both workflows parse as YAML (ruby); the release stamping loop; `cargo deny check` (after fixing the policy) and `cargo audit`; `cargo cyclonedx` output. Not validated: everything that runs on GitHub (CI, release, the new gate job and its tag-signature API check, environment approvals, `permissions`, the split attest/release jobs, attestation, real `gh attestation verify` and its `--signer-workflow`/`--source-ref` flags, the real API fields `scripts/check-release-tag.sh` reads (`verification.verified`, `tag`, `object.*`, `tagger.email`), `gh release create --target` with an existing tag, SBOM job, drafting), all signing and notarization, Linux/Windows builds and the cross targets, musl, `install.ps1` (no pwsh available; reviewed only; it now requires PowerShell 7+), the `publish` job and `gh release view --json targetCommitish` for a draft created with `--target <sha>`, the `persist-credentials: false` + extraheader fetch on GitHub, shellcheck and actionlint (not installed here; ci.yml runs actionlint), Homebrew/winget templates, the Chrome action.
