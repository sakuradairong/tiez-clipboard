param(
    [Parameter(Mandatory = $true)]
    [string]$SetupPath
)

$ErrorActionPreference = "Stop"

# Builds the TieZ install wizard on Windows and embeds an already-built NSIS setup.
# This script does not download setup.exe and does not pass /P, /UPDATE, /R, or /NS.
# The wizard binary itself calls the embedded setup with /S, plus a final unquoted /D= only when the user changes the path.

if (-not (Test-Path -LiteralPath $SetupPath)) {
    throw "Inner NSIS setup was not found: $SetupPath"
}

$resolved = (Resolve-Path -LiteralPath $SetupPath).Path
$bytes = [System.IO.File]::ReadAllBytes($resolved)
if ($bytes.Length -lt 1024 -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
    throw "Refusing to embed $resolved because it is not a Windows executable."
}

$wizardRoot = Split-Path -Parent $PSScriptRoot
$repoRoot = Split-Path -Parent $wizardRoot
Push-Location $wizardRoot
try {
    $env:TIEZ_SETUP_EXE = $resolved
    if (-not (Test-Path -LiteralPath "node_modules")) {
        npm ci
    }
    npm test
    cargo test --manifest-path Cargo.toml -p tiez-installer-core
    npm run tauri:build

    $built = @(
        (Join-Path $wizardRoot "target\release\tiez-installer.exe"),
        (Join-Path $wizardRoot "src-tauri\target\release\tiez-installer.exe")
    ) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $built) {
        throw "tiez-installer.exe was not produced. On a non-Windows machine this script cannot create the wizard executable."
    }
    $builtItem = Get-Item -LiteralPath $built
    $setupItem = Get-Item -LiteralPath $resolved
    if ($builtItem.Length -le $setupItem.Length) {
        throw "Wizard executable is not larger than the inner setup, so the embed did not land in the binary."
    }

    $version = (Get-Content -Raw (Join-Path $repoRoot "package.json") | ConvertFrom-Json).version
    $outDir = Join-Path $wizardRoot "dist-bootstrapper"
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    $out = Join-Path $outDir "TieZ-$version-windows-x64-installer.exe"
    Copy-Item -LiteralPath $builtItem.FullName -Destination $out -Force
    Write-Host "Wrote $out"
}
finally {
    Pop-Location
}
