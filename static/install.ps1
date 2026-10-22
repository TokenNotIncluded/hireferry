# Native Windows installation. Run in PowerShell; no administrator access needed.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0
$repository = if ($env:HFY_RELEASE_REPO) { $env:HFY_RELEASE_REPO } else { '{{REPOSITORY}}' }
$tagPrefix = if ($env:HFY_TAG_PREFIX) { $env:HFY_TAG_PREFIX } else { '{{TAG_PREFIX}}' }
$version = $env:HFY_VERSION
$installDir = if ($env:HFY_INSTALL_DIR) { $env:HFY_INSTALL_DIR } else { Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'HireFerry\bin' }
if ($env:OS -ne 'Windows_NT') { throw 'Use install.sh on Linux or macOS.' }
if ($repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' -or $tagPrefix -notmatch '^[A-Za-z0-9_.-]+$') { throw 'Invalid release repository or tag prefix.' }
$machine = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
switch ($machine.ToUpperInvariant()) {
    'AMD64' { $target = 'x86_64-pc-windows-msvc' }
    'ARM64' { $target = 'aarch64-pc-windows-msvc' }
    default { throw "Unsupported Windows architecture: $machine" }
}
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$work = Join-Path ([IO.Path]::GetTempPath()) ('hfy-install-' + [guid]::NewGuid().ToString('N'))
$staged = $null
New-Item -ItemType Directory -Path $work | Out-Null
function Download([string]$Url, [string]$Destination) {
    Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Destination -TimeoutSec 180
}
try {
    $base = "https://github.com/$repository/releases"
    if (-not $version) {
        Download "$base/latest/download/VERSION" (Join-Path $work 'VERSION')
        $version = (Get-Content -LiteralPath (Join-Path $work 'VERSION') -Raw).Trim()
    }
    if ($version -notmatch '^\d+\.\d+\.\d+([.-][A-Za-z0-9.-]+)?$') { throw 'Invalid release version.' }
    $asset = "hireferry-cli-$version-$target.zip"
    $url = "$base/download/$tagPrefix$version"
    Write-Host "Downloading hfy $version for $target..."
    $archive = Join-Path $work $asset
    Download "$url/$asset" $archive
    Download "$url/SHA256SUMS" (Join-Path $work 'SHA256SUMS')
    $matchesForAsset = @(Get-Content -LiteralPath (Join-Path $work 'SHA256SUMS') | Where-Object { $_ -match ('^[a-fA-F0-9]{64}\s+\*?' + [regex]::Escape($asset) + '$') })
    if ($matchesForAsset.Count -ne 1) { throw 'Missing or ambiguous release checksum.' }
    $expected = ($matchesForAsset[0] -split '\s+')[0]
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expected) { throw 'Checksum mismatch; nothing was installed.' }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    try {
        $entryName = "hireferry-cli-$version-$target/hfy.exe"
        $entries = @($zip.Entries | Where-Object { $_.FullName -eq $entryName })
        if ($entries.Count -ne 1 -or $entries[0].Length -eq 0) { throw 'The release does not contain exactly one hfy.exe.' }
        $inputStream = $entries[0].Open()
        $outputStream = [IO.File]::Create((Join-Path $work 'hfy.exe'))
        try { $inputStream.CopyTo($outputStream) } finally { $outputStream.Dispose(); $inputStream.Dispose() }
    } finally { $zip.Dispose() }
    $installDir = [IO.Path]::GetFullPath($installDir)
    $part = $installDir
    while ($part) {
        if (Test-Path -LiteralPath $part) {
            if ((Get-Item -LiteralPath $part -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Install path contains a reparse point: $part" }
        }
        $parent = [IO.Path]::GetDirectoryName($part)
        if ($parent -eq $part) { break }
        $part = $parent
    }
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    $destination = Join-Path $installDir 'hfy.exe'
    if (Test-Path -LiteralPath $destination) {
        $existing = Get-Item -LiteralPath $destination -Force
        if ($existing.PSIsContainer -or ($existing.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'The destination hfy.exe is not a regular file.' }
    }
    $staged = Join-Path $installDir ('.hfy-install-' + [guid]::NewGuid().ToString('N') + '.exe')
    Copy-Item -LiteralPath (Join-Path $work 'hfy.exe') -Destination $staged
    & $staged --version
    if ($LASTEXITCODE -ne 0) { throw 'The downloaded CLI failed its version check.' }
    Move-Item -LiteralPath $staged -Destination $destination -Force
    $staged = $null
    # Modify only the current user's PATH; tests and custom setups can opt out.
    if ($env:HFY_NO_PATH -ne '1') {
        $userPath = [string][Environment]::GetEnvironmentVariable('Path', 'User')
        if (-not (@($userPath -split ';') -contains $installDir)) {
            [Environment]::SetEnvironmentVariable('Path', (($userPath.TrimEnd(';') + ';' + $installDir).TrimStart(';')), 'User')
        }
        if (-not (@($env:Path -split ';') -contains $installDir)) { $env:Path = $installDir + ';' + $env:Path }
    }
    Write-Host "Installed: $destination"
    Write-Host 'Open a new terminal if hfy is not on PATH.'
    Write-Host 'Sign in: hfy --server "{{SERVER}}" auth login'
    Write-Host 'Remote terminal: hfy --server "{{SERVER}}" auth login --device'
} finally {
    if ($staged -and (Test-Path -LiteralPath $staged)) { Remove-Item -LiteralPath $staged -Force }
    Remove-Item -LiteralPath $work -Recurse -Force
}
