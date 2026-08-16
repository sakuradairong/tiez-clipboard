[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release",

    [int]$ReadyTimeoutSeconds = 15,

    [int]$RedirectTimeoutSeconds = 10,

    [ValidateRange(1, 1000)]
    [int]$LifecycleCycles = 1
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$ArtifactDirectory = Join-Path $Root "artifacts\x64\$Configuration"
$Executable = Join-Path $ArtifactDirectory "TieZ.exe"
$CoreDll = Join-Path $ArtifactDirectory "tiez_winui_core.dll"

if (-not (Test-Path -LiteralPath $Executable)) {
    throw "WinUI executable was not found. Run .\build.ps1 first."
}
if (-not (Test-Path -LiteralPath $CoreDll)) {
    throw "Rust core DLL was not found. Run .\build.ps1 first."
}

$runningTieZ = Get-Process -Name "TieZ" -ErrorAction SilentlyContinue
if ($null -ne $runningTieZ) {
    $processList = ($runningTieZ.Id | Sort-Object) -join ", "
    throw "TieZ is already running (PID: $processList). Exit it before this isolated test."
}

$EnvironmentNames = @(
    "TIEZ_WINUI_CORE_DLL",
    "TIEZ_WINUI_DB_PATH",
    "TIEZ_WINUI_DB_READ_ONLY",
    "TIEZ_WINUI_READY_FILE"
)
$PreviousEnvironment = @{}
foreach ($name in $EnvironmentNames) {
    $PreviousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}

$TemporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$TestDirectory = [IO.Path]::GetFullPath((Join-Path $TemporaryRoot (
    "tiez-single-instance-" + [guid]::NewGuid().ToString("N"))))
if (-not $TestDirectory.StartsWith(
        $TemporaryRoot,
        [StringComparison]::OrdinalIgnoreCase)) {
    throw "Unsafe temporary test path."
}
New-Item -ItemType Directory -Path $TestDirectory | Out-Null

$Primary = $null
$HiddenSecondary = $null
$MinimizedSecondary = $null
$VisibleSecondary = $null
$ReadyStopwatch = $null

try {
    [Environment]::SetEnvironmentVariable("TIEZ_WINUI_CORE_DLL", $CoreDll, "Process")
    [Environment]::SetEnvironmentVariable(
        "TIEZ_WINUI_DB_PATH",
        (Join-Path $TestDirectory "clipboard.db"),
        "Process")
    [Environment]::SetEnvironmentVariable("TIEZ_WINUI_DB_READ_ONLY", $null, "Process")
    [Environment]::SetEnvironmentVariable(
        "TIEZ_WINUI_READY_FILE",
        (Join-Path $TestDirectory "ready.txt"),
        "Process")

    $ReadyStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $Primary = Start-Process `
        -FilePath $Executable `
        -ArgumentList "--autostart" `
        -WindowStyle Hidden `
        -PassThru

    $ReadyFile = [Environment]::GetEnvironmentVariable("TIEZ_WINUI_READY_FILE", "Process")
    $ReadyDeadline = (Get-Date).AddSeconds($ReadyTimeoutSeconds)
    while (-not (Test-Path -LiteralPath $ReadyFile)) {
        $Primary.Refresh()
        if ($Primary.HasExited) {
            throw "Primary instance exited before reporting ready (exit code $($Primary.ExitCode))."
        }
        if ((Get-Date) -ge $ReadyDeadline) {
            throw "Primary instance did not report ready within $ReadyTimeoutSeconds seconds."
        }
        Start-Sleep -Milliseconds 50
    }
    $ReadyStopwatch.Stop()
    $RequestedToReadyMs = [Math]::Round($ReadyStopwatch.Elapsed.TotalMilliseconds, 1)

    Start-Sleep -Milliseconds 400
    $Primary.Refresh()
    $InitialHandle = [int64]$Primary.MainWindowHandle
    $InitialWorkingSetMiB = [Math]::Round($Primary.WorkingSet64 / 1MB, 2)
    $InitialPrivateMemoryMiB = [Math]::Round($Primary.PrivateMemorySize64 / 1MB, 2)

    $HiddenSecondary = Start-Process `
        -FilePath $Executable `
        -ArgumentList "--autostart" `
        -WindowStyle Hidden `
        -PassThru
    if (-not $HiddenSecondary.WaitForExit($RedirectTimeoutSeconds * 1000)) {
        throw "Hidden secondary instance did not redirect and exit."
    }
    $HiddenExitCode = $HiddenSecondary.ExitCode

    Start-Sleep -Milliseconds 400
    $Primary.Refresh()
    $HandleAfterHiddenRedirect = [int64]$Primary.MainWindowHandle

    $MinimizedSecondary = Start-Process `
        -FilePath $Executable `
        -ArgumentList "--minimized" `
        -WindowStyle Hidden `
        -PassThru
    if (-not $MinimizedSecondary.WaitForExit($RedirectTimeoutSeconds * 1000)) {
        throw "Minimized secondary instance did not redirect and exit."
    }
    $MinimizedExitCode = $MinimizedSecondary.ExitCode

    Start-Sleep -Milliseconds 400
    $Primary.Refresh()
    $HandleAfterMinimizedRedirect = [int64]$Primary.MainWindowHandle

    $Primary.Refresh()
    if ($InitialHandle -ne 0) {
        throw "Primary instance was not hidden (window handle $InitialHandle)."
    }
    if ($HiddenExitCode -ne 0 -or $HandleAfterHiddenRedirect -ne 0) {
        throw "Hidden activation failed or revealed the primary window."
    }
    if ($MinimizedExitCode -ne 0 -or $HandleAfterMinimizedRedirect -ne 0) {
        throw "Minimized activation failed or revealed the primary window."
    }
    if ($Primary.HasExited) {
        throw "Primary instance exited during redirection."
    }

    $VisibleExitCode = $null
    $ShownHandle = [int64]0
    $FinalHiddenHandle = [int64]0
    for ($cycle = 1; $cycle -le $LifecycleCycles; $cycle++) {
        $VisibleSecondary = Start-Process -FilePath $Executable -PassThru
        if (-not $VisibleSecondary.WaitForExit($RedirectTimeoutSeconds * 1000)) {
            throw "Visible secondary instance did not redirect and exit in cycle $cycle."
        }
        $VisibleExitCode = $VisibleSecondary.ExitCode

        $ShowDeadline = (Get-Date).AddSeconds($RedirectTimeoutSeconds)
        do {
            Start-Sleep -Milliseconds 50
            $Primary.Refresh()
            $ShownHandle = [int64]$Primary.MainWindowHandle
        } while ($ShownHandle -eq 0 -and (Get-Date) -lt $ShowDeadline)

        if ($VisibleExitCode -ne 0 -or $ShownHandle -eq 0) {
            throw "Visible activation did not reveal the primary window in cycle $cycle."
        }
        if (-not $Primary.CloseMainWindow()) {
            throw "WM_CLOSE could not be sent to the primary window in cycle $cycle."
        }

        $HideDeadline = (Get-Date).AddSeconds($RedirectTimeoutSeconds)
        do {
            Start-Sleep -Milliseconds 50
            $Primary.Refresh()
            $FinalHiddenHandle = [int64]$Primary.MainWindowHandle
        } while ($FinalHiddenHandle -ne 0 -and (Get-Date) -lt $HideDeadline)

        if ($FinalHiddenHandle -ne 0) {
            throw "Primary window did not close to the tray in cycle $cycle."
        }
        if ($Primary.HasExited) {
            throw "Primary instance exited during lifecycle cycle $cycle."
        }
    }

    $Primary.Refresh()
    $FinalWorkingSetMiB = [Math]::Round($Primary.WorkingSet64 / 1MB, 2)
    $FinalPrivateMemoryMiB = [Math]::Round($Primary.PrivateMemorySize64 / 1MB, 2)
    $PeakWorkingSetMiB = [Math]::Round($Primary.PeakWorkingSet64 / 1MB, 2)

    [pscustomobject]@{
        PrimaryPid = $Primary.Id
        PrimaryAlive = -not $Primary.HasExited
        RequestedToReadyMs = $RequestedToReadyMs
        InitialHiddenHandle = $InitialHandle
        HiddenSecondaryExitCode = $HiddenExitCode
        HandleAfterHiddenRedirect = $HandleAfterHiddenRedirect
        MinimizedSecondaryExitCode = $MinimizedExitCode
        HandleAfterMinimizedRedirect = $HandleAfterMinimizedRedirect
        LifecycleCycles = $LifecycleCycles
        VisibleSecondaryExitCode = $VisibleExitCode
        LastShownHandle = $ShownHandle
        FinalHiddenHandle = $FinalHiddenHandle
        InitialWorkingSetMiB = $InitialWorkingSetMiB
        FinalWorkingSetMiB = $FinalWorkingSetMiB
        PeakWorkingSetMiB = $PeakWorkingSetMiB
        InitialPrivateMemoryMiB = $InitialPrivateMemoryMiB
        FinalPrivateMemoryMiB = $FinalPrivateMemoryMiB
        PrivateMemoryGrowthMiB = [Math]::Round(
            $FinalPrivateMemoryMiB - $InitialPrivateMemoryMiB,
            2)
        HandleCount = $Primary.HandleCount
        DatabaseCreated = Test-Path -LiteralPath (
            [Environment]::GetEnvironmentVariable("TIEZ_WINUI_DB_PATH", "Process"))
    }
}
finally {
    foreach ($process in @(
            $VisibleSecondary,
            $MinimizedSecondary,
            $HiddenSecondary,
            $Primary)) {
        if ($null -eq $process) {
            continue
        }
        try {
            $process.Refresh()
            if (-not $process.HasExited) {
                Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
                $null = $process.WaitForExit(5000)
            }
        }
        catch {
            # The exact process created by this test has already terminated.
        }
    }

    foreach ($name in $EnvironmentNames) {
        [Environment]::SetEnvironmentVariable(
            $name,
            $PreviousEnvironment[$name],
            "Process")
    }

    $ResolvedTestDirectory = [IO.Path]::GetFullPath($TestDirectory)
    if ($ResolvedTestDirectory.StartsWith(
            $TemporaryRoot,
            [StringComparison]::OrdinalIgnoreCase) -and
        (Test-Path -LiteralPath $ResolvedTestDirectory)) {
        Remove-Item -LiteralPath $ResolvedTestDirectory -Recurse -Force
    }
}
