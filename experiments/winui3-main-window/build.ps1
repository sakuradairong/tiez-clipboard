[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release",

    [ValidateSet("x64")]
    [string]$Platform = "x64",

    [switch]$SkipRustTests,
    [switch]$SkipWinUIBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Packages = Join-Path $Root "packages"
$Artifacts = Join-Path $Root "artifacts\$Platform\$Configuration"
$RustManifest = Join-Path $Root "rust-core\Cargo.toml"
$Solution = Join-Path $Root "Tiez.WinUIProbe.sln"
$Nuget = Join-Path $Root ".tools\nuget.exe"

function Resolve-MSBuild {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $resolved = & $vswhere -latest -requires Microsoft.Component.MSBuild -find "MSBuild\**\Bin\MSBuild.exe" | Select-Object -First 1
        if ($resolved) {
            return $resolved
        }
    }

    $command = Get-Command msbuild.exe -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    throw "MSBuild was not found. Install Visual Studio 2022 with Desktop development with C++ and Windows application development workloads."
}

function Ensure-NuGet {
    if (Test-Path $Nuget) {
        return
    }

    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Nuget) | Out-Null
    Invoke-WebRequest https://dist.nuget.org/win-x86-commandline/latest/nuget.exe -OutFile $Nuget
}

if ($env:OS -ne "Windows_NT") {
    throw "This script must run on Windows."
}

$cargo = Get-Command cargo.exe -ErrorAction SilentlyContinue
if (-not $cargo) {
    throw "cargo.exe was not found. Install the stable MSVC Rust toolchain with rustup."
}

$hostTriple = & rustc -vV | Select-String "^host:" | ForEach-Object { $_.Line.Split(":", 2)[1].Trim() }
if ($hostTriple -notmatch "windows-msvc$") {
    throw "Rust must target the Windows MSVC host. Current host: $hostTriple"
}

if (-not $SkipRustTests) {
    & $cargo.Source test --manifest-path $RustManifest
    if ($LASTEXITCODE -ne 0) { throw "Rust tests failed." }
}

& $cargo.Source build --manifest-path $RustManifest --release
if ($LASTEXITCODE -ne 0) { throw "Rust core build failed." }

New-Item -ItemType Directory -Force -Path $Artifacts | Out-Null
Copy-Item (Join-Path $Root "rust-core\target\release\tiez_winui_core.dll") $Artifacts -Force

if (-not $SkipWinUIBuild) {
    Ensure-NuGet
    & $Nuget restore (Join-Path $Root "winui-app\packages.config") -PackagesDirectory $Packages -NonInteractive
    if ($LASTEXITCODE -ne 0) { throw "NuGet restore failed." }

    $msbuild = Resolve-MSBuild
    & $msbuild $Solution /m /restore:false /p:Configuration=$Configuration /p:Platform=$Platform
    if ($LASTEXITCODE -ne 0) { throw "WinUI build failed." }
}

Write-Host ""
Write-Host "Build output: $Artifacts" -ForegroundColor Green
if (-not $SkipWinUIBuild) {
    Write-Host "Run: $(Join-Path $Artifacts 'Tiez.WinUIProbe.exe')" -ForegroundColor Green
}
