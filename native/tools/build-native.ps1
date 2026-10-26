param(
    [ValidateSet('Debug', 'Release')][string]$Configuration = 'Release',
    [ValidateSet('ordinary', 'portable')][string]$Flavor = 'ordinary',
    [switch]$FrameworkDependent
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'Use PowerShell 7 on Windows x64 with the repository Rust and .NET toolchains.'
}
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$buildRoot = Join-Path $root 'native/build/target'
$cargoTarget = Join-Path $buildRoot "cargo/$Flavor"
$rustTarget = 'x86_64-pc-windows-msvc'
$libraryTarget = Join-Path $cargoTarget $rustTarget
$profile = $Configuration -eq 'Release' ? 'release' : 'debug'
$publishRoot = Join-Path $buildRoot "publish/$Flavor/$Configuration/$([guid]::NewGuid().ToString('N'))"
$artifactsRoot = Join-Path $buildRoot "managed/$Flavor/$Configuration"
function Get-NativeSourceReceipt {
    $paths = @(git -c core.quotePath=false ls-files --cached --others --exclude-standard -- native src-tauri rust-toolchain.toml)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot enumerate native source inputs.' }
    $inputs = @($paths | Sort-Object -Unique | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | ForEach-Object {
        [ordered]@{ path = $_; sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_).Hash }
    })
    $bytes = [Text.Encoding]::UTF8.GetBytes(($inputs | ConvertTo-Json -Compress))
    [pscustomobject]@{ sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)); files = $inputs }
}
Push-Location $root
try {
    $commit = (git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot determine source commit.' }
    $dirty = -not [string]::IsNullOrWhiteSpace((git status --porcelain | Out-String))
    $source = Get-NativeSourceReceipt
    # Each flavor has its own Cargo feature/build cache. Never copy a previously staged DLL.
    $cargoArgs = @('build', '--locked', '--manifest-path', 'native/core/Cargo.toml', '--target-dir', $cargoTarget, '--target', $rustTarget)
    if ($Configuration -eq 'Release') { $cargoArgs += '--release' }
    if ($Flavor -eq 'portable') { $cargoArgs += @('--features', 'portable') }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw 'Native Rust core build failed.' }
    $nativeDll = Join-Path $libraryTarget "$profile/tiez_native_core.dll"
    if (-not (Test-Path -LiteralPath $nativeDll -PathType Leaf)) { throw 'Cargo produced no expected x64 native DLL.' }
    $nativeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $nativeDll).Hash
    if (-not ('NativeBuildAbiProbe' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class NativeBuildAbiProbe {
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)] private delegate uint Version();
    public static uint Read(string path) {
        IntPtr library = NativeLibrary.Load(path);
        try { return Marshal.GetDelegateForFunctionPointer<Version>(NativeLibrary.GetExport(library, "tiez_core_abi_version"))(); }
        finally { NativeLibrary.Free(library); }
    }
}
'@
    }
    $abi = [NativeBuildAbiProbe]::Read($nativeDll)
    if ($abi -ne 1) { throw "Unsupported native ABI: $abi" }

    $selfContained = $FrameworkDependent ? 'false' : 'true'
    $publishArgs = @('publish', 'native/managed/TieZ.WinUI/TieZ.WinUI.csproj', '-c', $Configuration,
        '-r', 'win-x64', '--self-contained', $selfContained, '--artifacts-path', $artifactsRoot,
        '-o', $publishRoot, "-p:NativeCoreTargetDir=$libraryTarget", "-p:WindowsAppSDKSelfContained=$selfContained",
        '-p:PublishSingleFile=false', '-p:PublishTrimmed=false')
    & dotnet @publishArgs
    if ($LASTEXITCODE -ne 0) { throw 'Native WinUI publish failed.' }
    $audit = & (Join-Path $PSScriptRoot 'verify-native-publish.ps1') -Directory $publishRoot -ExpectedNativeSha256 $nativeHash -SelfContained (-not $FrameworkDependent)
    $files = @(Get-ChildItem -LiteralPath $publishRoot -Recurse -File)
    $manifestFiles = @($files | ForEach-Object {
        [ordered]@{ path = [IO.Path]::GetRelativePath($publishRoot, $_.FullName).Replace('\', '/');
            bytes = $_.Length; sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash }
    } | Sort-Object { $_.path })
    if ((Get-NativeSourceReceipt).sha256 -ne $source.sha256 -or (git rev-parse HEAD).Trim() -ne $commit) {
        throw 'Source inputs changed during the build; discard this unverified output and rebuild.'
    }
    $manifest = [ordered]@{
        format = 1; sourceCommit = $commit; sourceDirty = $dirty; sourceInputs = $source; configuration = $Configuration; flavor = $Flavor;
        runtimeIdentifier = 'win-x64'; rustTarget = $rustTarget; dotnetSelfContained = (-not $FrameworkDependent);
        windowsAppRuntimeSelfContained = (-not $FrameworkDependent); nativeSha256 = $nativeHash; nativeAbiVersion = $abi;
        rust = (& cargo --version | Out-String).Trim(); dotnet = (& dotnet --version | Out-String).Trim();
        browserAssetCheckPassed = $true; files = $manifestFiles;
        acceptance = 'Build/asset check only; clean-OS install, full feature, physical input and performance acceptance remain pending.'
    }
    [IO.File]::WriteAllText((Join-Path $publishRoot 'native-build.json'), ($manifest | ConvertTo-Json -Depth 6))
    "PASS native publish: $publishRoot"
    "Flavor=$Flavor Configuration=$Configuration SelfContained=$(-not $FrameworkDependent) nativeSHA256=$nativeHash files=$($files.Count)"
} finally { Pop-Location }
