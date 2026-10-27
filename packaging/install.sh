#!/bin/sh
# Nemo installer (macOS, Linux). No sudo: installs to ~/.local/bin by default.
#   NEMO_VERSION=0.1.0        version to install (default: latest release)
#   NEMO_REPO=owner/repo      GitHub repository. The default is stamped in by release.yml from packaging/REPO;
#                             an unstamped copy (or a value containing OWNER) refuses to run.
#   NEMO_INSTALL_DIR=DIR      default $HOME/.local/bin
#   NEMO_BIN_NAME=nemo        name of the installed command (e.g. nemo-rs to coexist with the Node CLI)
#   (default)                 requires `gh` and a passing `gh attestation verify` of SHA256SUMS (signer workflow + source ref v$version); no gh = refuse
#   NEMO_SKIP_ATTESTATION=1   accept checksum-only (SHA256SUMS itself unauthenticated; a loud warning is printed)
#   NEMO_DOWNLOAD_BASE=URL    override the download base URL (mirrors, tests); must be https://
#   NEMO_INSECURE_DOWNLOAD_BASE_I_KNOW=1  allow a non-https NEMO_DOWNLOAD_BASE (tests only)
set -eu

REPO=${NEMO_REPO:-@NEMO_REPO@}
DIR=${NEMO_INSTALL_DIR:-$HOME/.local/bin}
NAME=${NEMO_BIN_NAME:-nemo}

die() { echo "install.sh: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"; }
need curl; need tar; need uname

case $REPO in
  *OWNER*) die "repository '$REPO' is the unreplaced placeholder; set NEMO_REPO=owner/repo" ;;
  */*/*|*[!A-Za-z0-9_./-]*|/*|*/|'') die "bad repository '$REPO' (unstamped installer? set NEMO_REPO=owner/repo)" ;;
  */*) ;;
  *) die "bad repository '$REPO'" ;;
esac

case "$(uname -s)" in
  Darwin) os=apple-darwin ;;
  Linux) os=unknown-linux-gnu ;;
  *) die "unsupported OS $(uname -s); on Windows use install.ps1" ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  *) die "unsupported CPU $(uname -m)" ;;
esac
# Alpine and other musl systems: only x86_64 is built.
if [ "$os" = unknown-linux-gnu ] && command -v ldd >/dev/null 2>&1 && ldd --version 2>&1 | grep -qi musl; then
  [ "$arch" = x86_64 ] || die "no musl build for $arch"
  os=unknown-linux-musl
fi
target="$arch-$os"

proto='=https'
base=${NEMO_DOWNLOAD_BASE:-}
case $base in
  ''|https://*) ;;
  *) [ "${NEMO_INSECURE_DOWNLOAD_BASE_I_KNOW:-}" = 1 ] || die "NEMO_DOWNLOAD_BASE must be https:// (got '$base')"
     proto='=https,http,file'; echo "warning: insecure download base" >&2 ;;
esac
dl() { curl --proto "$proto" --proto-redir '=https' --tlsv1.2 --fail --location --silent --show-error "$@"; }

version=${NEMO_VERSION:-}
if [ -z "$version" ]; then
  url=$(dl -I -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") || die "cannot resolve latest release"
  version=${url##*/v}
fi
version=${version#v}
case $version in ''|*[!0-9A-Za-z.+-]*) die "bad version '$version' (allowed: 0-9 A-Z a-z . + -)" ;; esac
base=${base:-https://github.com/$REPO/releases/download/v$version}
archive="nemo-$version-$target.tar.gz"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
echo "Downloading $archive"
dl -o "$tmp/$archive" "$base/$archive" || die "download failed: $base/$archive"
dl -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" || die "download failed: $base/SHA256SUMS"

want=$(grep -F "  $archive" "$tmp/SHA256SUMS" | awk '{print $1}')
[ -n "$want" ] || die "$archive not listed in SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum "$tmp/$archive" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then got=$(shasum -a 256 "$tmp/$archive" | awk '{print $1}')
else die "need sha256sum or shasum"; fi
[ "$want" = "$got" ] || die "checksum mismatch for $archive (expected $want, got $got)"
echo "sha256 ok"

# SHA256SUMS is only as trustworthy as its source: verify its build-provenance attestation (made by release.yml).
if [ "${NEMO_SKIP_ATTESTATION:-}" = 1 ]; then
  echo "WARNING: NEMO_SKIP_ATTESTATION=1: SHA256SUMS was NOT authenticated; only the archive hash against it was checked. Anyone who can alter the download can alter both." >&2
elif command -v gh >/dev/null 2>&1; then
  gh attestation verify "$tmp/SHA256SUMS" --repo "$REPO" --signer-workflow "$REPO/.github/workflows/release.yml" \
    --source-ref "refs/tags/v$version" \
    || die "attestation verification of SHA256SUMS failed (not logged in to gh? set NEMO_SKIP_ATTESTATION=1 to accept checksum-only)"
  echo "attestation ok"
else
  die "gh (GitHub CLI) not found, so SHA256SUMS cannot be authenticated. Install gh (https://cli.github.com) and run 'gh auth login', or set NEMO_SKIP_ATTESTATION=1 to accept a checksum-only install"
fi

tar -xzf "$tmp/$archive" -C "$tmp"
src="$tmp/nemo-$version-$target"
[ -f "$src/nemo" ] || die "archive layout unexpected (no nemo)"
mkdir -p "$DIR"
install -m 0755 "$src/nemo" "$DIR/$NAME"
# Landlock backend looks for this helper next to the executable.
[ -f "$src/nemo-sandbox-exec" ] && install -m 0755 "$src/nemo-sandbox-exec" "$DIR/nemo-sandbox-exec"
echo "Installed $DIR/$NAME ($version, $target)"

case ":$PATH:" in *":$DIR:"*) ;; *) echo "Add $DIR to PATH." ;; esac
other=$(command -v "$NAME" 2>/dev/null || true)
if [ -n "$other" ] && [ "$other" != "$DIR/$NAME" ]; then
  echo "warning: '$NAME' on PATH currently resolves to $other (Node CLI?). Put $DIR first or use NEMO_BIN_NAME." >&2
fi
