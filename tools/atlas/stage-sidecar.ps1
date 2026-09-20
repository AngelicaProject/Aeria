[CmdletBinding()]
param(
    [ValidateSet("win-x64", "linux-x64")]
    [string]$Target = $(if ($IsWindows) { "win-x64" } else { "linux-x64" })
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$manifest = Get-Content -Raw (Join-Path $PSScriptRoot "version.json") | ConvertFrom-Json
$artifact = $manifest.artifacts.PSObject.Properties[$Target].Value
$downloadUrl = "$($manifest.releaseBaseUrl)/$($artifact.name)"
$downloadRoot = Join-Path ([System.IO.Path]::GetTempPath()) "aeria-atlas-$($manifest.version)-$Target"
$archivePath = Join-Path $downloadRoot $artifact.name
$extractRoot = Join-Path $downloadRoot "extract"
$destinationRoot = Join-Path $repoRoot "apps/desktop/src-tauri/binaries"
$suffix = if ($Target -eq "win-x64") { ".exe" } else { "" }
$destination = Join-Path $destinationRoot "harmonia-atlas-$($artifact.targetTriple)$suffix"

New-Item -ItemType Directory -Force -Path $downloadRoot, $extractRoot, $destinationRoot | Out-Null
Invoke-WebRequest -Uri $downloadUrl -OutFile $archivePath
$actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash.ToLowerInvariant()
if ($actualHash -ne $artifact.sha256) {
    throw "Atlas artifact hash mismatch. Expected $($artifact.sha256), got $actualHash."
}

if ($artifact.archive -eq "zip") {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        $entry = $archive.GetEntry($artifact.executable)
        if ($null -eq $entry) { throw "Expected executable $($artifact.executable) was not found in the archive." }
        $input = $entry.Open()
        $output = [System.IO.File]::Create($destination)
        try { $input.CopyTo($output) } finally { $output.Dispose(); $input.Dispose() }
    } finally {
        $archive.Dispose()
    }
} else {
    tar -xzf $archivePath -C $extractRoot -- $($artifact.executable)
    if ($LASTEXITCODE -ne 0) { throw "Could not extract the expected Atlas executable." }
    Copy-Item -LiteralPath (Join-Path $extractRoot $artifact.executable) -Destination $destination -Force
}

if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) {
    throw "Atlas sidecar was not staged at $destination."
}
if ($Target -eq "linux-x64") {
    & chmod +x $destination
}
Write-Output "Staged Harmonia Atlas $($manifest.version) at $destination"
