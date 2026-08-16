[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release",

    [int]$ReadyTimeoutSeconds = 15,

    [int]$ActivationTimeoutSeconds = 10
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

if (-not ("TieZHotkeyProbe" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public static class TieZHotkeyProbe
{
    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool RegisterHotKey(IntPtr hwnd, int id, uint modifiers, uint virtualKey);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool UnregisterHotKey(IntPtr hwnd, int id);

    [DllImport("user32.dll")]
    public static extern void keybd_event(byte virtualKey, byte scanCode, uint flags, UIntPtr extraInfo);

    [DllImport("user32.dll")]
    public static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extraInfo);
}
"@
}

$EnvironmentNames = @(
    "TIEZ_WINUI_CORE_DLL",
    "TIEZ_WINUI_USE_SYNTHETIC_DATA",
    "TIEZ_WINUI_HOTKEY",
    "TIEZ_WINUI_READY_FILE"
)
$PreviousEnvironment = @{}
foreach ($name in $EnvironmentNames) {
    $PreviousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}

$TemporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$TestDirectory = [IO.Path]::GetFullPath((Join-Path $TemporaryRoot (
    "tiez-hotkey-" + [guid]::NewGuid().ToString("N"))))
if (-not $TestDirectory.StartsWith(
        $TemporaryRoot,
        [StringComparison]::OrdinalIgnoreCase)) {
    throw "Unsafe temporary test path."
}
New-Item -ItemType Directory -Path $TestDirectory | Out-Null

$Primary = $null
$ProbeRegistered = $false
$ProbeId = 0x5A71
$ModAlt = 0x0001
$ModControl = 0x0002
$ModNoRepeat = 0x4000
$VkControl = 0x11
$VkMenu = 0x12
$VkF24 = 0x87
$KeyUp = 0x0002
$MouseMiddleDown = 0x0020
$MouseMiddleUp = 0x0040
$CustomModifiers = $ModAlt -bor $ModControl -bor $ModNoRepeat

try {
    [Environment]::SetEnvironmentVariable("TIEZ_WINUI_CORE_DLL", $CoreDll, "Process")
    [Environment]::SetEnvironmentVariable("TIEZ_WINUI_USE_SYNTHETIC_DATA", "1", "Process")
    [Environment]::SetEnvironmentVariable("TIEZ_WINUI_HOTKEY", "Ctrl+Alt+F24", "Process")
    [Environment]::SetEnvironmentVariable(
        "TIEZ_WINUI_READY_FILE",
        (Join-Path $TestDirectory "ready.txt"),
        "Process")

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

    $ProbeRegistered = [TieZHotkeyProbe]::RegisterHotKey(
        [IntPtr]::Zero,
        $ProbeId,
        $CustomModifiers,
        $VkF24)
    $RegistrationError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    if ($ProbeRegistered) {
        [TieZHotkeyProbe]::UnregisterHotKey([IntPtr]::Zero, $ProbeId) | Out-Null
        $ProbeRegistered = $false
        throw "Ctrl+Alt+F24 remained available; TieZ did not register the configured hotkey."
    }
    if ($RegistrationError -ne 1409) {
        throw "The configured hotkey probe failed with unexpected Win32 error $RegistrationError."
    }

    [TieZHotkeyProbe]::keybd_event($VkControl, 0, 0, [UIntPtr]::Zero)
    [TieZHotkeyProbe]::keybd_event($VkMenu, 0, 0, [UIntPtr]::Zero)
    [TieZHotkeyProbe]::keybd_event($VkF24, 0, 0, [UIntPtr]::Zero)
    [TieZHotkeyProbe]::keybd_event($VkF24, 0, $KeyUp, [UIntPtr]::Zero)
    [TieZHotkeyProbe]::keybd_event($VkMenu, 0, $KeyUp, [UIntPtr]::Zero)
    [TieZHotkeyProbe]::keybd_event($VkControl, 0, $KeyUp, [UIntPtr]::Zero)

    $ActivationDeadline = (Get-Date).AddSeconds($ActivationTimeoutSeconds)
    do {
        Start-Sleep -Milliseconds 50
        $Primary.Refresh()
        $ShownHandle = [int64]$Primary.MainWindowHandle
    } while ($ShownHandle -eq 0 -and (Get-Date) -lt $ActivationDeadline)

    if ($ShownHandle -eq 0) {
        throw "Ctrl+Alt+F24 was registered but did not reveal the TieZ main window."
    }
    if (-not $Primary.CloseMainWindow()) {
        throw "The activated main window did not accept WM_CLOSE."
    }

    $HideDeadline = (Get-Date).AddSeconds($ActivationTimeoutSeconds)
    do {
        Start-Sleep -Milliseconds 50
        $Primary.Refresh()
        $FinalHiddenHandle = [int64]$Primary.MainWindowHandle
    } while ($FinalHiddenHandle -ne 0 -and (Get-Date) -lt $HideDeadline)

    if ($FinalHiddenHandle -ne 0 -or $Primary.HasExited) {
        throw "The hotkey-activated window did not return to the tray-owned process."
    }

    $KeyboardPrimaryPid = $Primary.Id
    $KeyboardShownHandle = $ShownHandle
    $KeyboardFinalHiddenHandle = $FinalHiddenHandle
    Stop-Process -Id $Primary.Id -Force
    if (-not $Primary.WaitForExit(5000)) {
        throw "The keyboard-hotkey test process did not stop before the mouse test."
    }
    $Primary = $null

    [Environment]::SetEnvironmentVariable("TIEZ_WINUI_HOTKEY", "MouseMiddle", "Process")
    [Environment]::SetEnvironmentVariable(
        "TIEZ_WINUI_READY_FILE",
        (Join-Path $TestDirectory "mouse-ready.txt"),
        "Process")

    $Primary = Start-Process `
        -FilePath $Executable `
        -ArgumentList "--autostart" `
        -WindowStyle Hidden `
        -PassThru

    $MouseReadyFile = [Environment]::GetEnvironmentVariable(
        "TIEZ_WINUI_READY_FILE",
        "Process")
    $ReadyDeadline = (Get-Date).AddSeconds($ReadyTimeoutSeconds)
    while (-not (Test-Path -LiteralPath $MouseReadyFile)) {
        $Primary.Refresh()
        if ($Primary.HasExited) {
            throw "Mouse-middle test instance exited before reporting ready (exit code $($Primary.ExitCode))."
        }
        if ((Get-Date) -ge $ReadyDeadline) {
            throw "Mouse-middle test instance did not report ready within $ReadyTimeoutSeconds seconds."
        }
        Start-Sleep -Milliseconds 50
    }

    [TieZHotkeyProbe]::mouse_event($MouseMiddleDown, 0, 0, 0, [UIntPtr]::Zero)
    [TieZHotkeyProbe]::mouse_event($MouseMiddleUp, 0, 0, 0, [UIntPtr]::Zero)

    $ActivationDeadline = (Get-Date).AddSeconds($ActivationTimeoutSeconds)
    do {
        Start-Sleep -Milliseconds 50
        $Primary.Refresh()
        $MouseShownHandle = [int64]$Primary.MainWindowHandle
    } while ($MouseShownHandle -eq 0 -and (Get-Date) -lt $ActivationDeadline)

    if ($MouseShownHandle -eq 0) {
        throw "MouseMiddle did not reveal the TieZ main window."
    }
    if (-not $Primary.CloseMainWindow()) {
        throw "The mouse-middle-activated main window did not accept WM_CLOSE."
    }

    $HideDeadline = (Get-Date).AddSeconds($ActivationTimeoutSeconds)
    do {
        Start-Sleep -Milliseconds 50
        $Primary.Refresh()
        $MouseFinalHiddenHandle = [int64]$Primary.MainWindowHandle
    } while ($MouseFinalHiddenHandle -ne 0 -and (Get-Date) -lt $HideDeadline)

    if ($MouseFinalHiddenHandle -ne 0 -or $Primary.HasExited) {
        throw "The mouse-middle-activated window did not return to the tray-owned process."
    }

    [pscustomobject]@{
        KeyboardPrimaryPid = $KeyboardPrimaryPid
        KeyboardHotkey = "Ctrl+Alt+F24"
        RegistrationBlockedForProbe = $RegistrationError -eq 1409
        KeyboardActivatedWindowHandle = $KeyboardShownHandle
        KeyboardFinalHiddenHandle = $KeyboardFinalHiddenHandle
        MousePrimaryPid = $Primary.Id
        MouseHotkey = "MouseMiddle"
        MouseActivatedWindowHandle = $MouseShownHandle
        MouseFinalHiddenHandle = $MouseFinalHiddenHandle
        MousePrimaryAlive = -not $Primary.HasExited
    }
}
finally {
    if ($ProbeRegistered) {
        [TieZHotkeyProbe]::UnregisterHotKey([IntPtr]::Zero, $ProbeId) | Out-Null
    }
    if ($null -ne $Primary) {
        try {
            $Primary.Refresh()
            if (-not $Primary.HasExited) {
                Stop-Process -Id $Primary.Id -Force -ErrorAction SilentlyContinue
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
