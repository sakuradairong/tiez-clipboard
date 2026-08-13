[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release",

    [int]$ReadyTimeoutSeconds = 15,
    [int]$SampleSeconds = 30,
    [switch]$KeepOpen
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$ArtifactDir = Join-Path $Root "artifacts\x64\$Configuration"
$Executable = Join-Path $ArtifactDir "Tiez.WinUIProbe.exe"
$CoreDll = Join-Path $ArtifactDir "tiez_winui_core.dll"
$ReadyFile = Join-Path $ArtifactDir "ready.txt"
$OutputFile = Join-Path $ArtifactDir "measurement.json"

if (-not (Test-Path $Executable)) {
    throw "WinUI executable was not found. Run .\build.ps1 first."
}
if (-not (Test-Path $CoreDll)) {
    throw "Rust core DLL was not found. Run .\build.ps1 first."
}

Remove-Item $ReadyFile -Force -ErrorAction SilentlyContinue
$env:TIEZ_WINUI_CORE_DLL = $CoreDll
$env:TIEZ_WINUI_READY_FILE = $ReadyFile

$startedAt = [System.Diagnostics.Stopwatch]::StartNew()
$process = Start-Process $Executable -PassThru
$deadline = (Get-Date).AddSeconds($ReadyTimeoutSeconds)

while (-not (Test-Path $ReadyFile)) {
    if ($process.HasExited) {
        throw "The WinUI probe exited before reporting ready (exit code $($process.ExitCode))."
    }
    if ((Get-Date) -ge $deadline) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "The WinUI probe did not report ready within $ReadyTimeoutSeconds seconds."
    }
    Start-Sleep -Milliseconds 50
}

$readyMs = $startedAt.Elapsed.TotalMilliseconds
$samples = [System.Collections.Generic.List[object]]::new()
$sampleStopwatch = [System.Diagnostics.Stopwatch]::StartNew()

while ($sampleStopwatch.Elapsed.TotalSeconds -lt $SampleSeconds -and -not $process.HasExited) {
    $current = Get-Process -Id $process.Id -ErrorAction Stop
    $samples.Add([ordered]@{
        elapsed_ms = [math]::Round($sampleStopwatch.Elapsed.TotalMilliseconds, 1)
        private_bytes = [int64]$current.PrivateMemorySize64
        working_set_bytes = [int64]$current.WorkingSet64
        handles = $current.HandleCount
        threads = $current.Threads.Count
    })
    Start-Sleep -Seconds 1
}

$result = [ordered]@{
    schema_version = 1
    measured_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    executable = $Executable
    pid = $process.Id
    requested_to_ready_ms = [math]::Round($readyMs, 1)
    sample_seconds = $SampleSeconds
    samples = $samples
}

$result | ConvertTo-Json -Depth 6 | Set-Content $OutputFile -Encoding utf8
Write-Host "Ready in $([math]::Round($readyMs, 1)) ms"
Write-Host "Measurement: $OutputFile"

if (-not $KeepOpen -and -not $process.HasExited) {
    $process.CloseMainWindow() | Out-Null
    Start-Sleep -Milliseconds 500
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
    }
}
