$ErrorActionPreference = 'Stop'
$root = Join-Path ([IO.Path]::GetTempPath()) "TieZ-publish-audit-$([guid]::NewGuid().ToString('N'))"
$null = New-Item -ItemType Directory -Path $root
$verifier = Join-Path $PSScriptRoot 'verify-native-publish.ps1'
$checks = 0
function Assert-AuditRejected([string]$ExpectedMessage) {
    try {
        $null = & $verifier -Directory $root -ExpectedNativeSha256 $script:hash
    } catch {
        if ($_.Exception.Message -notlike "*$ExpectedMessage*") { throw }
        $script:checks++
        return
    }
    throw "Audit accepted a forbidden fixture: $ExpectedMessage"
}
try {
    $required = @('TieZ.WinUI.exe', 'TieZ.WinUI.dll', 'Microsoft.UI.Xaml.dll', 'coreclr.dll', 'hostfxr.dll',
        'Microsoft.WindowsAppRuntime.Bootstrap.dll', 'tiez_native_core.dll', 'TieZ.WinUI.pri', 'App.xbf', 'MainWindow.xbf',
        'HistoryView.xbf', 'ApplicationPickerView.xbf', 'TagManagementView.xbf', 'TagPickerView.xbf')
    foreach ($name in $required) { [IO.File]::WriteAllText((Join-Path $root $name), 'controlled non-executable fixture') }
    $runtime = Join-Path $root 'TieZ.WinUI.runtimeconfig.json'
    $included = '{"runtimeOptions":{"includedFrameworks":[{"name":"Microsoft.NETCore.App","version":"10.0.8"}]}}'
    [IO.File]::WriteAllText($runtime, $included)
    $deps = Join-Path $root 'TieZ.WinUI.deps.json'
    [IO.File]::WriteAllText($deps, '{"libraries":{"TieZ.WinUI/1.0":{}}}')
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $root 'tiez_native_core.dll')).Hash
    if (-not (& $verifier -Directory $root -ExpectedNativeSha256 $hash).passed) { throw 'Valid fixture rejected' }
    $checks++
    [IO.File]::AppendAllText((Join-Path $root 'tiez_native_core.dll'), 'changed')
    Assert-AuditRejected 'does not match'
    [IO.File]::WriteAllText((Join-Path $root 'tiez_native_core.dll'), 'controlled non-executable fixture')
    foreach ($name in @('WebView2Loader.dll', 'Microsoft.Web.WebView2.Core.dll', 'tauri.dll', 'react.production.js')) {
        $path = Join-Path $root $name
        [IO.File]::WriteAllText($path, 'forbidden fixture')
        try { Assert-AuditRejected 'runtime assets remain' } finally { [IO.File]::Delete($path) }
    }
    [IO.File]::WriteAllText($deps, '{"libraries":{"Microsoft.Web.WebView2/1.0":{}}}')
    Assert-AuditRejected 'runtime dependencies remain'
    [IO.File]::WriteAllText($deps, '{"libraries":{"TieZ.WinUI/1.0":{}}}')
    [IO.File]::WriteAllText($runtime, '{"runtimeOptions":{"framework":{"name":"Microsoft.NETCore.App","version":"10.0.8"}}}')
    Assert-AuditRejected 'requires an installed framework'
    [IO.File]::WriteAllText($runtime, $included)
    [IO.File]::Delete((Join-Path $root 'MainWindow.xbf'))
    Assert-AuditRejected 'runtime asset is missing'
    [IO.File]::WriteAllText((Join-Path $root 'MainWindow.xbf'), 'controlled non-executable fixture')
    [IO.File]::Delete((Join-Path $root 'coreclr.dll'))
    Assert-AuditRejected 'runtime asset is missing'
    if (-not (& $verifier -Directory $root -ExpectedNativeSha256 $hash -SelfContained $false).passed) {
        throw 'Explicit framework-dependent mode was rejected'
    }
    $checks++
    "PASS: $checks isolated publish-audit checks; source DLL mismatch, prohibited assets/dependencies and missing runtime rejection."
} finally {
    if ((Get-Item -LiteralPath $root).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Audit fixture was replaced by a reparse point.' }
    [IO.Directory]::Delete($root, $true)
}
