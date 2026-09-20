[CmdletBinding()]
param(
    [ValidateSet("win-x64", "linux-x64")]
    [string]$Target = $(if ($IsWindows) { "win-x64" } else { "linux-x64" })
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$manifest = Get-Content -Raw (Join-Path $PSScriptRoot "version.json") | ConvertFrom-Json
$artifact = $manifest.artifacts.PSObject.Properties[$Target].Value
$suffix = if ($Target -eq "win-x64") { ".exe" } else { "" }
$binary = Join-Path $repoRoot "apps/desktop/src-tauri/binaries/harmonia-atlas-$($artifact.targetTriple)$suffix"

if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw "Expected staged Atlas executable was not found at $binary."
}

$versionOutput = (& $binary --version 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $versionOutput -ne $manifest.version) {
    throw "Atlas --version returned '$versionOutput', expected '$($manifest.version)'."
}

$helpOutput = (& $binary --help 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw "Atlas --help failed: $helpOutput"
}
if ($helpOutput -notmatch '(?m)package\s+--game-path.*--language.*--output.*--events\s+jsonl') {
    throw "Atlas help does not expose the required package JSONL contract."
}

$packageHelpOutput = (& $binary package --help 2>&1 | Out-String)
if ($packageHelpOutput -notmatch '--events\s+jsonl') {
    throw "Atlas package help/usage does not expose --events jsonl."
}

$probeRoot = Join-Path ([System.IO.Path]::GetTempPath()) "aeria-atlas-smoke-$PID"
New-Item -ItemType Directory -Force -Path $probeRoot | Out-Null
try {
    $probeOutput = (& $binary package `
        --game-path (Join-Path $probeRoot "empty-game") `
        --language en `
        --output (Join-Path $probeRoot "source.hsp") `
        --events jsonl 2>&1 | Out-String)
    if ($probeOutput -match '(?i)unknown (command|option)|unrecognized option|unexpected argument') {
        throw "Atlas rejected the required package JSONL arguments: $probeOutput"
    }
} finally {
    Remove-Item -LiteralPath $probeRoot -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Output "Verified Harmonia Atlas $($manifest.version) package JSONL contract at $binary"
