$ErrorActionPreference = 'Stop'
$probeRoot = Join-Path $PSScriptRoot '../artifacts/cc-switch-followup'
$probeRoot = [System.IO.Path]::GetFullPath($probeRoot)
New-Item -ItemType Directory -Force -Path $probeRoot | Out-Null
$probeArchive = Join-Path $probeRoot 'cli.zip'
$probeDigest = '2493764062063f2b0e5a3a1eda009f4e62451a0c113dc3f294beb10267deb6ff'
function Get-ProbeDigest([string]$path) {
    $probeStream = [System.IO.File]::OpenRead($path)
    $probeHash = [System.Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($probeHash.ComputeHash($probeStream))).Replace('-', '').ToLowerInvariant() }
    finally { $probeStream.Dispose(); $probeHash.Dispose() }
}
# GitHub's release API avoids relying on a latest URL or changing PATH/global installs.
if (!(Test-Path -LiteralPath $probeArchive) -or (Get-ProbeDigest $probeArchive) -ne $probeDigest) {
    & curl.exe --fail --location --silent --show-error --continue-at - --connect-timeout 10 --max-time 300 -H 'Accept: application/octet-stream' 'https://api.github.com/repos/SaladDay/cc-switch-cli/releases/assets/618944616' --output $probeArchive
    if ($LASTEXITCODE -ne 0) { throw 'CC_SWITCH_CLI_DOWNLOAD_FAILED (rerun to resume)' }
}
if ((Get-ProbeDigest $probeArchive) -ne $probeDigest) {
    throw 'CC_SWITCH_CLI_ARCHIVE_CHECKSUM_MISMATCH'
}
Add-Type -AssemblyName System.IO.Compression.FileSystem
$probeDestination = Join-Path $probeRoot 'cli-bin'
New-Item -ItemType Directory -Force -Path $probeDestination | Out-Null
$probeZip = [System.IO.Compression.ZipFile]::OpenRead($probeArchive)
try {
    foreach ($probeEntry in $probeZip.Entries) {
        $probeTarget = [System.IO.Path]::GetFullPath((Join-Path $probeDestination $probeEntry.FullName))
        if (!$probeTarget.StartsWith($probeDestination + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) { throw 'CC_SWITCH_CLI_ZIP_PATH_INVALID' }
        if ($probeEntry.Name -eq '') { New-Item -ItemType Directory -Force -Path $probeTarget | Out-Null; continue }
        New-Item -ItemType Directory -Force -Path ([System.IO.Path]::GetDirectoryName($probeTarget)) | Out-Null
        [System.IO.Compression.ZipFileExtensions]::ExtractToFile($probeEntry, $probeTarget, $true)
    }
} finally { $probeZip.Dispose() }
Write-Output 'CC Switch CLI v5.11.0 portable archive verified and extracted; no global installation.'
