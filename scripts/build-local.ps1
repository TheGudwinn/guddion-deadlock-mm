# Local signed build: reads the updater key and password from %USERPROFILE%\.deadlock-mm.
# Usage: .\scripts\build-local.ps1 [-Bundles nsis,msi]
param([string]$Bundles = "nsis")

$keyDir = Join-Path $env:USERPROFILE ".deadlock-mm"
$env:TAURI_SIGNING_PRIVATE_KEY = Join-Path $keyDir "updater.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = (Get-Content (Join-Path $keyDir "updater.password") -Raw).Trim()

Set-Location (Split-Path $PSScriptRoot -Parent)
pnpm tauri build --bundles $Bundles
