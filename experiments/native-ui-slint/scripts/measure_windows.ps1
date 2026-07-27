[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Label,

    [Parameter(Mandatory = $true)]
    [string]$Executable,

    [string[]]$ExecutableArgument = @(),

    [Parameter(Mandatory = $true)]
    [string]$WindowTitle,

    [ValidateSet("visible", "hidden", "destroyed")]
    [string]$SampleState = "visible",

    [string]$StateTriggerChord = "",

    [ValidateRange(0, 100)]
    [int]$StateTriggerCount = 1,

    [ValidateRange(0.0, 300.0)]
    [double]$StateTriggerDelaySeconds = 0.0,

    [ValidateRange(0.0, 30.0)]
    [double]$StateTriggerIntervalSeconds = 0.25,

    [switch]$MeasureWake,

    [string]$WakeTriggerChord = "",

    [ValidateRange(1, 300)]
    [int]$WindowTimeoutSeconds = 30,

    [ValidateRange(1, 300)]
    [int]$StateTimeoutSeconds = 30,

    [ValidateRange(0.0, 600.0)]
    [double]$WarmupSeconds = 5.0,

    [ValidateRange(1, 100)]
    [int]$Runs = 5,

    [ValidateRange(1, 100)]
    [int]$SamplesPerRun = 3,

    [ValidateRange(0.1, 60.0)]
    [double]$SampleIntervalSeconds = 1.0,

    [Parameter(Mandatory = $true)]
    [string]$Output
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "measure_windows.ps1 must run on Windows."
}

if ($null -eq ("BenchmarkNative" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public sealed class BenchmarkWindowInfo
{
    public IntPtr Handle { get; set; }
    public uint ProcessId { get; set; }
    public string Title { get; set; }
    public bool Visible { get; set; }
}

public static class BenchmarkNative
{
    private delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int maxCount);

    [DllImport("user32.dll")]
    private static extern int GetWindowTextLength(IntPtr hWnd);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);

    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll")]
    private static extern bool IsWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(uint inputCount, INPUT[] inputs, int inputSize);

    [StructLayout(LayoutKind.Sequential)]
    private struct INPUT
    {
        public uint type;
        public InputUnion data;
    }

    [StructLayout(LayoutKind.Explicit)]
    private struct InputUnion
    {
        [FieldOffset(0)]
        public KEYBDINPUT keyboard;

        [FieldOffset(0)]
        public MOUSEINPUT mouse;

        [FieldOffset(0)]
        public HARDWAREINPUT hardware;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct KEYBDINPUT
    {
        public ushort virtualKey;
        public ushort scanCode;
        public uint flags;
        public uint time;
        public UIntPtr extraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MOUSEINPUT
    {
        public int dx;
        public int dy;
        public uint mouseData;
        public uint flags;
        public uint time;
        public UIntPtr extraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct HARDWAREINPUT
    {
        public uint message;
        public ushort parameterLow;
        public ushort parameterHigh;
    }

    public static BenchmarkWindowInfo[] GetTopLevelWindows()
    {
        var windows = new List<BenchmarkWindowInfo>();
        EnumWindows(delegate(IntPtr handle, IntPtr ignored)
        {
            int length = GetWindowTextLength(handle);
            var title = new StringBuilder(length + 1);
            GetWindowText(handle, title, title.Capacity);
            uint processId;
            GetWindowThreadProcessId(handle, out processId);
            windows.Add(new BenchmarkWindowInfo
            {
                Handle = handle,
                ProcessId = processId,
                Title = title.ToString(),
                Visible = IsWindowVisible(handle)
            });
            return true;
        }, IntPtr.Zero);
        return windows.ToArray();
    }

    public static bool WindowExists(IntPtr handle)
    {
        return IsWindow(handle);
    }

    public static bool WindowIsVisible(IntPtr handle)
    {
        return IsWindow(handle) && IsWindowVisible(handle);
    }

    public static bool FocusWindow(IntPtr handle, int timeoutMilliseconds)
    {
        if (!IsWindow(handle))
        {
            return false;
        }
        SetForegroundWindow(handle);
        int waited = 0;
        while (waited <= timeoutMilliseconds)
        {
            if (GetForegroundWindow() == handle)
            {
                return true;
            }
            Thread.Sleep(25);
            waited += 25;
        }
        return false;
    }

    private static void SendKey(byte virtualKey, bool keyUp)
    {
        var input = new INPUT();
        input.type = 1;
        input.data.keyboard.virtualKey = virtualKey;
        input.data.keyboard.flags = keyUp ? 0x0002U : 0U;
        var inputs = new INPUT[] { input };
        if (SendInput(1, inputs, Marshal.SizeOf(typeof(INPUT))) != 1)
        {
            throw new Win32Exception(Marshal.GetLastWin32Error(), "SendInput failed");
        }
    }

    public static void PressChord(byte[] virtualKeys)
    {
        var pressed = new List<byte>();
        Exception failure = null;
        try
        {
            foreach (byte virtualKey in virtualKeys)
            {
                pressed.Add(virtualKey);
                SendKey(virtualKey, false);
            }
            Thread.Sleep(25);
        }
        catch (Exception error)
        {
            failure = error;
        }
        finally
        {
            for (int index = pressed.Count - 1; index >= 0; index--)
            {
                try
                {
                    SendKey(pressed[index], true);
                }
                catch (Exception error)
                {
                    if (failure == null)
                    {
                        failure = error;
                    }
                }
            }
        }
        if (failure != null)
        {
            throw failure;
        }
    }
}
"@
}

function Get-Median {
    param([double[]]$Values)

    if ($Values.Count -eq 0) {
        return $null
    }
    $sorted = @($Values | Sort-Object)
    $middle = [Math]::Floor($sorted.Count / 2)
    if (($sorted.Count % 2) -eq 1) {
        return [double]$sorted[$middle]
    }
    return ([double]$sorted[$middle - 1] + [double]$sorted[$middle]) / 2.0
}

function Get-ProcessTreeIds {
    param(
        [int]$RootProcessId,
        [object[]]$AllProcesses = @(Get-CimInstance Win32_Process)
    )

    $ids = [System.Collections.Generic.HashSet[int]]::new()
    [void]$ids.Add($RootProcessId)

    $changed = $true
    while ($changed) {
        $changed = $false
        foreach ($process in $AllProcesses) {
            $processId = [int]$process.ProcessId
            $parentProcessId = [int]$process.ParentProcessId
            if ($ids.Contains($parentProcessId) -and $ids.Add($processId)) {
                $changed = $true
            }
        }
    }

    return @($ids | Sort-Object)
}

function Get-MatchingWindows {
    param(
        [int[]]$ProcessIds,
        [string]$ExactTitle
    )

    $idSet = [System.Collections.Generic.HashSet[uint32]]::new()
    foreach ($processId in $ProcessIds) {
        [void]$idSet.Add([uint32]$processId)
    }

    return @(
        [BenchmarkNative]::GetTopLevelWindows() |
            Where-Object { $idSet.Contains($_.ProcessId) -and $_.Title -ceq $ExactTitle }
    )
}

function Test-TrackedWindowState {
    param(
        [int]$RootProcessId,
        [IntPtr]$WindowHandle,
        [string]$ExpectedState,
        [string]$WindowTitle
    )

    switch ($ExpectedState) {
        "visible" { return [BenchmarkNative]::WindowIsVisible($WindowHandle) }
        "hidden" {
            if (-not [BenchmarkNative]::WindowExists($WindowHandle) -or
                [BenchmarkNative]::WindowIsVisible($WindowHandle)) {
                return $false
            }
            $treeIds = @(Get-ProcessTreeIds -RootProcessId $RootProcessId)
            $visibleMatches = @(
                Get-MatchingWindows -ProcessIds $treeIds -ExactTitle $WindowTitle |
                    Where-Object Visible
            )
            return $visibleMatches.Count -eq 0
        }
        "destroyed" {
            if ([BenchmarkNative]::WindowExists($WindowHandle)) {
                return $false
            }
            $treeIds = @(Get-ProcessTreeIds -RootProcessId $RootProcessId)
            return @(Get-MatchingWindows -ProcessIds $treeIds -ExactTitle $WindowTitle).Count -eq 0
        }
        default { throw "Unsupported state: $ExpectedState" }
    }
}

function Wait-ForWindowState {
    param(
        [System.Diagnostics.Process]$RootProcess,
        [IntPtr]$WindowHandle,
        [string]$ExpectedState,
        [string]$WindowTitle,
        [int]$TimeoutSeconds
    )

    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        $RootProcess.Refresh()
        if ($RootProcess.HasExited) {
            throw "Root process exited before reaching '$ExpectedState'. Exit code: $($RootProcess.ExitCode)"
        }

        if (Test-TrackedWindowState -RootProcessId $RootProcess.Id -WindowHandle $WindowHandle -ExpectedState $ExpectedState -WindowTitle $WindowTitle) {
            return
        }
        Start-Sleep -Milliseconds 50
    }

    $handleText = '0x{0:X}' -f $WindowHandle.ToInt64()
    throw "Timed out after $TimeoutSeconds seconds waiting for window $handleText to become '$ExpectedState'."
}

function Convert-ChordToVirtualKeys {
    param([string]$Chord)

    if ([string]::IsNullOrWhiteSpace($Chord)) {
        return [byte[]]@()
    }

    $keys = [System.Collections.Generic.List[byte]]::new()
    foreach ($rawToken in $Chord.Split("+")) {
        $token = $rawToken.Trim().ToUpperInvariant()
        $virtualKey = switch ($token) {
            "ALT" { 0x12; break }
            "CTRL" { 0x11; break }
            "CONTROL" { 0x11; break }
            "SHIFT" { 0x10; break }
            "WIN" { 0x5B; break }
            "WINDOWS" { 0x5B; break }
            "ESC" { 0x1B; break }
            "ESCAPE" { 0x1B; break }
            "ENTER" { 0x0D; break }
            "SPACE" { 0x20; break }
            default {
                if ($token -match '^F([1-9]|1[0-9]|2[0-4])$') {
                    0x70 + [int]$Matches[1] - 1
                } elseif ($token.Length -eq 1 -and $token[0] -match '[A-Z0-9]') {
                    [int][char]$token[0]
                } else {
                    throw "Unsupported key in chord '$Chord': '$rawToken'"
                }
            }
        }
        $keys.Add([byte]$virtualKey)
    }
    return $keys.ToArray()
}

function Get-ProcessRole {
    param(
        [int]$ProcessId,
        [int]$RootProcessId,
        [string]$Name,
        [string]$CommandLine
    )

    if ($ProcessId -eq $RootProcessId) {
        return "application"
    }
    if ($Name -notmatch "msedgewebview2") {
        return "child"
    }
    if ($CommandLine -match "--type=renderer") {
        return "webview2-renderer"
    }
    if ($CommandLine -match "--type=gpu-process") {
        return "webview2-gpu"
    }
    if ($CommandLine -match "--utility-sub-type=network") {
        return "webview2-network"
    }
    if ($CommandLine -match "--type=utility") {
        return "webview2-utility"
    }
    if ($CommandLine -match "--type=crashpad-handler") {
        return "webview2-crashpad"
    }
    return "webview2-browser"
}

function Convert-CimCreationDateToUtc {
    param([object]$Value)

    if ($Value -is [DateTime]) {
        return ([DateTime]$Value).ToUniversalTime()
    }
    if ($Value -is [DateTimeOffset]) {
        return ([DateTimeOffset]$Value).UtcDateTime
    }

    $text = [string]$Value
    if ([string]::IsNullOrWhiteSpace($text)) {
        throw "CIM CreationDate was empty."
    }
    try {
        return [Management.ManagementDateTimeConverter]::ToDateTime($text).ToUniversalTime()
    } catch {
        $styles = [Globalization.DateTimeStyles]::AssumeUniversal -bor
            [Globalization.DateTimeStyles]::AdjustToUniversal
        return [DateTime]::Parse(
            $text,
            [Globalization.CultureInfo]::InvariantCulture,
            $styles
        ).ToUniversalTime()
    }
}

function Get-ProcessTreeSample {
    param([int]$RootProcessId)

    for ($attempt = 1; $attempt -le 5; $attempt++) {
        $cimProcesses = @(Get-CimInstance Win32_Process)
        $cimById = @{}
        foreach ($cimProcess in $cimProcesses) {
            $cimById[[int]$cimProcess.ProcessId] = $cimProcess
        }

        $treeIds = @(
            Get-ProcessTreeIds -RootProcessId $RootProcessId -AllProcesses $cimProcesses
        )
        $performanceById = @{}
        foreach ($performanceProcess in @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process)) {
            $performanceById[[int]$performanceProcess.IDProcess] = $performanceProcess
        }

        $processes = @()
        $sampleIncomplete = $false
        foreach ($processId in $treeIds) {
            $cimProcess = $cimById[$processId]
            $performanceProcess = $performanceById[$processId]
            try {
                $process = Get-Process -Id $processId -ErrorAction Stop
                $processStartUtc = $process.StartTime.ToUniversalTime()
            } catch {
                $sampleIncomplete = $true
                break
            }

            if ($null -eq $cimProcess -or $null -eq $performanceProcess) {
                $sampleIncomplete = $true
                break
            }

            $cimCreatedUtc = Convert-CimCreationDateToUtc -Value $cimProcess.CreationDate
            if ([Math]::Abs(($processStartUtc - $cimCreatedUtc).TotalSeconds) -gt 2) {
                $sampleIncomplete = $true
                break
            }

            $fileVersion = $null
            try {
                $fileVersion = $process.MainModule.FileVersionInfo.FileVersion
            } catch {
                $fileVersion = $null
            }

            $processes += [ordered]@{
                pid = $processId
                parent_pid = [int]$cimProcess.ParentProcessId
                name = $process.ProcessName
                role = Get-ProcessRole -ProcessId $processId -RootProcessId $RootProcessId -Name $process.ProcessName -CommandLine ([string]$cimProcess.CommandLine)
                executable_path = [string]$cimProcess.ExecutablePath
                file_version = $fileVersion
                started_at_utc = $processStartUtc.ToString("o")
                working_set_bytes = [int64]$process.WorkingSet64
                private_working_set_bytes = [int64]$performanceProcess.WorkingSetPrivate
                commit_bytes = [int64]$process.PrivateMemorySize64
            }
        }

        if (-not $sampleIncomplete -and $processes.Count -gt 0) {
            return [ordered]@{
                captured_at_utc = [DateTime]::UtcNow.ToString("o")
                process_count = $processes.Count
                working_set_bytes = [int64](($processes | Measure-Object -Property working_set_bytes -Sum).Sum)
                private_working_set_bytes = [int64](($processes | Measure-Object -Property private_working_set_bytes -Sum).Sum)
                commit_bytes = [int64](($processes | Measure-Object -Property commit_bytes -Sum).Sum)
                processes = $processes
            }
        }
        Start-Sleep -Milliseconds 200
    }

    throw "Could not obtain a stable process/performance-counter snapshot after 5 attempts."
}

function Stop-StartedProcess {
    param(
        [System.Diagnostics.Process]$RootProcess,
        [DateTime]$ExpectedStartUtc
    )

    $startedProcesses = @()
    try {
        $allProcesses = @(Get-CimInstance Win32_Process)
        foreach ($processId in @(
            Get-ProcessTreeIds -RootProcessId $RootProcess.Id -AllProcesses $allProcesses
        )) {
            try {
                $process = Get-Process -Id $processId -ErrorAction Stop
                $startedProcesses += [ordered]@{
                    pid = $processId
                    started_at_utc = $process.StartTime.ToUniversalTime()
                }
            } catch {
                # The process disappeared while the cleanup snapshot was taken.
            }
        }
    } catch {
        # Root-only cleanup below is still protected by its recorded start time.
    }

    try {
        $liveProcess = Get-Process -Id $RootProcess.Id -ErrorAction Stop
        if ([Math]::Abs(($liveProcess.StartTime.ToUniversalTime() - $ExpectedStartUtc).TotalSeconds) -le 2) {
            $liveProcess.Kill()
            [void]$liveProcess.WaitForExit(5000)
        }
    } catch {
        # The launched process already exited or cannot be inspected. Never kill
        # another process solely because it reused the original PID.
    }

    foreach ($identity in @($startedProcesses | Sort-Object pid -Descending)) {
        if ([int]$identity.pid -eq $RootProcess.Id) {
            continue
        }
        try {
            $liveProcess = Get-Process -Id ([int]$identity.pid) -ErrorAction Stop
            if ([Math]::Abs(
                ($liveProcess.StartTime.ToUniversalTime() - [DateTime]$identity.started_at_utc).TotalSeconds
            ) -le 2) {
                $liveProcess.Kill()
                [void]$liveProcess.WaitForExit(5000)
            }
        } catch {
            # The descendant exited with the root or its PID is no longer inspectable.
        }
    }
}

$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
$outputPath = [IO.Path]::GetFullPath($Output)
$outputDirectory = Split-Path -Parent $outputPath
if (-not [string]::IsNullOrEmpty($outputDirectory)) {
    [IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
}

$virtualKeys = Convert-ChordToVirtualKeys -Chord $StateTriggerChord
$wakeVirtualKeys = Convert-ChordToVirtualKeys -Chord $WakeTriggerChord
if ($SampleState -ne "visible" -and $virtualKeys.Count -eq 0) {
    Write-Warning "No trigger chord was supplied. The application must reach '$SampleState' autonomously."
}
if ($MeasureWake -and $SampleState -eq "visible") {
    throw "-MeasureWake requires -SampleState hidden or destroyed."
}
if ($MeasureWake -and $wakeVirtualKeys.Count -eq 0) {
    throw "-MeasureWake requires -WakeTriggerChord."
}

$operatingSystem = Get-CimInstance Win32_OperatingSystem
$computerSystem = Get-CimInstance Win32_ComputerSystem
$processor = @(Get-CimInstance Win32_Processor | Select-Object -ExpandProperty Name)
$results = @()

for ($run = 1; $run -le $Runs; $run++) {
    $rootProcess = $null
    $rootProcessStartUtc = $null
    try {
        $launchTimer = [System.Diagnostics.Stopwatch]::StartNew()
        $startParameters = @{
            FilePath = $resolvedExecutable
            PassThru = $true
            WorkingDirectory = Split-Path -Parent $resolvedExecutable
        }
        if ($ExecutableArgument.Count -gt 0) {
            $startParameters.ArgumentList = $ExecutableArgument
        }
        $rootProcess = Start-Process @startParameters
        $rootProcessStartUtc = $rootProcess.StartTime.ToUniversalTime()

        $visibleWindows = @()
        $windowTimer = [System.Diagnostics.Stopwatch]::StartNew()
        while ($windowTimer.Elapsed.TotalSeconds -lt $WindowTimeoutSeconds) {
            $rootProcess.Refresh()
            if ($rootProcess.HasExited) {
                throw "Root process exited before creating its main window. Exit code: $($rootProcess.ExitCode)"
            }
            $treeIds = @(Get-ProcessTreeIds -RootProcessId $rootProcess.Id)
            $visibleWindows = @(
                Get-MatchingWindows -ProcessIds $treeIds -ExactTitle $WindowTitle |
                    Where-Object Visible
            )
            if ($visibleWindows.Count -gt 0) {
                break
            }
            Start-Sleep -Milliseconds 50
        }
        if ($visibleWindows.Count -eq 0) {
            $observedTitles = @(
                [BenchmarkNative]::GetTopLevelWindows() |
                    Where-Object { $_.ProcessId -in $treeIds -and -not [string]::IsNullOrEmpty($_.Title) } |
                    ForEach-Object Title
            )
            throw "Timed out waiting for exact-title window '$WindowTitle'. Observed process-tree titles: $($observedTitles -join ', ')"
        }
        $windowMilliseconds = [Math]::Round($launchTimer.Elapsed.TotalMilliseconds, 1)
        $window = $visibleWindows[0]

        $triggerMilliseconds = $null
        if ($SampleState -ne "visible") {
            if ($StateTriggerDelaySeconds -gt 0) {
                Start-Sleep -Milliseconds ([int]($StateTriggerDelaySeconds * 1000))
            }
            if ($virtualKeys.Count -gt 0) {
                if (-not [BenchmarkNative]::FocusWindow($window.Handle, 2000)) {
                    Write-Warning "Could not focus the tracked window before sending '$StateTriggerChord'; continuing because global shortcuts do not require focus."
                }
                $triggerMilliseconds = $launchTimer.Elapsed.TotalMilliseconds
                for ($triggerIndex = 0; $triggerIndex -lt $StateTriggerCount; $triggerIndex++) {
                    [BenchmarkNative]::PressChord($virtualKeys)
                    if ($triggerIndex + 1 -lt $StateTriggerCount) {
                        Start-Sleep -Milliseconds ([int]($StateTriggerIntervalSeconds * 1000))
                    }
                }
            }
            Wait-ForWindowState -RootProcess $rootProcess -WindowHandle $window.Handle -ExpectedState $SampleState -WindowTitle $WindowTitle -TimeoutSeconds $StateTimeoutSeconds
        }
        $stateMilliseconds = [Math]::Round($launchTimer.Elapsed.TotalMilliseconds, 1)
        $triggerToStateMilliseconds = if ($null -ne $triggerMilliseconds) {
            [Math]::Round($stateMilliseconds - $triggerMilliseconds, 1)
        } else {
            $null
        }

        if ($WarmupSeconds -gt 0) {
            Start-Sleep -Milliseconds ([int]($WarmupSeconds * 1000))
        }

        $samples = @()
        for ($sampleIndex = 0; $sampleIndex -lt $SamplesPerRun; $sampleIndex++) {
            if (-not (Test-TrackedWindowState -RootProcessId $rootProcess.Id -WindowHandle $window.Handle -ExpectedState $SampleState -WindowTitle $WindowTitle)) {
                throw "Window left expected state '$SampleState' before sample $($sampleIndex + 1)."
            }
            $samples += Get-ProcessTreeSample -RootProcessId $rootProcess.Id
            if ($sampleIndex + 1 -lt $SamplesPerRun) {
                Start-Sleep -Milliseconds ([int]($SampleIntervalSeconds * 1000))
            }
        }

        $wakeMilliseconds = $null
        if ($MeasureWake) {
            $wakeTimer = [System.Diagnostics.Stopwatch]::StartNew()
            [BenchmarkNative]::PressChord($wakeVirtualKeys)
            $newWindow = $null
            if ($SampleState -eq "hidden") {
                Wait-ForWindowState -RootProcess $rootProcess -WindowHandle $window.Handle -ExpectedState "visible" -WindowTitle $WindowTitle -TimeoutSeconds $StateTimeoutSeconds
                $newWindow = $window
            } else {
                while ($wakeTimer.Elapsed.TotalSeconds -lt $StateTimeoutSeconds) {
                    $treeIds = @(Get-ProcessTreeIds -RootProcessId $rootProcess.Id)
                    $replacementWindows = @(
                        Get-MatchingWindows -ProcessIds $treeIds -ExactTitle $WindowTitle |
                            Where-Object Visible
                    )
                    if ($replacementWindows.Count -gt 0) {
                        $newWindow = $replacementWindows[0]
                        break
                    }
                    Start-Sleep -Milliseconds 50
                }
                if ($null -eq $newWindow) {
                    throw "Timed out waiting for a replacement window after wake chord '$WakeTriggerChord'."
                }
            }
            $wakeMilliseconds = [Math]::Round($wakeTimer.Elapsed.TotalMilliseconds, 1)
        }

        $runResult = [ordered]@{
            run = $run
            root_pid = $rootProcess.Id
            window_handle = ('0x{0:X}' -f $window.Handle.ToInt64())
            window_pid = [int]$window.ProcessId
            window_ms = $windowMilliseconds
            state_ms = $stateMilliseconds
            trigger_to_state_ms = $triggerToStateMilliseconds
            wake_ms = $wakeMilliseconds
            median_process_count = Get-Median -Values @($samples | ForEach-Object { [double]$_.process_count })
            median_working_set_bytes = [int64](Get-Median -Values @($samples | ForEach-Object { [double]$_.working_set_bytes }))
            median_private_working_set_bytes = [int64](Get-Median -Values @($samples | ForEach-Object { [double]$_.private_working_set_bytes }))
            median_commit_bytes = [int64](Get-Median -Values @($samples | ForEach-Object { [double]$_.commit_bytes }))
            samples = $samples
        }
        $results += $runResult

        $progress = [ordered]@{
            current = $run
            total = $Runs
            unit = "runs"
            percent = [Math]::Round(($run / $Runs) * 100, 1)
            message = $Label
        } | ConvertTo-Json -Compress
        Write-Host "JCODE_PROGRESS $progress"
    } finally {
        if ($null -ne $rootProcess -and $null -ne $rootProcessStartUtc) {
            Stop-StartedProcess -RootProcess $rootProcess -ExpectedStartUtc $rootProcessStartUtc
            Start-Sleep -Milliseconds 500
        }
    }
}

$document = [ordered]@{
    schema_version = 1
    label = $Label
    captured_at_utc = [DateTime]::UtcNow.ToString("o")
    executable = $resolvedExecutable
    executable_sha256 = (Get-FileHash -LiteralPath $resolvedExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    executable_arguments = $ExecutableArgument
    window_title = $WindowTitle
    sample_state = $SampleState
    state_trigger = [ordered]@{
        chord = if ([string]::IsNullOrWhiteSpace($StateTriggerChord)) { $null } else { $StateTriggerChord }
        count = $StateTriggerCount
        delay_seconds = $StateTriggerDelaySeconds
        interval_seconds = $StateTriggerIntervalSeconds
    }
    wake = [ordered]@{
        measured = [bool]$MeasureWake
        chord = if ([string]::IsNullOrWhiteSpace($WakeTriggerChord)) { $null } else { $WakeTriggerChord }
    }
    protocol = [ordered]@{
        runs = $Runs
        window_timeout_seconds = $WindowTimeoutSeconds
        state_timeout_seconds = $StateTimeoutSeconds
        warmup_seconds = $WarmupSeconds
        samples_per_run = $SamplesPerRun
        sample_interval_seconds = $SampleIntervalSeconds
        process_scope = "root process plus recursively discovered descendants"
        private_working_set_source = "Win32_PerfFormattedData_PerfProc_Process.WorkingSetPrivate"
        commit_source = "System.Diagnostics.Process.PrivateMemorySize64"
    }
    host = [ordered]@{
        os_caption = [string]$operatingSystem.Caption
        os_version = [string]$operatingSystem.Version
        os_build = [string]$operatingSystem.BuildNumber
        architecture = [string]$operatingSystem.OSArchitecture
        physical_memory_bytes = [int64]$computerSystem.TotalPhysicalMemory
        processors = $processor
        powershell = [string]$PSVersionTable.PSVersion
    }
    results = $results
    summary = [ordered]@{
        median_window_ms = [Math]::Round((Get-Median -Values @($results | ForEach-Object { [double]$_.window_ms })), 1)
        median_state_ms = [Math]::Round((Get-Median -Values @($results | ForEach-Object { [double]$_.state_ms })), 1)
        median_trigger_to_state_ms = if (@($results | Where-Object { $null -ne $_.trigger_to_state_ms }).Count -gt 0) {
            [Math]::Round((Get-Median -Values @($results | Where-Object { $null -ne $_.trigger_to_state_ms } | ForEach-Object { [double]$_.trigger_to_state_ms })), 1)
        } else {
            $null
        }
        median_wake_ms = if (@($results | Where-Object { $null -ne $_.wake_ms }).Count -gt 0) {
            [Math]::Round((Get-Median -Values @($results | Where-Object { $null -ne $_.wake_ms } | ForEach-Object { [double]$_.wake_ms })), 1)
        } else {
            $null
        }
        median_process_count = Get-Median -Values @($results | ForEach-Object { [double]$_.median_process_count })
        median_working_set_bytes = [int64](Get-Median -Values @($results | ForEach-Object { [double]$_.median_working_set_bytes }))
        median_private_working_set_bytes = [int64](Get-Median -Values @($results | ForEach-Object { [double]$_.median_private_working_set_bytes }))
        median_commit_bytes = [int64](Get-Median -Values @($results | ForEach-Object { [double]$_.median_commit_bytes }))
    }
}

$json = $document | ConvertTo-Json -Depth 20 -WarningAction Stop
$json | Set-Content -LiteralPath $outputPath -Encoding utf8
$json
