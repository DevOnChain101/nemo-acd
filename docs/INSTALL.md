# Installing Nemo

Nemo is one native binary, `nemo`. On Linux a helper, `nemo-sandbox-exec`, must sit in the same directory (the Landlock sandbox backend looks for it next to `nemo`). All methods below keep them together. Nothing needs sudo. UNVERIFIED on GitHub until the first release exists; the npm and `install.sh` paths were exercised locally against a locally built archive (macOS x86_64 only).

## 1. Install script (macOS, Linux)

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://github.com/DevOnChain101/nemo-acd/releases/latest/download/install.sh | sh
```

Environment: `NEMO_VERSION` (must match `^[0-9A-Za-z.+-]+$`), `NEMO_REPO`, `NEMO_INSTALL_DIR` (default `~/.local/bin`), `NEMO_BIN_NAME` (default `nemo`), `NEMO_SKIP_ATTESTATION=1` (accept a checksum-only install; the default requires `gh`), `NEMO_DOWNLOAD_BASE` (must be https; `NEMO_INSECURE_DOWNLOAD_BASE_I_KNOW=1` allows others, tests only).

Verification: all downloads use `curl --proto '=https' --tlsv1.2`. The archive's sha256 is checked against `SHA256SUMS`. When `gh` is installed, `SHA256SUMS` itself is checked with `gh attestation verify` (build provenance from `release.yml`, pinned to that workflow and the `refs/tags/v<version>` source ref; needs `gh auth login`) and a failure aborts. Without `gh` the installer prints a warning and continues with the checksum only (the checksum file then comes from the same server as the archive and proves integrity, not origin); `NEMO_SKIP_ATTESTATION=1` does the same even when `gh` exists. After installing it runs `nemo doctor`. The `install.sh` you pipe into `sh` is not covered by this: download it, read it, and verify it with `gh attestation verify install.sh --repo DevOnChain101/nemo-acd`. Prefer downloading `install.sh` first and reading it. musl systems get the x86_64 musl build when published.

## 2. Windows

`install.ps1` from the release (needs PowerShell 7+ (`pwsh`); same variables and the same checksum/attestation behaviour; installs to `%LOCALAPPDATA%\Programs\Nemo` and runs `nemo doctor`). Binaries are not Authenticode-signed, so SmartScreen may warn; release provenance is the GitHub attestation. To test a dry-run build (docs/RELEASE.md, Dry run) without a release, unzip the `nemo-<version>-x86_64-pc-windows-msvc.zip` from the `release-assets` artifact and run `nemo.exe doctor`.

## 3. npm (any OS with Node >= 18)

```sh
npm install -g @devonchain101/nemo
nemo --version
```

`@devonchain101/nemo` is a small JS shim that resolves the platform package `@devonchain101/nemo-<os>-<arch>` (an optional dependency holding the binary) and runs it, forwarding the exit code and SIGTERM/SIGHUP/SIGQUIT. SIGINT is not forwarded because the terminal already sends Ctrl-C to the child. Do not install with `--omit=optional`/`--no-optional`; the shim then reports that no native binary was found.

## 4. Manual

Download `nemo-<version>-<target>.tar.gz|zip` and `SHA256SUMS` from the release, check `sha256sum -c --ignore-missing SHA256SUMS`, `gh attestation verify SHA256SUMS --repo DevOnChain101/nemo-acd` (authenticates the checksum file) and optionally the archive itself, extract, and put the contents of the directory on your PATH. Targets: `x86_64`/`aarch64` `apple-darwin`, `unknown-linux-gnu`, `pc-windows-msvc`; `x86_64-unknown-linux-musl` is experimental.

## 5. Homebrew / winget

Templates only: `packaging/homebrew/nemo.rb.tmpl`, `packaging/winget/*.tmpl`. Not published. Fill `@VERSION@` and the `@SHA256_*@` values from the release `SHA256SUMS`.

## Requirements and checks

- git on PATH; a Chrome-family browser only for browser features (`NEMO_BROWSER_PATH` to choose one; Nemo never downloads a browser).
- Linux sandbox: the default `--sandbox enforce` needs `bubblewrap` (`bwrap`) with unprivileged user namespaces. Landlock alone (kernel 5.13+) cannot express the standard policy (read-only `.git/hooks` inside the writable workspace), so `nemo doctor` shows `enforce_ready: no` and `nemo run` refuses unless you pass `--sandbox best-effort`. On Ubuntu 24.04 bubblewrap needs `sudo sysctl kernel.apparmor_restrict_unprivileged_userns=0` (or an AppArmor profile for bwrap).
- Run `nemo doctor --cwd .` and read the sandbox section.
- Keys: `NEMO_*` environment variables only (README, docs/specs/config.md).

## Uninstall

Delete the binary (and `nemo-sandbox-exec`), or `npm rm -g @devonchain101/nemo`. State lives in `$NEMO_HOME` (default `~/.nemo`), which is shared with the Node CLI; remove it only if you want to drop sessions too.
