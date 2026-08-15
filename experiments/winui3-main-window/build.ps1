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
$RepositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $Root "..\.."))
$Packages = Join-Path $Root "packages"
$Artifacts = Join-Path $Root "artifacts\$Platform\$Configuration"
$WinUIOutput = Join-Path $Root "$Platform\$Configuration\Tiez.WinUIProbe"
$CoreManifest = Join-Path $Root "..\..\crates\tiez-core\Cargo.toml"
$RustManifest = Join-Path $Root "rust-core\Cargo.toml"
$Solution = Join-Path $Root "Tiez.WinUIProbe.sln"
$Nuget = Join-Path $Root ".tools\nuget.exe"
$VersionScript = Join-Path $RepositoryRoot "scripts\verify-release-versions.mjs"

function Resolve-MSBuild {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        # Default vswhere product filter skips Build Tools. This machine (and many
        # CI images) only has VS Build Tools, so search every product first.
        $queries = @(
            @("-latest", "-products", "*", "-requires", "Microsoft.Component.MSBuild", "-find", "MSBuild\**\Bin\MSBuild.exe"),
            @("-latest", "-prerelease", "-products", "*", "-requires", "Microsoft.Component.MSBuild", "-find", "MSBuild\**\Bin\MSBuild.exe")
        )
        foreach ($query in $queries) {
            $resolved = & $vswhere @query | Select-Object -First 1
            if ($resolved) {
                return $resolved
            }
        }
    }

    $command = Get-Command msbuild.exe -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    throw "MSBuild was not found. Install Visual Studio 2022 with Desktop development with C++ and Windows application development workloads, or Visual Studio Build Tools with the MSBuild and C++ workloads."
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

$node = Get-Command node.exe -ErrorAction SilentlyContinue
if (-not $node) {
    throw "node.exe was not found. Install Node.js LTS so release versions can be verified."
}
$Version = (& $node.Source $VersionScript --print).Trim()
if ($LASTEXITCODE -ne 0 -or $Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "TieZ release version validation failed."
}
$VersionParts = $Version.Split('.') | ForEach-Object { [int]$_ }
if ($VersionParts.Count -ne 3 -or ($VersionParts | Where-Object { $_ -gt 65535 })) {
    throw "TieZ version cannot be represented by Windows resources: $Version"
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
    $env:CARGO_TARGET_DIR = Join-Path $Root "rust-core\target"
    & $cargo.Source test --manifest-path $CoreManifest --locked
    if ($LASTEXITCODE -ne 0) { throw "Shared Rust core tests failed." }

    & $cargo.Source test --manifest-path $RustManifest --locked
    if ($LASTEXITCODE -ne 0) { throw "WinUI C ABI tests failed." }
}

$env:CARGO_TARGET_DIR = Join-Path $Root "rust-core\target"
$rustBuildArguments = @("build", "--manifest-path", $RustManifest, "--release", "--locked")
if ($Configuration -eq "Release") {
    $rustBuildArguments += @("--features", "production-default")
}
& $cargo.Source @rustBuildArguments
if ($LASTEXITCODE -ne 0) { throw "Rust core build failed." }

if (-not $SkipWinUIBuild) {
    $rootPrefix = [System.IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
    foreach ($generatedDirectory in @($Artifacts, $WinUIOutput)) {
        $resolvedGeneratedDirectory = [System.IO.Path]::GetFullPath($generatedDirectory)
        if (-not $resolvedGeneratedDirectory.StartsWith(
            $rootPrefix,
            [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to clean a generated directory outside the WinUI experiment: $resolvedGeneratedDirectory"
        }
        if (Test-Path -LiteralPath $resolvedGeneratedDirectory) {
            Remove-Item -LiteralPath $resolvedGeneratedDirectory -Recurse -Force
        }
    }
}
New-Item -ItemType Directory -Force -Path $Artifacts | Out-Null

if (-not $SkipWinUIBuild) {
    Ensure-NuGet
    & $Nuget restore (Join-Path $Root "winui-app\packages.config") -PackagesDirectory $Packages -NonInteractive
    if ($LASTEXITCODE -ne 0) { throw "NuGet restore failed." }

    $msbuild = Resolve-MSBuild
    & $msbuild $Solution /m /restore:false /p:Configuration=$Configuration /p:Platform=$Platform /p:TiezVersionMajor=$($VersionParts[0]) /p:TiezVersionMinor=$($VersionParts[1]) /p:TiezVersionPatch=$($VersionParts[2])
    if ($LASTEXITCODE -ne 0) { throw "WinUI build failed." }

    $winuiExecutable = Join-Path $WinUIOutput "TieZ.exe"
    if (-not (Test-Path $winuiExecutable)) {
        throw "WinUI build output was not found: $winuiExecutable"
    }
    Copy-Item (Join-Path $WinUIOutput "*") $Artifacts -Recurse -Force
}

Copy-Item (Join-Path $Root "rust-core\target\release\tiez_winui_core.dll") $Artifacts -Force

Write-Host ""
Write-Host "Build output: $Artifacts (TieZ $Version)" -ForegroundColor Green
if (-not $SkipWinUIBuild) {
    Write-Host "Run: $(Join-Path $Artifacts 'TieZ.exe')" -ForegroundColor Green
}
