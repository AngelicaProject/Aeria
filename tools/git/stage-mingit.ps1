# Stages the pinned MinGit distribution that the Windows desktop bundle ships
# as its Git runtime. MinGit includes Git Credential Manager and configures
# `credential.helper=manager`.
[CmdletBinding()]
param(
    [ValidateSet("win-x64")]
    [string]$Target = "win-x64"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$manifest = Get-Content -Raw (Join-Path $PSScriptRoot "version.json") | ConvertFrom-Json
$artifact = $manifest.artifacts.PSObject.Properties[$Target].Value
$downloadRoot = Join-Path ([System.IO.Path]::GetTempPath()) "aeria-mingit-$($manifest.version)-$Target"
$archivePath = Join-Path $downloadRoot "mingit.zip"
$destination = Join-Path $repoRoot "apps/desktop/src-tauri/binaries/mingit"
$stamp = Join-Path $destination ".aeria-mingit-version"

if ((Test-Path -LiteralPath $stamp) -and ((Get-Content -Raw -LiteralPath $stamp).Trim() -eq $artifact.sha256)) {
    Write-Output "MinGit $($manifest.version) is already staged at $destination"
    exit 0
}

New-Item -ItemType Directory -Force -Path $downloadRoot | Out-Null
if (-not (Test-Path -LiteralPath $archivePath) -or
    (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash.ToLowerInvariant() -ne $artifact.sha256) {
    Invoke-WebRequest -Uri $artifact.url -OutFile $archivePath
}
$actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash.ToLowerInvariant()
if ($actualHash -ne $artifact.sha256) {
    throw "MinGit artifact hash mismatch. Expected $($artifact.sha256), got $actualHash."
}

if (Test-Path -LiteralPath $destination) {
    Remove-Item -LiteralPath $destination -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $destination | Out-Null
Expand-Archive -LiteralPath $archivePath -DestinationPath $destination

$git = Join-Path $destination "cmd/git.exe"
if (-not (Test-Path -LiteralPath $git -PathType Leaf)) {
    throw "MinGit was not staged: $git is missing."
}
$version = & $git --version
if ($LASTEXITCODE -ne 0) { throw "Staged MinGit does not run." }
$helper = & $git config --system --get credential.helper
if ($helper -ne "manager") { throw "Staged MinGit does not configure Git Credential Manager (got '$helper')." }

Set-Content -LiteralPath $stamp -Value $artifact.sha256 -NoNewline
Write-Output "Staged $version at $destination"
