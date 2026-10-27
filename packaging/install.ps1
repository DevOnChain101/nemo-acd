# Nemo installer for Windows (PowerShell 7+; Windows PowerShell 5.1 is not supported). No admin rights: installs to %LOCALAPPDATA%\Programs\Nemo.
#   $env:NEMO_VERSION, $env:NEMO_INSTALL_DIR, $env:NEMO_BIN_NAME (default nemo),
#   $env:NEMO_REPO (owner/repo; default stamped in by release.yml from packaging/REPO, an unstamped copy refuses to run),
#   (default) requires gh + a passing `gh attestation verify` of SHA256SUMS (signer workflow + source ref v$version); no gh = refuse,
#   $env:NEMO_SKIP_ATTESTATION=1 (accept checksum-only; prints a loud warning),
#   $env:NEMO_DOWNLOAD_BASE (mirrors, tests; must be https:// unless $env:NEMO_INSECURE_DOWNLOAD_BASE_I_KNOW=1)
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw "PowerShell 7+ is required (this is $($PSVersionTable.PSVersion)). Install it with: winget install Microsoft.PowerShell, then run this script with pwsh" }
$repo = if ($env:NEMO_REPO) { $env:NEMO_REPO } else { '@NEMO_REPO@' }
if ($repo -cmatch 'OWNER') { throw "repository '$repo' is the unreplaced placeholder; set `$env:NEMO_REPO=owner/repo" }
if ($repo -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') { throw "bad repository '$repo' (unstamped installer? set `$env:NEMO_REPO=owner/repo)" }
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
if ($env:NEMO_DOWNLOAD_BASE -and $env:NEMO_DOWNLOAD_BASE -notmatch '^https://' -and $env:NEMO_INSECURE_DOWNLOAD_BASE_I_KNOW -ne '1') {
  throw "NEMO_DOWNLOAD_BASE must be https:// (got '$($env:NEMO_DOWNLOAD_BASE)')"
}
$dir = if ($env:NEMO_INSTALL_DIR) { $env:NEMO_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\Nemo' }
$name = if ($env:NEMO_BIN_NAME) { $env:NEMO_BIN_NAME } else { 'nemo' }

$arch = switch ($env:PROCESSOR_ARCHITECTURE) { 'AMD64' { 'x86_64' } 'ARM64' { 'aarch64' } default { throw "unsupported CPU $($env:PROCESSOR_ARCHITECTURE)" } }
$target = "$arch-pc-windows-msvc"

$version = $env:NEMO_VERSION
if (-not $version) {
  $r = Invoke-WebRequest -Uri "https://github.com/$repo/releases/latest" -MaximumRedirection 0 -SkipHttpErrorCheck -ErrorAction SilentlyContinue
  $version = ($r.Headers.Location -split '/v')[-1]
  if (-not $version) { throw 'cannot resolve latest release' }
}
$version = $version.TrimStart('v')
if ($version -notmatch '^[0-9A-Za-z.+-]+$') { throw "bad version '$version' (allowed: 0-9 A-Z a-z . + -)" }
$base = if ($env:NEMO_DOWNLOAD_BASE) { $env:NEMO_DOWNLOAD_BASE } else { "https://github.com/$repo/releases/download/v$version" }
$archive = "nemo-$version-$target.zip"

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("nemo-" + [guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
  Write-Host "Downloading $archive"
  Invoke-WebRequest -Uri "$base/$archive" -OutFile (Join-Path $tmp $archive)
  Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile (Join-Path $tmp 'SHA256SUMS')
  $line = Get-Content (Join-Path $tmp 'SHA256SUMS') | Where-Object { $_ -match [regex]::Escape("  $archive") + '$' } | Select-Object -First 1
  if (-not $line) { throw "$archive not listed in SHA256SUMS" }
  $want = ($line -split '\s+')[0].ToLower()
  $got = (Get-FileHash (Join-Path $tmp $archive) -Algorithm SHA256).Hash.ToLower()
  if ($want -ne $got) { throw "checksum mismatch for $archive (expected $want, got $got)" }
  Write-Host 'sha256 ok'
  # SHA256SUMS is only as trustworthy as its source: verify its build-provenance attestation (made by release.yml).
  if ($env:NEMO_SKIP_ATTESTATION -eq '1') {
    Write-Warning 'NEMO_SKIP_ATTESTATION=1: SHA256SUMS was NOT authenticated; only the archive hash against it was checked. Anyone who can alter the download can alter both.'
  } elseif (Get-Command gh -ErrorAction SilentlyContinue) {
    gh attestation verify (Join-Path $tmp 'SHA256SUMS') --repo $repo --signer-workflow "$repo/.github/workflows/release.yml" --source-ref "refs/tags/v$version"
    if ($LASTEXITCODE -ne 0) { throw 'attestation verification of SHA256SUMS failed (not logged in to gh? set NEMO_SKIP_ATTESTATION=1 to accept checksum-only)' }
    Write-Host 'attestation ok'
  } else {
    throw "gh (GitHub CLI) not found, so SHA256SUMS cannot be authenticated. Install gh (https://cli.github.com) and run 'gh auth login', or set `$env:NEMO_SKIP_ATTESTATION=1 to accept a checksum-only install"
  }
  Expand-Archive (Join-Path $tmp $archive) -DestinationPath $tmp
  $src = Join-Path $tmp "nemo-$version-$target"
  New-Item -ItemType Directory -Force $dir | Out-Null
  Copy-Item (Join-Path $src 'nemo.exe') (Join-Path $dir "$name.exe") -Force
  Write-Host "Installed $(Join-Path $dir "$name.exe") ($version, $target)"
  if (($env:PATH -split ';') -notcontains $dir) { Write-Host "Add $dir to PATH (user environment variables)." }
  $other = Get-Command $name -ErrorAction SilentlyContinue
  if ($other -and $other.Source -ne (Join-Path $dir "$name.exe")) { Write-Warning "'$name' on PATH resolves to $($other.Source) (Node CLI?). Put $dir first or set NEMO_BIN_NAME." }
} finally { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
