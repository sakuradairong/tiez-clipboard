param(
    [ValidateSet('Debug', 'Release')][string]$Configuration = 'Debug',
    [ValidateRange(1, 600)][int]$TimeoutSeconds = 120,
    [switch]$IconsOnly
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$exe = Join-Path $root "native/managed/TieZ.Core.Smoke/bin/$Configuration/net10.0/win-x64/TieZ.Core.Smoke.exe"
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw 'Build the matching Rust DLL and TieZ.Core.Smoke project before running this check.'
}
$info = [Diagnostics.ProcessStartInfo]::new($exe)
$info.WorkingDirectory = Split-Path $exe
$info.UseShellExecute = $false
$info.RedirectStandardOutput = $true
$info.RedirectStandardError = $true
if ($IconsOnly) { $info.ArgumentList.Add('--app-icons-only') }
$process = [Diagnostics.Process]::new()
$process.StartInfo = $info
$started = $false
try {
    $started = $process.Start()
    if (-not $started) { throw 'Managed smoke did not start.' }
    $output = $process.StandardOutput.ReadToEndAsync()
    $errors = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        $process.Kill($true)
        $null = $process.WaitForExit(5000)
        throw "Managed smoke exceeded ${TimeoutSeconds}s, including synchronous native teardown."
    }
    if (-not $output.Wait(5000) -or -not $errors.Wait(5000)) {
        throw 'Managed smoke exited without releasing its redirected streams.'
    }
    $output.Result
    $errors.Result
    if ($process.ExitCode -ne 0) { throw "Managed smoke failed with exit $($process.ExitCode)." }
    'PASS: the owned smoke process exited under the process-level watchdog.'
} finally {
    # Never select processes by name: kill only this launched process and its descendants.
    if ($started -and -not $process.HasExited) {
        $process.Kill($true)
        $null = $process.WaitForExit(5000)
    }
    $process.Dispose()
}
