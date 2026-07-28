[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Label,

    [Parameter(Mandatory = $true)]
    [string]$Executable,

    [string[]]$ExecutableArgument = @(),

    [string]$MainWindowTitle = "TieZ",

    [ValidateSet("hidden", "destroyed")]
    [string]$Mode = "destroyed",

    [ValidateRange(100, 1000)]
    [int]$Cycles = 100,

    [ValidateRange(1, 300)]
    [int]$WindowTimeoutSeconds = 30,

    [ValidateRange(1, 300)]
    [int]$PhaseTimeoutSeconds = 30,

    [ValidateRange(1, 300)]
    [int]$ReadyTimeoutSeconds = 30,

    [ValidateRange(30, 30)]
    [int]$MemorySettleSeconds = 30,

    [ValidateRange(5, 5)]
    [int]$FastMemorySettleSeconds = 5,

    [ValidateScript({ $_ -ge 5 -and $_ -le 99 -and ($_ % 2) -eq 1 })]
    [int]$MemoryRuns = 5,

    [ValidateRange(1, 10000)]
    [int]$PollIntervalMilliseconds = 100,

    [ValidateRange(1, 60000)]
    [int]$CycleDelayMilliseconds = 250,

    [ValidateRange(1, 750)]
    [int]$SearchReadyMedianThresholdMilliseconds = 750,

    [ValidateRange(1, 1500)]
    [int]$SearchReadyWorstFiveThresholdMilliseconds = 1500,

    [ValidateRange(40, 100)]
    [int]$MemoryReductionPercentThreshold = 40,

    [ValidateRange(50, 1048576)]
    [int]$MemoryReductionMiBThreshold = 50,

    [Parameter(Mandatory = $true)]
    [string]$Output
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "measure_lifecycle_windows.ps1 must run on Windows 11 or another Windows host with WebView2. Linux is screening-only."
}

if ($MemorySettleSeconds -lt $FastMemorySettleSeconds) {
    throw "MemorySettleSeconds must be greater than or equal to FastMemorySettleSeconds."
}

if ($null -eq ("Issue154LifecycleNative" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public sealed class Issue154WindowInfo
{
    public IntPtr Handle { get; set; }
    public uint ProcessId { get; set; }
    public string Title { get; set; }
    public bool Visible { get; set; }
}

public static class Issue154LifecycleNative
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

    [DllImport("shell32.dll", SetLastError = true)]
    private static extern IntPtr CommandLineToArgvW(
        [MarshalAs(UnmanagedType.LPWStr)] string commandLine,
        out int argumentCount);

    [DllImport("kernel32.dll")]
    private static extern IntPtr LocalFree(IntPtr memory);

    public static string[] SplitCommandLine(string commandLine)
    {
        if (String.IsNullOrWhiteSpace(commandLine))
        {
            return new string[0];
        }

        int argumentCount;
        IntPtr arguments = CommandLineToArgvW(commandLine, out argumentCount);
        if (arguments == IntPtr.Zero)
        {
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        }

        try
        {
            var result = new string[argumentCount];
            for (int index = 0; index < argumentCount; index++)
            {
                IntPtr value = Marshal.ReadIntPtr(arguments, index * IntPtr.Size);
                result[index] = Marshal.PtrToStringUni(value);
            }
            return result;
        }
        finally
        {
            LocalFree(arguments);
        }
    }

    public static Issue154WindowInfo[] GetTopLevelWindows()
    {
        var windows = new List<Issue154WindowInfo>();
        EnumWindows(delegate(IntPtr handle, IntPtr ignored)
        {
            int length = GetWindowTextLength(handle);
            var title = new StringBuilder(length + 1);
            GetWindowText(handle, title, title.Capacity);
            uint processId;
            GetWindowThreadProcessId(handle, out processId);
            windows.Add(new Issue154WindowInfo
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
}
"@
}

function ConvertTo-Issue154NormalizedPath {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        return $null
    }
    $expandedPath = [Environment]::ExpandEnvironmentVariables($Path)
    if (-not [IO.Path]::IsPathRooted($expandedPath)) {
        return $null
    }
    $fullPath = [IO.Path]::GetFullPath($expandedPath)
    $root = [IO.Path]::GetPathRoot($fullPath)
    if ([string]::Equals($fullPath, $root, [StringComparison]::OrdinalIgnoreCase)) {
        return $root
    }
    return $fullPath.TrimEnd([char[]]@('\', '/'))
}

function Get-Issue154WebView2UserDataFolder {
    param([string]$CommandLine)

    $arguments = @([Issue154LifecycleNative]::SplitCommandLine($CommandLine))
    for ($index = 0; $index -lt $arguments.Count; $index++) {
        $argument = [string]$arguments[$index]
        if ([string]::Equals($argument, "--user-data-dir", [StringComparison]::OrdinalIgnoreCase)) {
            if ($index + 1 -ge $arguments.Count) {
                return $null
            }
            return ConvertTo-Issue154NormalizedPath -Path ([string]$arguments[$index + 1])
        }
        if ($argument.StartsWith("--user-data-dir=", [StringComparison]::OrdinalIgnoreCase)) {
            $prefix = "--user-data-dir="
            return ConvertTo-Issue154NormalizedPath -Path $argument.Substring($prefix.Length)
        }
    }
    return $null
}

function Get-Issue154Median {
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

function Get-Issue154LockedFileSha256 {
    param([System.IO.FileStream]$Stream)

    $Stream.Position = 0
    return (Get-FileHash -InputStream $Stream -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-Issue154ProcessTreeIds {
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

function Get-Issue154ProcessRole {
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

function Convert-Issue154CimCreationDateToUtc {
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
        $styles = [Globalization.DateTimeStyles]::AssumeUniversal -bor [Globalization.DateTimeStyles]::AdjustToUniversal
        return [DateTime]::Parse($text, [Globalization.CultureInfo]::InvariantCulture, $styles).ToUniversalTime()
    }
}

function Get-Issue154ProcessTreeSample {
    param(
        [int]$RootProcessId,
        [string[]]$ExpectedWebView2UserDataFolders = @(),
        [string]$ExpectedRootExecutablePath,
        [string]$ExpectedRootStartedAtUtc,
        [string[]]$ReferenceProcessIdentityKeys = @()
    )

    for ($attempt = 1; $attempt -le 5; $attempt++) {
        $cimProcesses = @(Get-CimInstance Win32_Process)
        $cimById = @{}
        foreach ($cimProcess in $cimProcesses) {
            $cimById[[int]$cimProcess.ProcessId] = $cimProcess
        }
        $performanceById = @{}
        foreach ($performanceProcess in @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process)) {
            $performanceById[[int]$performanceProcess.IDProcess] = $performanceProcess
        }
        $treeIds = @(Get-Issue154ProcessTreeIds -RootProcessId $RootProcessId -AllProcesses $cimProcesses)
        $treeIdSet = [System.Collections.Generic.HashSet[int]]::new()
        foreach ($treeId in $treeIds) {
            [void]$treeIdSet.Add([int]$treeId)
        }
        $discoveredWebView2UserDataFolders = @(
            $treeIds |
                ForEach-Object { Get-Issue154WebView2UserDataFolder -CommandLine ([string]$cimById[[int]$_].CommandLine) } |
                Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
                Sort-Object -Unique
        )
        $treeWebView2UserDataFolders = if ($ExpectedWebView2UserDataFolders.Count -gt 0) {
            @(
                $ExpectedWebView2UserDataFolders |
                    ForEach-Object { ConvertTo-Issue154NormalizedPath -Path $_ } |
                    Sort-Object -Unique
            )
        } else {
            $discoveredWebView2UserDataFolders
        }
        $attributedIds = [System.Collections.Generic.HashSet[int]]::new()
        foreach ($treeId in $treeIds) {
            [void]$attributedIds.Add([int]$treeId)
        }
        foreach ($cimProcess in $cimProcesses) {
            if ([string]$cimProcess.Name -notmatch '^msedgewebview2\.exe$') {
                continue
            }
            $candidateFolder = Get-Issue154WebView2UserDataFolder -CommandLine ([string]$cimProcess.CommandLine)
            if ([string]::IsNullOrWhiteSpace($candidateFolder)) {
                continue
            }
            foreach ($userDataFolder in $treeWebView2UserDataFolders) {
                if ([string]::Equals($candidateFolder, $userDataFolder, [StringComparison]::OrdinalIgnoreCase)) {
                    [void]$attributedIds.Add([int]$cimProcess.ProcessId)
                    break
                }
            }
        }
        $treeIds = @($attributedIds | Sort-Object)
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
            $cimCreatedUtc = Convert-Issue154CimCreationDateToUtc -Value $cimProcess.CreationDate
            if ([Math]::Abs(($processStartUtc - $cimCreatedUtc).TotalSeconds) -gt 2) {
                $sampleIncomplete = $true
                break
            }
            if ($processId -eq $RootProcessId) {
                $rootExecutablePath = ConvertTo-Issue154NormalizedPath -Path ([string]$cimProcess.ExecutablePath)
                $expectedRootPath = ConvertTo-Issue154NormalizedPath -Path $ExpectedRootExecutablePath
                if (-not [string]::IsNullOrWhiteSpace($expectedRootPath) -and
                    -not [string]::Equals($rootExecutablePath, $expectedRootPath, [StringComparison]::OrdinalIgnoreCase)) {
                    $sampleIncomplete = $true
                    break
                }
                if (-not [string]::IsNullOrWhiteSpace($ExpectedRootStartedAtUtc)) {
                    $expectedRootStart = [DateTime]::Parse(
                        $ExpectedRootStartedAtUtc,
                        [Globalization.CultureInfo]::InvariantCulture,
                        [Globalization.DateTimeStyles]::AssumeUniversal -bor [Globalization.DateTimeStyles]::AdjustToUniversal)
                    if ([Math]::Abs(($processStartUtc - $expectedRootStart).TotalSeconds) -gt 2) {
                        $sampleIncomplete = $true
                        break
                    }
                }
            }
            $processes += [ordered]@{
                pid = $processId
                parent_pid = [int]$cimProcess.ParentProcessId
                name = $process.ProcessName
                role = Get-Issue154ProcessRole -ProcessId $processId -RootProcessId $RootProcessId -Name $process.ProcessName -CommandLine ([string]$cimProcess.CommandLine)
                attribution = if ($treeIdSet.Contains($processId)) { "descendant" } else { "webview2_user_data_dir" }
                executable_path = [string]$cimProcess.ExecutablePath
                started_at_utc = $processStartUtc.ToString("o")
                working_set_bytes = [int64]$process.WorkingSet64
                private_working_set_bytes = [int64]$performanceProcess.WorkingSetPrivate
                commit_bytes = [int64]$process.PrivateMemorySize64
            }
        }

        if (-not $sampleIncomplete -and $processes.Count -gt 0) {
            $processIdentityKeys = @(
                $processes |
                    ForEach-Object { "{0}|{1}|{2}" -f $_.role, $_.executable_path, $_.started_at_utc } |
                    Sort-Object -Unique
            )
            $referenceProcessIdentityKeys = if ($ReferenceProcessIdentityKeys.Count -gt 0) {
                @($ReferenceProcessIdentityKeys | Sort-Object -Unique)
            } else {
                $processIdentityKeys
            }
            $identitiesAddedFromReference = @(
                $processIdentityKeys |
                    Where-Object { $referenceProcessIdentityKeys -cnotcontains $_ }
            )
            $identitiesMissingFromReference = @(
                $referenceProcessIdentityKeys |
                    Where-Object { $processIdentityKeys -cnotcontains $_ }
            )
            return [ordered]@{
                captured_at_utc = [DateTime]::UtcNow.ToString("o")
                process_count = $processes.Count
                descendant_process_count = @($processes | Where-Object { $_.attribution -eq "descendant" }).Count
                attributed_webview2_process_count = @($processes | Where-Object { $_.attribution -eq "webview2_user_data_dir" }).Count
                webview2_user_data_folders = $treeWebView2UserDataFolders
                process_identity_keys = $processIdentityKeys
                reference_process_identity_keys = $referenceProcessIdentityKeys
                identities_added_from_reference = $identitiesAddedFromReference
                identities_missing_from_reference = $identitiesMissingFromReference
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

function Get-Issue154MainWindowCount {
    param(
        [int]$RootProcessId,
        [string]$ExactTitle
    )
    $treeIds = @(Get-Issue154ProcessTreeIds -RootProcessId $RootProcessId)
    $idSet = [System.Collections.Generic.HashSet[uint32]]::new()
    foreach ($processId in $treeIds) {
        [void]$idSet.Add([uint32]$processId)
    }
    $matches = @(
        [Issue154LifecycleNative]::GetTopLevelWindows() |
            Where-Object { $idSet.Contains($_.ProcessId) -and $_.Title -ceq $ExactTitle -and $_.Visible }
    )
    return $matches.Count
}

function Invoke-Issue154LifecycleCommand {
    param(
        [string]$CommandName,
        [hashtable]$Payload,
        [int]$TimeoutSeconds = 30
    )

    $harnessDirectory = [Environment]::GetEnvironmentVariable("TIEZ_INTERNAL_LIFECYCLE_HARNESS_DIR", "Process")
    if ([string]::IsNullOrWhiteSpace($harnessDirectory)) {
        throw "TIEZ_INTERNAL_LIFECYCLE_HARNESS_DIR is not set for the current process."
    }
    if (-not [IO.Directory]::Exists($harnessDirectory)) {
        throw "TIEZ_INTERNAL_LIFECYCLE_HARNESS_DIR does not exist: $harnessDirectory"
    }

    $requestId = "req_{0}_{1}_{2}" -f $PID, [DateTime]::UtcNow.Ticks, ([Guid]::NewGuid().ToString("N"))
    $requestPath = Join-Path $harnessDirectory ("{0}.request.json" -f $requestId)
    $responsePath = Join-Path $harnessDirectory ("{0}.response.json" -f $requestId)
    $tmpRequestPath = Join-Path $harnessDirectory (".{0}.request.json.{1}.tmp" -f $requestId, $PID)
    $body = [ordered]@{
        id = $requestId
        command = $CommandName
        payload = $Payload
    } | ConvertTo-Json -Depth 20

    $utf8NoBom = New-Object System.Text.UTF8Encoding $false
    [IO.File]::WriteAllText($tmpRequestPath, $body, $utf8NoBom)
    [IO.File]::Move($tmpRequestPath, $requestPath)

    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        if ([IO.File]::Exists($responsePath)) {
            try {
                $content = [IO.File]::ReadAllText($responsePath, [System.Text.Encoding]::UTF8)
                $response = $content | ConvertFrom-Json
            } catch {
                Start-Sleep -Milliseconds $PollIntervalMilliseconds
                continue
            }

            if ($response.id -cne $requestId) {
                throw "Lifecycle harness response id mismatch. Expected '$requestId', got '$($response.id)'."
            }
            if (-not [bool]$response.success) {
                throw "Lifecycle harness command '$CommandName' failed: $($response.error)"
            }
            try {
                Remove-Item -LiteralPath $responsePath -Force -ErrorAction SilentlyContinue
            } catch {
            }
            return $response.payload
        }
        Start-Sleep -Milliseconds $PollIntervalMilliseconds
    }

    Remove-Item -LiteralPath $tmpRequestPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $requestPath -Force -ErrorAction SilentlyContinue
    throw "Timed out waiting for lifecycle harness response to '$CommandName' request '$requestId'."
}

function Get-Issue154PropertyValue {
    param(
        [object]$Object,
        [string[]]$Names
    )

    if ($null -eq $Object) {
        return $null
    }
    foreach ($name in $Names) {
        if ($Object -is [System.Collections.IDictionary] -and $Object.Contains($name)) {
            return $Object[$name]
        }
        $property = $Object.PSObject.Properties[$name]
        if ($null -ne $property) {
            return $property.Value
        }
    }
    return $null
}

function Test-Issue154PositiveValue {
    param([object]$Value)

    if ($null -eq $Value) {
        return $false
    }
    if ($Value -is [bool]) {
        return [bool]$Value
    }
    try {
        return ([double]$Value) -gt 0
    } catch {
        return $false
    }
}

function Test-Issue154ClipboardProbePayload {
    param(
        [object]$Payload,
        [string]$Token
    )

    $eventValue = Get-Issue154PropertyValue -Object $Payload -Names @(
        "listener_event_count_increased",
        "listener_event_increased",
        "event_count_increased",
        "clipboard_listener_event_increased",
        "listener_events_added",
        "clipboard_listener_events_added",
        "listener_event_delta",
        "clipboard_listener_event_delta",
        "event_delta"
    )
    $eventIncreased = Test-Issue154PositiveValue -Value $eventValue

    $exactMatch = [bool](Get-Issue154PropertyValue -Object $Payload -Names @("exact_history_match"))
    $exactMatchCount = Get-Issue154PropertyValue -Object $Payload -Names @("exact_history_match_count")
    $persistentId = Get-Issue154PropertyValue -Object $Payload -Names @("persisted_entry_id")
    $sessionId = Get-Issue154PropertyValue -Object $Payload -Names @("session_entry_id")
    $tokenEchoMatches = ([string](Get-Issue154PropertyValue -Object $Payload -Names @("token"))) -ceq $Token
    $persistentExact = $null -ne $persistentId
    $sessionExact = $null -ne $sessionId
    $uniqueHistoryMatch = $exactMatch -and ([int]$exactMatchCount -eq 1)

    return [ordered]@{
        ok = [bool]($eventIncreased -and $tokenEchoMatches -and $uniqueHistoryMatch)
        listener_event_increased = [bool]$eventIncreased
        persistent_history_exact_match = [bool]$persistentExact
        session_history_exact_match = [bool]$sessionExact
        exact_history_match_count = [int]$exactMatchCount
    }
}

function Wait-Issue154ClipboardProbe {
    param(
        [string]$Token,
        [int]$Cycle,
        [uint64]$ClipboardEventCountBefore,
        [int]$TimeoutSeconds
    )

    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $attempt = 0
    $lastPayload = $null
    $lastVerification = $null
    while ($timer.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        $attempt++
        $lastPayload = Invoke-Issue154LifecycleCommand -CommandName "get_main_ui_lifecycle_clipboard_probe" -Payload @{
            token = $Token
            cycle = $Cycle
            clipboard_event_count_before = $ClipboardEventCountBefore
        } -TimeoutSeconds $TimeoutSeconds
        $lastVerification = Test-Issue154ClipboardProbePayload -Payload $lastPayload -Token $Token
        if ([bool]$lastVerification.ok) {
            return [ordered]@{
                attempts = $attempt
                elapsed_ms = [Math]::Round($timer.Elapsed.TotalMilliseconds, 1)
                verification = $lastVerification
                payload = $lastPayload
            }
        }
        Start-Sleep -Milliseconds $PollIntervalMilliseconds
    }

    throw "Timed out waiting for clipboard listener/history probe for token '$Token'. Last probe: $($lastPayload | ConvertTo-Json -Compress -Depth 20). Last verification: $($lastVerification | ConvertTo-Json -Compress -Depth 20)"
}

function Wait-Issue154LifecycleSnapshot {
    param(
        [int]$RootProcessId,
        [string]$ExpectedPhase,
        [Nullable[uint64]]$ExpectedGeneration = $null,
        [Nullable[uint64]]$ExpectedRequestId = $null,
        [switch]$InitialReady,
        [int]$TimeoutSeconds
    )

    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $lastSnapshot = $null
    while ($timer.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
        $lastSnapshot = Invoke-Issue154LifecycleCommand -CommandName "get_main_ui_lifecycle_snapshot" -Payload @{}
        if ($null -ne $ExpectedRequestId -and
            $null -ne $lastSnapshot.failed_request_id -and
            [uint64]$lastSnapshot.failed_request_id -eq [uint64]$ExpectedRequestId) {
            throw "Lifecycle request '$ExpectedRequestId' failed. Snapshot: $($lastSnapshot | ConvertTo-Json -Compress -Depth 20)"
        }
        $generationMatches = $null -eq $ExpectedGeneration -or [uint64]$lastSnapshot.generation -eq [uint64]$ExpectedGeneration
        $requestMatches = $null -eq $ExpectedRequestId -or [uint64]$lastSnapshot.completed_request_id -eq [uint64]$ExpectedRequestId
        if ($lastSnapshot.phase -eq $ExpectedPhase -and $generationMatches -and $requestMatches) {
            $visibleMainCount = Get-Issue154MainWindowCount -RootProcessId $RootProcessId -ExactTitle $MainWindowTitle
            $nativeStateMatches = switch ($ExpectedPhase) {
                "hidden" {
                    [bool]$lastSnapshot.main_window_present -and
                    -not [bool]$lastSnapshot.main_window_visible -and
                    -not [bool]$lastSnapshot.main_window_focused -and
                    $visibleMainCount -eq 0
                }
                "destroyed" {
                    -not [bool]$lastSnapshot.main_window_present -and
                    -not [bool]$lastSnapshot.main_window_visible -and
                    -not [bool]$lastSnapshot.main_window_focused -and
                    $visibleMainCount -eq 0
                }
                "ready" {
                    [bool]$lastSnapshot.main_window_present -and
                    [bool]$lastSnapshot.main_window_visible -and
                    ($InitialReady -or [bool]$lastSnapshot.main_window_focused) -and
                    $visibleMainCount -eq 1
                }
                default { $visibleMainCount -le 1 }
            }
            if ($nativeStateMatches) {
                return $lastSnapshot
            }
        }
        Start-Sleep -Milliseconds $PollIntervalMilliseconds
    }
    throw "Timed out waiting for lifecycle phase '$ExpectedPhase'. Last snapshot: $($lastSnapshot | ConvertTo-Json -Compress -Depth 20)"
}

function Stop-Issue154StartedProcess {
    param(
        [System.Diagnostics.Process]$RootProcess,
        [DateTime]$ExpectedStartUtc
    )

    try {
        $liveProcess = Get-Process -Id $RootProcess.Id -ErrorAction Stop
        if ([Math]::Abs(($liveProcess.StartTime.ToUniversalTime() - $ExpectedStartUtc).TotalSeconds) -le 2) {
            $liveProcess.Kill()
            [void]$liveProcess.WaitForExit(5000)
        }
    } catch {
        # Never kill another process solely because it reused the original PID.
    }
}

$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
$outputPath = [IO.Path]::GetFullPath($Output)
$outputDirectory = Split-Path -Parent $outputPath
if (-not [string]::IsNullOrEmpty($outputDirectory)) {
    [IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
}

$operatingSystem = Get-CimInstance Win32_OperatingSystem
$windowsBuild = [int]$operatingSystem.BuildNumber
if ([string]$operatingSystem.Caption -notmatch "Windows 11" -or $windowsBuild -lt 22000) {
    throw "Issue #154 Stage B requires Windows 11 build 22000 or newer. Found '$($operatingSystem.Caption)' build $windowsBuild."
}
$computerSystem = Get-CimInstance Win32_ComputerSystem
$processor = @(Get-CimInstance Win32_Processor | Select-Object -ExpandProperty Name)
$rootProcess = $null
$rootProcessStartUtc = $null
$cyclesResult = @()
$baselineSnapshot = $null
$baselineSample = $null
$memoryAfter5s = $null
$memoryAfter30s = $null
$memoryRunsResult = @()
$finalSnapshot = $null
$finalTraces = $null
do {
    $harnessDirectory = Join-Path ([IO.Path]::GetTempPath()) ("tiez-lifecycle-harness-{0}-{1}" -f $PID, ([Guid]::NewGuid().ToString("N")))
} while ([IO.Directory]::Exists($harnessDirectory))
$previousLifecycleMode = [Environment]::GetEnvironmentVariable("TIEZ_EXPERIMENT_MAIN_UI_LIFECYCLE", "Process")
$previousHarnessDirectory = [Environment]::GetEnvironmentVariable("TIEZ_INTERNAL_LIFECYCLE_HARNESS_DIR", "Process")
$executableLock = [IO.File]::Open(
    $resolvedExecutable,
    [IO.FileMode]::Open,
    [IO.FileAccess]::Read,
    [IO.FileShare]::Read)
$executableSha256BeforeStart = Get-Issue154LockedFileSha256 -Stream $executableLock
$executableSha256AfterStart = $null
$executableSha256AfterRun = $null

try {
    [IO.Directory]::CreateDirectory($harnessDirectory) | Out-Null
    [Environment]::SetEnvironmentVariable("TIEZ_EXPERIMENT_MAIN_UI_LIFECYCLE", $Mode, "Process")
    [Environment]::SetEnvironmentVariable("TIEZ_INTERNAL_LIFECYCLE_HARNESS_DIR", $harnessDirectory, "Process")

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
    $executableSha256AfterStart = Get-Issue154LockedFileSha256 -Stream $executableLock
    if ($executableSha256BeforeStart -cne $executableSha256AfterStart) {
        throw "Executable SHA-256 changed while the measured process was starting."
    }

    $baselineSnapshot = Wait-Issue154LifecycleSnapshot -RootProcessId $rootProcess.Id -ExpectedPhase "ready" -InitialReady -TimeoutSeconds $WindowTimeoutSeconds
    $baselineSample = Get-Issue154ProcessTreeSample `
        -RootProcessId $rootProcess.Id `
        -ExpectedRootExecutablePath $resolvedExecutable `
        -ExpectedRootStartedAtUtc ($rootProcessStartUtc.ToString("o"))
    $expectedWebView2UserDataFolders = @($baselineSample.webview2_user_data_folders)
    $referenceProcessIdentityKeys = @($baselineSample.process_identity_keys)
    if ($expectedWebView2UserDataFolders.Count -eq 0) {
        throw "Baseline process sample did not discover a WebView2 --user-data-dir for related-process attribution."
    }

    for ($cycle = 1; $cycle -le $Cycles; $cycle++) {
        $cycleTimer = [System.Diagnostics.Stopwatch]::StartNew()
        $hideResponse = Invoke-Issue154LifecycleCommand -CommandName "lifecycle_test_hide" -Payload @{
            mode = $Mode
            cycle = $cycle
        }
        $downSnapshot = Wait-Issue154LifecycleSnapshot -RootProcessId $rootProcess.Id -ExpectedPhase $Mode -ExpectedGeneration ([uint64]$hideResponse.generation_before) -ExpectedRequestId ([uint64]$hideResponse.request_id) -TimeoutSeconds $PhaseTimeoutSeconds
        $token = "tiez_issue154_{0}_{1}_{2}" -f $Mode, $cycle, ([Guid]::NewGuid().ToString("N"))
        Set-Clipboard -Value $token
        $clipboardProbe = Wait-Issue154ClipboardProbe -Token $token -Cycle $cycle -ClipboardEventCountBefore ([uint64]$downSnapshot.clipboard_event_count) -TimeoutSeconds $PhaseTimeoutSeconds
        $showTimer = [System.Diagnostics.Stopwatch]::StartNew()
        $showResponse = Invoke-Issue154LifecycleCommand -CommandName "lifecycle_test_show" -Payload @{
            mode = $Mode
            cycle = $cycle
        }
        $readySnapshot = Wait-Issue154LifecycleSnapshot -RootProcessId $rootProcess.Id -ExpectedPhase "ready" -ExpectedGeneration ([uint64]$showResponse.expected_generation) -ExpectedRequestId ([uint64]$showResponse.request_id) -TimeoutSeconds $ReadyTimeoutSeconds
        $showReadyMilliseconds = [Math]::Round($showTimer.Elapsed.TotalMilliseconds, 1)
        $cycleMilliseconds = [Math]::Round($cycleTimer.Elapsed.TotalMilliseconds, 1)
        if ($null -eq $readySnapshot.requested_visible_focused_hydrated_search_ready_ms) {
            throw "Cycle $cycle reached ready without an internal requested-visible-focused-hydrated-search-ready latency."
        }
        $searchReadyMilliseconds = [double]$readySnapshot.requested_visible_focused_hydrated_search_ready_ms
        $mainCount = Get-Issue154MainWindowCount -RootProcessId $rootProcess.Id -ExactTitle $MainWindowTitle
        $clipboardOk = [bool]$clipboardProbe.verification.ok
        $cyclesResult += [ordered]@{
            cycle = $cycle
            requested_mode = $Mode
            hide_response = $hideResponse
            down_phase = $downSnapshot.phase
            down_snapshot = $downSnapshot
            clipboard_token = $token
            clipboard_probe = $clipboardProbe
            show_response = $showResponse
            ready_phase = $readySnapshot.phase
            ready_snapshot = $readySnapshot
            main_window_count = $mainCount
            requested_visible_focused_hydrated_search_ready_ms = [Math]::Round($searchReadyMilliseconds, 1)
            show_to_ready_ms = $showReadyMilliseconds
            total_cycle_ms = $cycleMilliseconds
            clipboard_activity_history_consistent = $clipboardOk
            generation = $readySnapshot.generation
        }
        if ($mainCount -gt 1) {
            throw "Cycle $cycle observed more than one visible main window."
        }
        if (-not $clipboardOk) {
            throw "Cycle $cycle clipboard probe did not confirm listener event increase and exact history token match."
        }
        $progress = [ordered]@{
            current = $cycle
            total = $Cycles
            unit = "cycles"
            percent = [Math]::Round(($cycle / $Cycles) * 100, 1)
            message = $Label
        } | ConvertTo-Json -Compress
        Write-Host "JCODE_PROGRESS $progress"
        Start-Sleep -Milliseconds $CycleDelayMilliseconds
    }

    for ($run = 1; $run -le $MemoryRuns; $run++) {
        $hideResponse = Invoke-Issue154LifecycleCommand -CommandName "lifecycle_test_hide" -Payload @{
            mode = $Mode
            memory_run = $run
        }
        $downSnapshot = Wait-Issue154LifecycleSnapshot -RootProcessId $rootProcess.Id -ExpectedPhase $Mode -ExpectedGeneration ([uint64]$hideResponse.generation_before) -ExpectedRequestId ([uint64]$hideResponse.request_id) -TimeoutSeconds $PhaseTimeoutSeconds
        Start-Sleep -Seconds $FastMemorySettleSeconds
        $sample5s = Get-Issue154ProcessTreeSample `
            -RootProcessId $rootProcess.Id `
            -ExpectedWebView2UserDataFolders $expectedWebView2UserDataFolders `
            -ExpectedRootExecutablePath $resolvedExecutable `
            -ExpectedRootStartedAtUtc ($rootProcessStartUtc.ToString("o")) `
            -ReferenceProcessIdentityKeys $referenceProcessIdentityKeys
        $remainingSeconds = $MemorySettleSeconds - $FastMemorySettleSeconds
        if ($remainingSeconds -gt 0) {
            Start-Sleep -Seconds $remainingSeconds
        }
        $sample30s = Get-Issue154ProcessTreeSample `
            -RootProcessId $rootProcess.Id `
            -ExpectedWebView2UserDataFolders $expectedWebView2UserDataFolders `
            -ExpectedRootExecutablePath $resolvedExecutable `
            -ExpectedRootStartedAtUtc ($rootProcessStartUtc.ToString("o")) `
            -ReferenceProcessIdentityKeys $referenceProcessIdentityKeys
        $showTimer = [System.Diagnostics.Stopwatch]::StartNew()
        $showResponse = Invoke-Issue154LifecycleCommand -CommandName "lifecycle_test_show" -Payload @{
            mode = $Mode
            memory_run = $run
        }
        $readySnapshot = Wait-Issue154LifecycleSnapshot -RootProcessId $rootProcess.Id -ExpectedPhase "ready" -ExpectedGeneration ([uint64]$showResponse.expected_generation) -ExpectedRequestId ([uint64]$showResponse.request_id) -TimeoutSeconds $ReadyTimeoutSeconds
        $memoryRunsResult += [ordered]@{
            run = $run
            requested_mode = $Mode
            hide_response = $hideResponse
            down_phase = $downSnapshot.phase
            down_snapshot = $downSnapshot
            sample_after_5s = $sample5s
            sample_after_30s = $sample30s
            show_response = $showResponse
            ready_phase = $readySnapshot.phase
            ready_snapshot = $readySnapshot
            show_to_ready_ms = [Math]::Round($showTimer.Elapsed.TotalMilliseconds, 1)
        }
        $progress = [ordered]@{
            current = $run
            total = $MemoryRuns
            unit = "memory_runs"
            percent = [Math]::Round(($run / $MemoryRuns) * 100, 1)
            message = $Label
        } | ConvertTo-Json -Compress
        Write-Host "JCODE_PROGRESS $progress"
    }
    $memoryAfter5s = $memoryRunsResult[-1].sample_after_5s
    $memoryAfter30s = $memoryRunsResult[-1].sample_after_30s
    $finalSnapshot = Invoke-Issue154LifecycleCommand -CommandName "get_main_ui_lifecycle_snapshot" -Payload @{}
    $finalTraces = Invoke-Issue154LifecycleCommand -CommandName "get_main_ui_lifecycle_traces" -Payload @{}
} finally {
    if ($null -ne $rootProcess -and $null -ne $rootProcessStartUtc) {
        Stop-Issue154StartedProcess -RootProcess $rootProcess -ExpectedStartUtc $rootProcessStartUtc
    }
    [Environment]::SetEnvironmentVariable("TIEZ_EXPERIMENT_MAIN_UI_LIFECYCLE", $previousLifecycleMode, "Process")
    [Environment]::SetEnvironmentVariable("TIEZ_INTERNAL_LIFECYCLE_HARNESS_DIR", $previousHarnessDirectory, "Process")
    if ([IO.Directory]::Exists($harnessDirectory)) {
        Remove-Item -LiteralPath $harnessDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
    if ($null -ne $executableLock) {
        $executableSha256AfterRun = Get-Issue154LockedFileSha256 -Stream $executableLock
        $executableLock.Dispose()
    }
}

$latencies = @($cyclesResult | ForEach-Object { [double]$_.requested_visible_focused_hydrated_search_ready_ms })
$worstFive = @($latencies | Sort-Object -Descending | Select-Object -First 5)
$latencyGatePass = (Get-Issue154Median -Values $latencies) -le $SearchReadyMedianThresholdMilliseconds -and (($worstFive | Measure-Object -Maximum).Maximum) -le $SearchReadyWorstFiveThresholdMilliseconds
$clipboardGatePass = @($cyclesResult | Where-Object { -not $_.clipboard_activity_history_consistent }).Count -eq 0
$lifecycleGatePass = @($cyclesResult | Where-Object { $_.main_window_count -gt 1 -or $_.down_phase -ne $Mode -or $_.ready_phase -ne "ready" }).Count -eq 0
$generationGatePass = @(
    $cyclesResult |
        Where-Object { [uint64]$_.generation -ne [uint64]$_.show_response.expected_generation }
).Count -eq 0
$memory5PrivateValues = @($memoryRunsResult | ForEach-Object { [double]$_.sample_after_5s.private_working_set_bytes })
$memory30PrivateValues = @($memoryRunsResult | ForEach-Object { [double]$_.sample_after_30s.private_working_set_bytes })
$memory5CommitValues = @($memoryRunsResult | ForEach-Object { [double]$_.sample_after_5s.commit_bytes })
$memory30CommitValues = @($memoryRunsResult | ForEach-Object { [double]$_.sample_after_30s.commit_bytes })
$memory5WorkingSetValues = @($memoryRunsResult | ForEach-Object { [double]$_.sample_after_5s.working_set_bytes })
$memory30WorkingSetValues = @($memoryRunsResult | ForEach-Object { [double]$_.sample_after_30s.working_set_bytes })
$memory5ProcessCountValues = @($memoryRunsResult | ForEach-Object { [double]$_.sample_after_5s.process_count })
$memory30ProcessCountValues = @($memoryRunsResult | ForEach-Object { [double]$_.sample_after_30s.process_count })
$memoryMedians = [ordered]@{
    after_5s = [ordered]@{
        private_working_set_bytes = [int64](Get-Issue154Median -Values $memory5PrivateValues)
        private_working_set_mib = [Math]::Round((Get-Issue154Median -Values $memory5PrivateValues) / 1048576.0, 1)
        commit_bytes = [int64](Get-Issue154Median -Values $memory5CommitValues)
        commit_mib = [Math]::Round((Get-Issue154Median -Values $memory5CommitValues) / 1048576.0, 1)
        working_set_bytes = [int64](Get-Issue154Median -Values $memory5WorkingSetValues)
        working_set_mib = [Math]::Round((Get-Issue154Median -Values $memory5WorkingSetValues) / 1048576.0, 1)
        process_count = [Math]::Round((Get-Issue154Median -Values $memory5ProcessCountValues), 1)
    }
    after_30s = [ordered]@{
        private_working_set_bytes = [int64](Get-Issue154Median -Values $memory30PrivateValues)
        private_working_set_mib = [Math]::Round((Get-Issue154Median -Values $memory30PrivateValues) / 1048576.0, 1)
        commit_bytes = [int64](Get-Issue154Median -Values $memory30CommitValues)
        commit_mib = [Math]::Round((Get-Issue154Median -Values $memory30CommitValues) / 1048576.0, 1)
        working_set_bytes = [int64](Get-Issue154Median -Values $memory30WorkingSetValues)
        working_set_mib = [Math]::Round((Get-Issue154Median -Values $memory30WorkingSetValues) / 1048576.0, 1)
        process_count = [Math]::Round((Get-Issue154Median -Values $memory30ProcessCountValues), 1)
    }
}
$observedProcessIdentities = @(
    @($baselineSample.process_identity_keys)
    foreach ($memoryRun in $memoryRunsResult) {
        @($memoryRun.sample_after_5s.process_identity_keys)
        @($memoryRun.sample_after_30s.process_identity_keys)
    }
) | Sort-Object -Unique
$observedProcessIdentities = @($observedProcessIdentities)
if ($executableSha256BeforeStart -cne $executableSha256AfterRun) {
    throw "Executable SHA-256 changed during the measured run."
}

$document = [ordered]@{
    schema_version = 2
    label = $Label
    captured_at_utc = [DateTime]::UtcNow.ToString("o")
    executable = $resolvedExecutable
    executable_sha256 = $executableSha256BeforeStart
    executable_hash_observations = [ordered]@{
        before_start = $executableSha256BeforeStart
        after_start = $executableSha256AfterStart
        after_run = $executableSha256AfterRun
    }
    executable_arguments = $ExecutableArgument
    main_window_title = $MainWindowTitle
    mode = $Mode
    protocol = [ordered]@{
        cycles = $Cycles
        memory_runs = $MemoryRuns
        ipc_commands = @("lifecycle_test_hide", "get_main_ui_lifecycle_clipboard_probe", "lifecycle_test_show", "get_main_ui_lifecycle_snapshot", "get_main_ui_lifecycle_traces")
        transport = "TIEZ_INTERNAL_LIFECYCLE_HARNESS_DIR filesystem protocol with unique safe string id <id>.request.json files atomically published from same-directory tmp files and matching <id>.response.json payloads"
        harness_directory_runtime = "unique empty temporary directory created before launch, passed in process env, removed on exit"
        lifecycle_mode_env = "TIEZ_EXPERIMENT_MAIN_UI_LIFECYCLE"
        harness_dir_env = "TIEZ_INTERNAL_LIFECYCLE_HARNESS_DIR"
        exact_executable_hash_required_across_modes = $true
        max_visible_main_windows = 1
        latency_metric = "requested-visible-focused-hydrated-search_ready"
        median_latency_threshold_ms = $SearchReadyMedianThresholdMilliseconds
        worst_five_latency_threshold_ms = $SearchReadyWorstFiveThresholdMilliseconds
        memory_samples_seconds = @($FastMemorySettleSeconds, $MemorySettleSeconds)
        memory_sampling_state = "independent $Mode down state for each memory run, followed by explicit lifecycle_test_show and ready wait"
        memory_scope = "root process plus recursively discovered descendants and msedgewebview2 processes sharing the baseline --user-data-dir"
        descendant_tree_requires_process_explorer_etw_cross_check = $true
        paired_memory_gate = "pending; this single-mode report intentionally does not compute the hidden-vs-destroyed comparison gate. Compare matched reports from the same executable/configuration in a pair script or later analysis. Thresholds reserved for that paired analysis: >= $MemoryReductionPercentThreshold percent and >= $MemoryReductionMiBThreshold MiB at both horizons."
        clipboard_check = "while down, Set-Clipboard writes a unique token and get_main_ui_lifecycle_clipboard_probe must report listener event increase plus exact token in persistent or session history"
        windows_11_required = $true
        linux_screening_only = $true
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
    manual_context = [ordered]@{
        data_snapshot = $null
        feature_flags = $null
        service_configuration = $null
        power_plan = $null
        webview2_runtime_version = $null
        foreground_application = $null
        note = "Fill these fields from the Windows runbook before paired sign-off. The measurement script does not infer them."
    }
    baseline_ready_snapshot = $baselineSnapshot
    baseline_memory_sample = $baselineSample
    cycles = $cyclesResult
    memory_runs = $memoryRunsResult
    memory_after_5s = $memoryAfter5s
    memory_after_30s = $memoryAfter30s
    memory_medians = $memoryMedians
    observed_process_identities = $observedProcessIdentities
    final_snapshot = $finalSnapshot
    final_traces = $finalTraces
    summary = [ordered]@{
        median_requested_visible_focused_hydrated_search_ready_ms = [Math]::Round((Get-Issue154Median -Values $latencies), 1)
        worst_five_requested_visible_focused_hydrated_search_ready_ms = $worstFive
        max_requested_visible_focused_hydrated_search_ready_ms = [Math]::Round((($latencies | Measure-Object -Maximum).Maximum), 1)
        memory_median_after_5s_private_working_set_bytes = $memoryMedians["after_5s"]["private_working_set_bytes"]
        memory_median_after_5s_private_working_set_mib = $memoryMedians["after_5s"]["private_working_set_mib"]
        memory_median_after_30s_private_working_set_bytes = $memoryMedians["after_30s"]["private_working_set_bytes"]
        memory_median_after_30s_private_working_set_mib = $memoryMedians["after_30s"]["private_working_set_mib"]
        paired_hidden_vs_destroyed_comparison = "pending"
        paired_memory_gate_pass = $null
        paired_memory_gate_note = "Not computed by this single-mode report. Use a matched hidden/destroyed pair from the same executable hash, feature set, data snapshot, and service configuration."
        latency_gate_pass = [bool]$latencyGatePass
        lifecycle_gate_pass = [bool]($lifecycleGatePass -and $generationGatePass)
        clipboard_gate_pass = [bool]$clipboardGatePass
        single_mode_functional_pass = [bool]($latencyGatePass -and $lifecycleGatePass -and $generationGatePass -and $clipboardGatePass)
        overall_pass = $null
        windows_11_real_machine_run = $true
    }
}

$json = $document | ConvertTo-Json -Depth 50
$utf8NoBom = New-Object System.Text.UTF8Encoding $false
[IO.File]::WriteAllText($outputPath, $json, $utf8NoBom)
$validatorPath = Join-Path $PSScriptRoot "validate_lifecycle_report.ps1"
& $validatorPath -Report $outputPath | Out-Null
$json
