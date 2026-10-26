param(
    [Parameter(Mandatory)][string]$Directory,
    [Parameter(Mandatory)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ExpectedNativeSha256,
    [bool]$SelfContained = $true
)
$ErrorActionPreference = 'Stop'
$directoryPath = (Resolve-Path -LiteralPath $Directory).Path
$copiedDll = Join-Path $directoryPath 'tiez_native_core.dll'
if (-not (Test-Path -LiteralPath $copiedDll -PathType Leaf) -or
    (Get-FileHash -Algorithm SHA256 -LiteralPath $copiedDll).Hash -ne $ExpectedNativeSha256) {
    throw 'Published native DLL does not match the Rust build just completed.'
}
$files = @(Get-ChildItem -LiteralPath $directoryPath -Recurse -File)
$forbidden = @($files | Where-Object { $_.Name -match '(?i)webview2|^wry\.|^tauri\.|^(react|vite)(\.|-)' })
if ($forbidden.Count) { throw "Browser/legacy runtime assets remain: $($forbidden.Name -join ', ')" }
$deps = [IO.File]::ReadAllText((Join-Path $directoryPath 'TieZ.WinUI.deps.json')) | ConvertFrom-Json
$badDependencies = @($deps.libraries.PSObject.Properties.Name | Where-Object { $_ -match '(?i)webview2|tauri|(^|/)wry' })
if ($badDependencies.Count) { throw "Browser/legacy runtime dependencies remain: $($badDependencies -join ', ')" }
$required = @('TieZ.WinUI.exe', 'TieZ.WinUI.dll', 'TieZ.WinUI.runtimeconfig.json', 'TieZ.WinUI.pri', 'App.xbf', 'MainWindow.xbf',
    'HistoryView.xbf', 'ApplicationPickerView.xbf', 'TagManagementView.xbf', 'TagPickerView.xbf')
if ($SelfContained) {
    $required += @('Microsoft.UI.Xaml.dll', 'coreclr.dll', 'hostfxr.dll', 'Microsoft.WindowsAppRuntime.Bootstrap.dll')
}
foreach ($asset in $required) {
    if (-not (Test-Path -LiteralPath (Join-Path $directoryPath $asset) -PathType Leaf)) {
        throw "Published runtime asset is missing: $asset"
    }
}
if ($SelfContained) {
    $runtime = ([IO.File]::ReadAllText((Join-Path $directoryPath 'TieZ.WinUI.runtimeconfig.json')) | ConvertFrom-Json).runtimeOptions
    if ($null -ne $runtime.framework -or $null -ne $runtime.frameworks -or -not $runtime.includedFrameworks) {
        throw 'Self-contained runtime configuration still requires an installed framework.'
    }
}
[pscustomobject]@{ passed = $true; files = $files.Count; nativeSha256 = $ExpectedNativeSha256; selfContained = $SelfContained }
