param([Parameter(Mandatory=$true)][string]$PackageDirectory)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$fixture = Join-Path $env:TEMP ('hfy-installer-test-' + [guid]::NewGuid().ToString('N'))
$assets = Join-Path $fixture 'assets'
$destination = Join-Path $fixture 'install with spaces'
$previous = @{}
foreach ($key in @('HFY_INSTALL_DIR','HFY_NO_PATH','HFY_RELEASE_REPO','HFY_TAG_PREFIX','HFY_VERSION')) { $previous[$key] = [Environment]::GetEnvironmentVariable($key) }
function Invoke-WebRequest {
    param([string]$Uri, [string]$OutFile, [switch]$UseBasicParsing, [int]$TimeoutSec)
    if ($Uri -notlike 'https://github.com/Fixture/hireferry/releases/*') { throw "Unexpected download: $Uri" }
    Copy-Item -LiteralPath (Join-Path $assets (($Uri -split '/')[-1])) -Destination $OutFile
}
try {
    New-Item -ItemType Directory -Path $assets -Force | Out-Null
    Copy-Item (Join-Path $PackageDirectory '*') $assets
    $manifest = Get-Content -LiteralPath (Join-Path $root 'crates/hireferry-cli/Cargo.toml') -Raw
    $version = [regex]::Match($manifest, '(?m)^version = "([^"]+)"$').Groups[1].Value
    Set-Content -LiteralPath (Join-Path $assets 'VERSION') -Value $version -Encoding ascii
    $env:HFY_INSTALL_DIR = $destination
    $env:HFY_NO_PATH = '1'
    $env:HFY_RELEASE_REPO = 'Fixture/hireferry'
    $env:HFY_TAG_PREFIX = 'cli-v'
    Remove-Item Env:HFY_VERSION -ErrorAction SilentlyContinue
    & (Join-Path $root 'static/install.ps1')
    $binary = Join-Path $destination 'hfy.exe'
    $output = & $binary --version
    if ($LASTEXITCODE -ne 0 -or $output -notlike "*$version*") { throw 'Installed client did not run.' }
    $before = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
    $archive = Get-ChildItem -LiteralPath $assets -Filter '*.zip'
    Set-Content -LiteralPath (Join-Path $assets 'SHA256SUMS') -Value (('0'*64) + '  ' + $archive.Name) -Encoding ascii
    $failed = $false
    try { & (Join-Path $root 'static/install.ps1') } catch { $failed = $true }
    if (-not $failed -or (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash -ne $before) { throw 'Checksum failure did not preserve the installed client.' }
    Write-Host 'Windows installer: native install, execution and checksum rejection passed.'
} finally {
    foreach ($key in $previous.Keys) { [Environment]::SetEnvironmentVariable($key,$previous[$key]) }
    Remove-Item -LiteralPath $fixture -Recurse -Force -ErrorAction SilentlyContinue
}
