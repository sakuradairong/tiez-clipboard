[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Report,

    [switch]$PassThru
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Issue154Semantic {
    param(
        [object]$Condition,
        [string]$Message
    )
    if ($Condition -isnot [bool] -or -not $Condition) {
        throw "Issue #154 lifecycle report semantic validation failed: $Message"
    }
}

function Assert-Issue154Boolean {
    param(
        [object]$Value,
        [bool]$Expected,
        [string]$Name
    )
    Assert-Issue154Semantic ($Value -is [bool]) "$Name must be a JSON boolean"
    Assert-Issue154Semantic ($Value -eq $Expected) "$Name must be $($Expected.ToString().ToLowerInvariant())"
}

function Assert-Issue154Array {
    param(
        [object]$Value,
        [string]$Name
    )
    Assert-Issue154Semantic ($Value -is [System.Array]) "$Name must be a JSON array"
}

function Test-Issue154Equal {
    param(
        [object]$Left,
        [object]$Right
    )
    $leftJson = ConvertTo-Json -InputObject $Left -Compress -Depth 50
    $rightJson = ConvertTo-Json -InputObject $Right -Compress -Depth 50
    return $leftJson -ceq $rightJson
}

function Get-Issue154Median {
    param([double[]]$Values)
    Assert-Issue154Semantic ($Values.Count -gt 0) "median input must not be empty"
    $sorted = @($Values | Sort-Object)
    $middle = [Math]::Floor($sorted.Count / 2)
    if (($sorted.Count % 2) -eq 1) {
        return [double]$sorted[$middle]
    }
    return ([double]$sorted[$middle - 1] + [double]$sorted[$middle]) / 2.0
}

function Assert-Issue154NumberEqual {
    param(
        [double]$Expected,
        [double]$Actual,
        [string]$Name,
        [double]$Tolerance = 0.05
    )
    Assert-Issue154Semantic ([Math]::Abs($Expected - $Actual) -le $Tolerance) "$Name must be $Expected, found $Actual"
}

function Test-Issue154StringSetEqual {
    param(
        [object[]]$Left,
        [object[]]$Right,
        [switch]$IgnoreCase
    )
    $leftValues = @($Left | ForEach-Object { [string]$_ })
    $rightValues = @($Right | ForEach-Object { [string]$_ })
    if ($IgnoreCase) {
        $leftValues = @($leftValues | ForEach-Object { $_.ToLowerInvariant() })
        $rightValues = @($rightValues | ForEach-Object { $_.ToLowerInvariant() })
    }
    $leftValues = @($leftValues | Sort-Object -Unique)
    $rightValues = @($rightValues | Sort-Object -Unique)
    return (Test-Issue154Equal -Left $leftValues -Right $rightValues)
}

function Get-Issue154ProcessIdentityKey {
    param([object]$Process)
    $startedAtUtc = if ($Process.started_at_utc -is [DateTime]) {
        ([DateTime]$Process.started_at_utc).ToUniversalTime().ToString("o")
    } elseif ($Process.started_at_utc -is [DateTimeOffset]) {
        ([DateTimeOffset]$Process.started_at_utc).UtcDateTime.ToString("o")
    } else {
        [string]$Process.started_at_utc
    }
    return "{0}|{1}|{2}|{3}" -f $Process.pid, $Process.role, $Process.executable_path, $startedAtUtc
}

function Get-Issue154LockedFileSha256 {
    param([System.IO.FileStream]$Stream)
    $Stream.Position = 0
    return (Get-FileHash -InputStream $Stream -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-Issue154SnapshotBooleans {
    param(
        [object]$Snapshot,
        [string]$Name
    )
    foreach ($property in @(
        "enabled",
        "react_mounted",
        "hydrated",
        "search_ready",
        "search_results_settled",
        "focused",
        "main_window_present",
        "main_window_visible",
        "main_window_focused",
        "history_probe_available",
        "explicit_exit_requested",
        "worker_running"
    )) {
        Assert-Issue154Semantic ($Snapshot.$property -is [bool]) "$Name.$property must be a JSON boolean"
    }
    Assert-Issue154Boolean -Value $Snapshot.enabled -Expected $true -Name "$Name.enabled"
    Assert-Issue154Boolean -Value $Snapshot.history_probe_available -Expected $true -Name "$Name.history_probe_available"
    Assert-Issue154Boolean -Value $Snapshot.explicit_exit_requested -Expected $false -Name "$Name.explicit_exit_requested"
}

function Assert-Issue154ProcessSample {
    param(
        [object]$Sample,
        [string[]]$ExpectedReferenceIdentities,
        [string]$ExpectedRootIdentity,
        [int]$ExpectedRootProcessId,
        [string]$ExpectedRootExecutablePath,
        [string[]]$ExpectedWebView2UserDataFolders,
        [string]$Name
    )

    Assert-Issue154Array -Value $Sample.processes -Name "$Name.processes"
    Assert-Issue154Array -Value $Sample.process_identity_keys -Name "$Name.process_identity_keys"
    Assert-Issue154Array -Value $Sample.reference_process_identity_keys -Name "$Name.reference_process_identity_keys"
    Assert-Issue154Array -Value $Sample.identities_added_from_reference -Name "$Name.identities_added_from_reference"
    Assert-Issue154Array -Value $Sample.identities_missing_from_reference -Name "$Name.identities_missing_from_reference"
    Assert-Issue154Array -Value $Sample.webview2_user_data_folders -Name "$Name.webview2_user_data_folders"

    $processes = @($Sample.processes)
    Assert-Issue154Semantic ($processes.Count -gt 0) "$Name.processes must not be empty"
    $processIds = @($processes | ForEach-Object { [int]$_.pid })
    Assert-Issue154Semantic (@($processIds | Sort-Object -Unique).Count -eq $processes.Count) "$Name contains duplicate process IDs"
    $identityKeys = @($processes | ForEach-Object { Get-Issue154ProcessIdentityKey -Process $_ } | Sort-Object -Unique)
    Assert-Issue154Semantic ($identityKeys.Count -eq $processes.Count) "$Name contains duplicate process identities"
    Assert-Issue154Semantic (Test-Issue154StringSetEqual -Left $identityKeys -Right @($Sample.process_identity_keys)) "$Name.process_identity_keys do not match processes"
    Assert-Issue154Semantic (@($Sample.process_identity_keys).Count -eq $identityKeys.Count) "$Name.process_identity_keys contains duplicates"
    Assert-Issue154Semantic (Test-Issue154StringSetEqual -Left @($Sample.reference_process_identity_keys) -Right $ExpectedReferenceIdentities) "$Name.reference_process_identity_keys do not match baseline"
    Assert-Issue154Semantic (@($Sample.reference_process_identity_keys).Count -eq @($ExpectedReferenceIdentities | Sort-Object -Unique).Count) "$Name.reference_process_identity_keys contains duplicates"
    Assert-Issue154Semantic ($identityKeys -ccontains $ExpectedRootIdentity) "$Name does not contain the measured root process identity"
    Assert-Issue154Semantic ($processIds -contains $ExpectedRootProcessId) "$Name does not contain the measured root process ID"
    Assert-Issue154Semantic (Test-Issue154StringSetEqual -Left @($Sample.webview2_user_data_folders) -Right $ExpectedWebView2UserDataFolders -IgnoreCase) "$Name WebView2 user-data-dir scope differs from baseline"

    $added = @($identityKeys | Where-Object { $ExpectedReferenceIdentities -cnotcontains $_ })
    $missing = @($ExpectedReferenceIdentities | Where-Object { $identityKeys -cnotcontains $_ })
    Assert-Issue154Semantic (Test-Issue154StringSetEqual -Left $added -Right @($Sample.identities_added_from_reference)) "$Name.identities_added_from_reference is incorrect"
    Assert-Issue154Semantic (Test-Issue154StringSetEqual -Left $missing -Right @($Sample.identities_missing_from_reference)) "$Name.identities_missing_from_reference is incorrect"
    Assert-Issue154Semantic (@($Sample.identities_added_from_reference).Count -eq $added.Count) "$Name.identities_added_from_reference contains duplicates"
    Assert-Issue154Semantic (@($Sample.identities_missing_from_reference).Count -eq $missing.Count) "$Name.identities_missing_from_reference contains duplicates"

    $descendantCount = @($processes | Where-Object { $_.attribution -ceq "descendant" }).Count
    $attributedWebView2Count = @($processes | Where-Object { $_.attribution -ceq "webview2_user_data_dir" }).Count
    Assert-Issue154Semantic ([int]$Sample.process_count -eq $processes.Count) "$Name.process_count is incorrect"
    Assert-Issue154Semantic ([int]$Sample.descendant_process_count -eq $descendantCount) "$Name.descendant_process_count is incorrect"
    Assert-Issue154Semantic ([int]$Sample.attributed_webview2_process_count -eq $attributedWebView2Count) "$Name.attributed_webview2_process_count is incorrect"

    $rootProcesses = @($processes | Where-Object { [int]$_.pid -eq $ExpectedRootProcessId })
    Assert-Issue154Semantic ($rootProcesses.Count -eq 1) "$Name must contain exactly one measured root process"
    Assert-Issue154Semantic ([string]$rootProcesses[0].role -ceq "application" -and [string]$rootProcesses[0].attribution -ceq "descendant") "$Name measured root role/attribution is incorrect"
    $reportedRootPath = [IO.Path]::GetFullPath([string]$rootProcesses[0].executable_path)
    $expectedRootPath = [IO.Path]::GetFullPath($ExpectedRootExecutablePath)
    Assert-Issue154Semantic ([string]::Equals($reportedRootPath, $expectedRootPath, [StringComparison]::OrdinalIgnoreCase)) "$Name measured root executable differs from the report executable"
    $processesById = @{}
    foreach ($process in $processes) {
        $processesById[[int]$process.pid] = $process
    }
    foreach ($process in @($processes | Where-Object { [string]$_.attribution -ceq "descendant" -and [int]$_.pid -ne $ExpectedRootProcessId })) {
        $visited = [System.Collections.Generic.HashSet[int]]::new()
        $current = $process
        $reachesRoot = $false
        while ($null -ne $current -and $visited.Add([int]$current.pid)) {
            $parentId = [int]$current.parent_pid
            if ($parentId -eq $ExpectedRootProcessId) {
                $reachesRoot = $true
                break
            }
            if (-not $processesById.ContainsKey($parentId)) {
                break
            }
            $current = $processesById[$parentId]
        }
        Assert-Issue154Semantic $reachesRoot "$Name descendant PID $($process.pid) does not reach the measured root through parent_pid"
    }
    foreach ($process in @($processes | Where-Object { [string]$_.attribution -ceq "webview2_user_data_dir" })) {
        Assert-Issue154Semantic ([string]$process.name -match "(?i)^msedgewebview2(?:\.exe)?$") "$Name user-data-dir attribution includes a non-WebView2 process"
        Assert-Issue154Semantic (-not [string]::IsNullOrWhiteSpace([string]$process.webview2_user_data_folder)) "$Name user-data-dir attribution lacks its normalized folder"
        Assert-Issue154Semantic (Test-Issue154StringSetEqual -Left @([string]$process.webview2_user_data_folder) -Right @([string]$ExpectedWebView2UserDataFolders[0])) "$Name user-data-dir attributed process is outside the baseline scope"
    }

    $computedRoleCounts = @{}
    foreach ($process in $processes) {
        $role = [string]$process.role
        if ($computedRoleCounts.ContainsKey($role)) {
            $computedRoleCounts[$role]++
        } else {
            $computedRoleCounts[$role] = 1
        }
        $isWebView2 = [string]$process.name -imatch '^msedgewebview2(?:\.exe)?$'
        if ($isWebView2) {
            $hasUserDataFolder = -not [string]::IsNullOrWhiteSpace([string]$process.webview2_user_data_folder)
            if ([string]$process.attribution -ceq "webview2_user_data_dir") {
                Assert-Issue154Semantic $hasUserDataFolder "$Name attributed WebView2 process is missing its user-data-dir"
            }
            if ($hasUserDataFolder) {
                $folderMatches = @(
                    $Sample.webview2_user_data_folders |
                        Where-Object { [string]::Equals([string]$_, [string]$process.webview2_user_data_folder, [StringComparison]::OrdinalIgnoreCase) }
                ).Count
                Assert-Issue154Semantic ($folderMatches -eq 1) "$Name WebView2 process user-data-dir is outside the sample scope"
            }
        } else {
            Assert-Issue154Semantic ($null -eq $process.webview2_user_data_folder) "$Name non-WebView2 process must not claim a WebView2 user-data-dir"
        }
    }
    $reportedRoleNames = @($Sample.role_counts.PSObject.Properties | ForEach-Object { $_.Name })
    Assert-Issue154Semantic (Test-Issue154StringSetEqual -Left @($computedRoleCounts.Keys) -Right $reportedRoleNames) "$Name.role_counts role set is incorrect"
    foreach ($role in $computedRoleCounts.Keys) {
        $reportedRoleCount = $Sample.role_counts.PSObject.Properties[[string]$role].Value
        Assert-Issue154Semantic ([int]$reportedRoleCount -eq [int]$computedRoleCounts[$role]) "$Name.role_counts.$role is incorrect"
    }

    $workingSet = [int64](($processes | Measure-Object -Property working_set_bytes -Sum).Sum)
    $privateWorkingSet = [int64](($processes | Measure-Object -Property private_working_set_bytes -Sum).Sum)
    $commit = [int64](($processes | Measure-Object -Property commit_bytes -Sum).Sum)
    Assert-Issue154Semantic ([int64]$Sample.working_set_bytes -eq $workingSet) "$Name.working_set_bytes is incorrect"
    Assert-Issue154Semantic ([int64]$Sample.private_working_set_bytes -eq $privateWorkingSet) "$Name.private_working_set_bytes is incorrect"
    Assert-Issue154Semantic ([int64]$Sample.commit_bytes -eq $commit) "$Name.commit_bytes is incorrect"
}

function Assert-Issue154ReadySnapshot {
    param(
        [object]$Snapshot,
        [uint64]$RequestId,
        [uint64]$Generation,
        [string]$Mode,
        [string]$Name
    )
    Assert-Issue154SnapshotBooleans -Snapshot $Snapshot -Name $Name
    Assert-Issue154Semantic ([string]$Snapshot.mode -ceq $Mode) "$Name.mode is incorrect"
    Assert-Issue154Semantic ([string]$Snapshot.phase -ceq "ready") "$Name.phase must be ready"
    Assert-Issue154Semantic ([uint64]$Snapshot.generation -eq $Generation) "$Name.generation is incorrect"
    Assert-Issue154Semantic ([uint64]$Snapshot.completed_request_id -eq $RequestId) "$Name.completed_request_id is incorrect"
    Assert-Issue154Semantic ([uint64]$Snapshot.active_request_id -eq $RequestId) "$Name.active_request_id is incorrect"
    Assert-Issue154Semantic ([string]$Snapshot.active_intent -ceq "test") "$Name.active_intent must be test"
    Assert-Issue154Semantic ($null -eq $Snapshot.failed_request_id) "$Name.failed_request_id must be null"
    Assert-Issue154Boolean -Value $Snapshot.main_window_present -Expected $true -Name "$Name.main_window_present"
    Assert-Issue154Boolean -Value $Snapshot.main_window_visible -Expected $true -Name "$Name.main_window_visible"
    Assert-Issue154Boolean -Value $Snapshot.main_window_focused -Expected $true -Name "$Name.main_window_focused"
    Assert-Issue154Boolean -Value $Snapshot.react_mounted -Expected $true -Name "$Name.react_mounted"
    Assert-Issue154Boolean -Value $Snapshot.hydrated -Expected $true -Name "$Name.hydrated"
    Assert-Issue154Boolean -Value $Snapshot.search_ready -Expected $true -Name "$Name.search_ready"
    Assert-Issue154Boolean -Value $Snapshot.focused -Expected $true -Name "$Name.focused"
    Assert-Issue154Semantic ([int]$Snapshot.main_window_count -eq 1) "$Name.main_window_count must be one"
    Assert-Issue154Semantic ($null -ne $Snapshot.requested_visible_focused_hydrated_search_ready_ms) "$Name usable-ready latency must be present"
    Assert-Issue154Semantic ([double]$Snapshot.requested_visible_focused_hydrated_search_ready_ms -ge 0) "$Name usable-ready latency must not be negative"
    Assert-Issue154Semantic ($null -eq $Snapshot.in_flight_target -and $null -eq $Snapshot.pending_target) "$Name must not retain an in-flight or pending target"
}

function Assert-Issue154DownSnapshot {
    param(
        [object]$Snapshot,
        [uint64]$RequestId,
        [uint64]$Generation,
        [Nullable[uint64]]$ExpectedActiveRequestId,
        [string]$Mode,
        [string]$Name
    )
    Assert-Issue154SnapshotBooleans -Snapshot $Snapshot -Name $Name
    Assert-Issue154Semantic ([string]$Snapshot.mode -ceq $Mode) "$Name.mode is incorrect"
    Assert-Issue154Semantic ([string]$Snapshot.phase -ceq $Mode) "$Name.phase is incorrect"
    Assert-Issue154Semantic ([uint64]$Snapshot.generation -eq $Generation) "$Name.generation is incorrect"
    Assert-Issue154Semantic ([uint64]$Snapshot.completed_request_id -eq $RequestId) "$Name.completed_request_id is incorrect"
    if ($null -eq $ExpectedActiveRequestId) {
        Assert-Issue154Semantic ($null -eq $Snapshot.active_request_id) "$Name.active_request_id must be null before the first wake"
        Assert-Issue154Semantic ($null -eq $Snapshot.active_intent) "$Name.active_intent must be null before the first wake"
    } else {
        Assert-Issue154Semantic ([uint64]$Snapshot.active_request_id -eq [uint64]$ExpectedActiveRequestId) "$Name.active_request_id must retain the preceding show request"
        Assert-Issue154Semantic ([string]$Snapshot.active_intent -ceq "test") "$Name.active_intent must retain the preceding test wake"
    }
    Assert-Issue154Semantic ($null -eq $Snapshot.failed_request_id) "$Name.failed_request_id must be null"
    Assert-Issue154Boolean -Value $Snapshot.main_window_visible -Expected $false -Name "$Name.main_window_visible"
    Assert-Issue154Boolean -Value $Snapshot.main_window_focused -Expected $false -Name "$Name.main_window_focused"
    Assert-Issue154Semantic ($null -eq $Snapshot.in_flight_target -and $null -eq $Snapshot.pending_target) "$Name must not retain an in-flight or pending target"
    if ($Mode -ceq "hidden") {
        Assert-Issue154Boolean -Value $Snapshot.main_window_present -Expected $true -Name "$Name.main_window_present"
        Assert-Issue154Semantic ([int]$Snapshot.main_window_count -eq 1) "$Name hidden main_window_count must be one"
    } else {
        Assert-Issue154Boolean -Value $Snapshot.main_window_present -Expected $false -Name "$Name.main_window_present"
        Assert-Issue154Semantic ([int]$Snapshot.main_window_count -eq 0) "$Name destroyed main_window_count must be zero"
    }
}

function Assert-Issue154Response {
    param(
        [object]$Response,
        [uint64]$ExpectedRequestId,
        [uint64]$ExpectedGenerationBefore,
        [Nullable[uint64]]$ExpectedGeneration,
        [string]$Name
    )
    Assert-Issue154Boolean -Value $Response.accepted -Expected $true -Name "$Name.accepted"
    Assert-Issue154Semantic ([uint64]$Response.request_id -eq $ExpectedRequestId) "$Name.request_id must be consecutive"
    Assert-Issue154Semantic ([uint64]$Response.generation_before -eq $ExpectedGenerationBefore) "$Name.generation_before is incorrect"
    if ($null -ne $ExpectedGeneration) {
        Assert-Issue154Semantic ([uint64]$Response.expected_generation -eq [uint64]$ExpectedGeneration) "$Name.expected_generation is incorrect"
    }
}

function Assert-Issue154FinalTraces {
    param(
        [object[]]$Traces,
        [object]$FinalSnapshot,
        [uint64]$FinalRequestId,
        [uint64]$FinalGeneration,
        [string]$Mode
    )
    Assert-Issue154Semantic ($Traces.Count -gt 0) "final_traces must not be empty"
    Assert-Issue154Semantic ([int]$FinalSnapshot.trace_count -eq $Traces.Count) "final_snapshot.trace_count must match final_traces length"
    $requestedTimestamps = @{}
    foreach ($trace in $Traces) {
        Assert-Issue154Semantic ([string]$trace.mode -ceq $Mode) "final_traces contains a different lifecycle mode"
        Assert-Issue154Semantic ([uint64]$trace.request_id -gt 0 -and [uint64]$trace.generation -gt 0) "final_traces contains an invalid request/generation"
        Assert-Issue154Semantic ([uint64]$trace.timestamp_unix_ms -gt 0 -and [uint64]$trace.elapsed_ms -ge 0) "final_traces contains an invalid timestamp/elapsed value"
        if ([string]$trace.phase -ceq "requested") {
            Assert-Issue154Semantic ([uint64]$trace.elapsed_ms -eq 0) "requested traces must have zero elapsed_ms"
            $requestedTimestamps[[string]$trace.request_id] = [uint64]$trace.timestamp_unix_ms
        }
    }
    foreach ($trace in $Traces) {
        $requestKey = [string]$trace.request_id
        if ([string]$trace.phase -cne "requested" -and $requestedTimestamps.ContainsKey($requestKey)) {
            $wallElapsed = [int64]$trace.timestamp_unix_ms - [int64]$requestedTimestamps[$requestKey]
            Assert-Issue154Semantic ($wallElapsed -ge 0) "trace timestamp precedes its requested trace"
            Assert-Issue154Semantic ([Math]::Abs($wallElapsed - [int64]$trace.elapsed_ms) -le 1000) "trace elapsed_ms is inconsistent with its requested timestamp"
        }
    }
    $finalRequestTraces = @($Traces | Where-Object { [uint64]$_.request_id -eq $FinalRequestId })
    Assert-Issue154Semantic ($finalRequestTraces.Count -gt 0) "final_traces does not contain the final request"
    Assert-Issue154Semantic (@($finalRequestTraces | Where-Object { [uint64]$_.generation -ne $FinalGeneration -or [string]$_.intent -cne "test" }).Count -eq 0) "final request trace generation/intent is incorrect"
    $finalPhases = @($finalRequestTraces | ForEach-Object { [string]$_.phase })
    foreach ($phase in @("requested", "visible", "focused", "hydrated", "search_ready", "ready")) {
        Assert-Issue154Semantic ($finalPhases -ccontains $phase) "final request traces are missing phase '$phase'"
    }
}

$resolvedReport = (Resolve-Path -LiteralPath $Report).Path
$document = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($resolvedReport, [Text.Encoding]::UTF8))
Assert-Issue154Semantic ([int]$document.schema_version -eq 3) "schema_version must be 3"
$mode = [string]$document.mode
Assert-Issue154Semantic ($mode -ceq "hidden" -or $mode -ceq "destroyed") "mode must be hidden or destroyed"

$resolvedExecutable = (Resolve-Path -LiteralPath ([string]$document.executable)).Path
$executableStream = [IO.File]::Open($resolvedExecutable, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
try {
    $hash = [string]$document.executable_sha256
    $hashObservations = $document.executable_hash_observations
    $actualExecutableHash = Get-Issue154LockedFileSha256 -Stream $executableStream
    Assert-Issue154Semantic ($actualExecutableHash -ceq $hash) "measured executable file SHA-256 differs from report"
    Assert-Issue154Semantic ([string]$hashObservations.before_start -ceq $hash) "before_start executable hash differs"
    Assert-Issue154Semantic ([string]$hashObservations.after_start -ceq $hash) "after_start executable hash differs"
    Assert-Issue154Semantic ([string]$hashObservations.after_run -ceq $hash) "after_run executable hash differs"

    Assert-Issue154Array -Value $document.executable_arguments -Name "executable_arguments"
    Assert-Issue154Boolean -Value $document.root_command_line.verified -Expected $true -Name "root_command_line.verified"
    Assert-Issue154Array -Value $document.root_command_line.arguments -Name "root_command_line.arguments"
    Assert-Issue154Semantic (-not [string]::IsNullOrWhiteSpace([string]$document.root_command_line.raw)) "root_command_line.raw must not be empty"
    $observedExecutable = [IO.Path]::GetFullPath([string]$document.root_command_line.executable)
    Assert-Issue154Semantic ([string]::Equals($observedExecutable, $resolvedExecutable, [StringComparison]::OrdinalIgnoreCase)) "root command line executable differs from the measured executable"
    Assert-Issue154Semantic (Test-Issue154Equal -Left $document.root_command_line.arguments -Right $document.executable_arguments) "root command line arguments differ from requested executable_arguments"

    Assert-Issue154Semantic ([int]$document.protocol.cycles -eq @($document.cycles).Count) "protocol.cycles does not match cycles length"
    Assert-Issue154Semantic ([int]$document.protocol.memory_runs -eq @($document.memory_runs).Count) "protocol.memory_runs does not match memory_runs length"
    Assert-Issue154Semantic ([int]$document.protocol.cycles -ge 100) "at least 100 cycles are required"
    Assert-Issue154Semantic ([int]$document.protocol.memory_runs -ge 5 -and [int]$document.protocol.memory_runs -le 99 -and ([int]$document.protocol.memory_runs % 2) -eq 1) "memory_runs must be an odd count from five through 99"
    Assert-Issue154Semantic (Test-Issue154Equal -Left @($document.protocol.memory_samples_seconds) -Right @(5, 30)) "memory sample horizons must be exactly 5 and 30 seconds"
    Assert-Issue154Semantic ([int]$document.protocol.max_visible_main_windows -eq 1) "max_visible_main_windows must be one"
    Assert-Issue154Semantic ([string]$document.protocol.lifecycle_mode_env -ceq "TIEZ_EXPERIMENT_MAIN_UI_LIFECYCLE" -and [string]$document.protocol.harness_dir_env -ceq "TIEZ_INTERNAL_LIFECYCLE_HARNESS_DIR") "internal environment-variable contract is incorrect"
    foreach ($command in @("lifecycle_test_hide", "lifecycle_test_show", "get_main_ui_lifecycle_clipboard_probe", "get_main_ui_lifecycle_snapshot", "get_main_ui_lifecycle_traces")) {
        Assert-Issue154Semantic (@($document.protocol.ipc_commands) -ccontains $command) "protocol.ipc_commands is missing $command"
    }
    Assert-Issue154Semantic ([string]$document.protocol.process_identity_key -ceq "pid|role|executable_path|started_at_utc") "process_identity_key is unsupported"
    Assert-Issue154Semantic ([string]$document.protocol.process_attribution_rule -ceq "root-descendants-plus-webview2-baseline-user-data-dir-v1") "process_attribution_rule is unsupported"
    Assert-Issue154Boolean -Value $document.protocol.stable_root_identity_required -Expected $true -Name "protocol.stable_root_identity_required"
    Assert-Issue154Boolean -Value $document.protocol.dynamic_process_identity_churn_allowed -Expected $true -Name "protocol.dynamic_process_identity_churn_allowed"
    Assert-Issue154Semantic ([double]$document.protocol.median_latency_threshold_ms -le 750) "median latency threshold is weaker than 750 ms"
    Assert-Issue154Semantic ([double]$document.protocol.worst_five_latency_threshold_ms -le 1500) "worst-five latency threshold is weaker than 1500 ms"
    Assert-Issue154Boolean -Value $document.protocol.exact_executable_hash_required_across_modes -Expected $true -Name "protocol.exact_executable_hash_required_across_modes"
    Assert-Issue154Boolean -Value $document.protocol.descendant_tree_requires_process_explorer_etw_cross_check -Expected $true -Name "protocol.descendant_tree_requires_process_explorer_etw_cross_check"
    Assert-Issue154Boolean -Value $document.protocol.windows_11_required -Expected $true -Name "protocol.windows_11_required"
    Assert-Issue154Boolean -Value $document.protocol.linux_screening_only -Expected $true -Name "protocol.linux_screening_only"
    Assert-Issue154Semantic ([string]$document.protocol.paired_memory_gate -match "40" -and [string]$document.protocol.paired_memory_gate -match "50") "paired_memory_gate must preserve the 40 percent and 50 MiB thresholds"

    $hostBuild = 0
    Assert-Issue154Semantic ([string]$document.host.os_caption -match "Windows 11") "host.os_caption must identify Windows 11"
    $hostBuildParsed = [int]::TryParse([string]$document.host.os_build, [ref]$hostBuild)
    Assert-Issue154Semantic ($hostBuildParsed -and $hostBuild -ge 22000) "host.os_build must be Windows 11 build 22000 or newer"
    Assert-Issue154Array -Value $document.host.processors -Name "host.processors"
    Assert-Issue154Semantic (@($document.host.processors).Count -gt 0 -and @($document.host.processors | Where-Object { [string]::IsNullOrWhiteSpace([string]$_) }).Count -eq 0) "host.processors must contain non-empty processor names"
    Assert-Issue154Semantic ([int64]$document.host.physical_memory_bytes -gt 0) "host.physical_memory_bytes must be positive"
    Assert-Issue154Semantic (-not [string]::IsNullOrWhiteSpace([string]$document.host.os_version) -and -not [string]::IsNullOrWhiteSpace([string]$document.host.architecture) -and -not [string]::IsNullOrWhiteSpace([string]$document.host.powershell)) "host version, architecture, and PowerShell must be recorded"

    Assert-Issue154SnapshotBooleans -Snapshot $document.baseline_ready_snapshot -Name "baseline_ready_snapshot"
    Assert-Issue154Semantic ([string]$document.baseline_ready_snapshot.mode -ceq $mode -and [string]$document.baseline_ready_snapshot.phase -ceq "ready") "baseline_ready_snapshot mode/phase is incorrect"
    Assert-Issue154Semantic ($null -eq $document.baseline_ready_snapshot.active_request_id -and $null -eq $document.baseline_ready_snapshot.active_intent -and $null -eq $document.baseline_ready_snapshot.completed_request_id -and $null -eq $document.baseline_ready_snapshot.failed_request_id) "baseline_ready_snapshot must precede lifecycle requests"
    Assert-Issue154Boolean -Value $document.baseline_ready_snapshot.main_window_present -Expected $true -Name "baseline_ready_snapshot.main_window_present"
    Assert-Issue154Boolean -Value $document.baseline_ready_snapshot.main_window_visible -Expected $true -Name "baseline_ready_snapshot.main_window_visible"
    Assert-Issue154Semantic ([int]$document.baseline_ready_snapshot.main_window_count -eq 1) "baseline_ready_snapshot.main_window_count must be one"

    $baselineReferenceIdentities = @($document.baseline_memory_sample.process_identity_keys)
    $baselineRootProcesses = @($document.baseline_memory_sample.processes | Where-Object { $_.role -ceq "application" -and $_.attribution -ceq "descendant" })
    Assert-Issue154Semantic ($baselineRootProcesses.Count -eq 1) "baseline must contain exactly one measured root process identity"
    $baselineRootProcessId = [int]$baselineRootProcesses[0].pid
    $baselineRootIdentity = Get-Issue154ProcessIdentityKey -Process $baselineRootProcesses[0]
    $baselineWebView2UserDataFolders = @($document.baseline_memory_sample.webview2_user_data_folders)
    Assert-Issue154Semantic ($baselineWebView2UserDataFolders.Count -gt 0) "baseline must contain at least one WebView2 user-data-dir"
    Assert-Issue154ProcessSample -Sample $document.baseline_memory_sample -ExpectedReferenceIdentities $baselineReferenceIdentities -ExpectedRootIdentity $baselineRootIdentity -ExpectedRootProcessId $baselineRootProcessId -ExpectedRootExecutablePath $resolvedExecutable -ExpectedWebView2UserDataFolders $baselineWebView2UserDataFolders -Name "baseline_memory_sample"
    Assert-Issue154Semantic (Test-Issue154StringSetEqual -Left @($document.baseline_memory_sample.reference_process_identity_keys) -Right $baselineReferenceIdentities) "baseline reference identities must be self-referential"

    $latencies = @()
    $tokens = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $expectedGeneration = [uint64]$document.baseline_ready_snapshot.generation
    $expectedRequestId = [uint64]1
    [Nullable[uint64]]$previousShowRequestId = $null
    for ($index = 0; $index -lt @($document.cycles).Count; $index++) {
        $cycle = @($document.cycles)[$index]
        $cycleNumber = $index + 1
        $name = "cycles[$index]"
        Assert-Issue154Semantic ([int]$cycle.cycle -eq $cycleNumber) "$name.cycle must be consecutive"
        Assert-Issue154Semantic ([string]$cycle.requested_mode -ceq $mode) "$name.requested_mode differs from report mode"
        Assert-Issue154Semantic ([string]$cycle.down_phase -ceq $mode) "$name.down_phase differs from report mode"
        Assert-Issue154Semantic ([string]$cycle.ready_phase -ceq "ready") "$name.ready_phase must be ready"
        Assert-Issue154Semantic ([int]$cycle.main_window_count -eq 1) "$name.main_window_count must be one"
        Assert-Issue154Response -Response $cycle.hide_response -ExpectedRequestId $expectedRequestId -ExpectedGenerationBefore $expectedGeneration -ExpectedGeneration $null -Name "$name.hide_response"
        Assert-Issue154DownSnapshot -Snapshot $cycle.down_snapshot -RequestId $expectedRequestId -Generation $expectedGeneration -ExpectedActiveRequestId $previousShowRequestId -Mode $mode -Name "$name.down_snapshot"
        $expectedRequestId++
        $expectedShowGeneration = if ($mode -ceq "destroyed") { $expectedGeneration + 1 } else { $expectedGeneration }
        Assert-Issue154Response -Response $cycle.show_response -ExpectedRequestId $expectedRequestId -ExpectedGenerationBefore $expectedGeneration -ExpectedGeneration $expectedShowGeneration -Name "$name.show_response"
        Assert-Issue154ReadySnapshot -Snapshot $cycle.ready_snapshot -RequestId $expectedRequestId -Generation $expectedShowGeneration -Mode $mode -Name "$name.ready_snapshot"
        Assert-Issue154Semantic ([uint64]$cycle.generation -eq $expectedShowGeneration) "$name.generation is incorrect"

        $token = [string]$cycle.clipboard_token
        Assert-Issue154Semantic (-not [string]::IsNullOrWhiteSpace($token) -and $tokens.Add($token)) "$name clipboard token must be non-empty and unique"
        Assert-Issue154Semantic ($token -ceq [string]$cycle.clipboard_probe.payload.token) "$name clipboard token differs from probe payload"
        Assert-Issue154Semantic ([uint64]$cycle.clipboard_probe.payload.clipboard_event_count_before -eq [uint64]$cycle.down_snapshot.clipboard_event_count) "$name clipboard baseline count is incorrect"
        $clipboardDelta = [uint64]$cycle.clipboard_probe.payload.clipboard_event_count - [uint64]$cycle.clipboard_probe.payload.clipboard_event_count_before
        Assert-Issue154Semantic ($clipboardDelta -gt 0 -and [uint64]$cycle.clipboard_probe.payload.clipboard_event_delta -eq $clipboardDelta) "$name clipboard delta is incorrect"
        Assert-Issue154Boolean -Value $cycle.clipboard_probe.verification.listener_event_increased -Expected $true -Name "$name.clipboard_probe.verification.listener_event_increased"
        Assert-Issue154Boolean -Value $cycle.clipboard_probe.payload.listener_event_count_increased -Expected $true -Name "$name.clipboard_probe.payload.listener_event_count_increased"
        Assert-Issue154Boolean -Value $cycle.clipboard_probe.verification.ok -Expected $true -Name "$name.clipboard_probe.verification.ok"
        Assert-Issue154Boolean -Value $cycle.clipboard_probe.payload.exact_history_match -Expected $true -Name "$name.clipboard_probe.payload.exact_history_match"
        Assert-Issue154Boolean -Value $cycle.clipboard_activity_history_consistent -Expected $true -Name "$name.clipboard_activity_history_consistent"
        Assert-Issue154Semantic ([int]$cycle.clipboard_probe.verification.exact_history_match_count -eq [int]$cycle.clipboard_probe.payload.exact_history_match_count -and [int]$cycle.clipboard_probe.payload.exact_history_match_count -eq 1) "$name clipboard match counts are incorrect"
        $persistentMatch = $null -ne $cycle.clipboard_probe.payload.persisted_entry_id
        $sessionMatch = $null -ne $cycle.clipboard_probe.payload.session_entry_id
        Assert-Issue154Boolean -Value $cycle.clipboard_probe.verification.persistent_history_exact_match -Expected $persistentMatch -Name "$name.clipboard_probe.verification.persistent_history_exact_match"
        Assert-Issue154Boolean -Value $cycle.clipboard_probe.verification.session_history_exact_match -Expected $sessionMatch -Name "$name.clipboard_probe.verification.session_history_exact_match"
        Assert-Issue154Semantic ($persistentMatch -xor $sessionMatch) "$name token must exist in exactly one history backend"
        if ($persistentMatch) {
            Assert-Issue154Semantic ([int64]$cycle.clipboard_probe.payload.persisted_entry_id -gt 0) "$name persisted_entry_id must be positive"
        }
        if ($sessionMatch) {
            Assert-Issue154Semantic ([int64]$cycle.clipboard_probe.payload.session_entry_id -lt 0) "$name session_entry_id must be negative"
        }

        $latency = [double]$cycle.ready_snapshot.requested_visible_focused_hydrated_search_ready_ms
        Assert-Issue154NumberEqual -Expected $latency -Actual ([double]$cycle.requested_visible_focused_hydrated_search_ready_ms) -Name "$name latency"
        $latencies += $latency
        $previousShowRequestId = $expectedRequestId
        $expectedRequestId++
        $expectedGeneration = $expectedShowGeneration
    }

    $allObservedIdentities = @($baselineReferenceIdentities)
    for ($index = 0; $index -lt @($document.memory_runs).Count; $index++) {
        $run = @($document.memory_runs)[$index]
        $runNumber = $index + 1
        $name = "memory_runs[$index]"
        Assert-Issue154Semantic ([int]$run.run -eq $runNumber) "$name.run must be consecutive"
        Assert-Issue154Semantic ([string]$run.requested_mode -ceq $mode -and [string]$run.down_phase -ceq $mode -and [string]$run.ready_phase -ceq "ready") "$name mode/phase differs from report mode"
        Assert-Issue154Response -Response $run.hide_response -ExpectedRequestId $expectedRequestId -ExpectedGenerationBefore $expectedGeneration -ExpectedGeneration $null -Name "$name.hide_response"
        Assert-Issue154DownSnapshot -Snapshot $run.down_snapshot -RequestId $expectedRequestId -Generation $expectedGeneration -ExpectedActiveRequestId $previousShowRequestId -Mode $mode -Name "$name.down_snapshot"
        Assert-Issue154ProcessSample -Sample $run.sample_after_5s -ExpectedReferenceIdentities $baselineReferenceIdentities -ExpectedRootIdentity $baselineRootIdentity -ExpectedRootProcessId $baselineRootProcessId -ExpectedRootExecutablePath $resolvedExecutable -ExpectedWebView2UserDataFolders $baselineWebView2UserDataFolders -Name "$name.sample_after_5s"
        Assert-Issue154ProcessSample -Sample $run.sample_after_30s -ExpectedReferenceIdentities $baselineReferenceIdentities -ExpectedRootIdentity $baselineRootIdentity -ExpectedRootProcessId $baselineRootProcessId -ExpectedRootExecutablePath $resolvedExecutable -ExpectedWebView2UserDataFolders $baselineWebView2UserDataFolders -Name "$name.sample_after_30s"
        $allObservedIdentities += @($run.sample_after_5s.process_identity_keys)
        $allObservedIdentities += @($run.sample_after_30s.process_identity_keys)
        $expectedRequestId++
        $expectedShowGeneration = if ($mode -ceq "destroyed") { $expectedGeneration + 1 } else { $expectedGeneration }
        Assert-Issue154Response -Response $run.show_response -ExpectedRequestId $expectedRequestId -ExpectedGenerationBefore $expectedGeneration -ExpectedGeneration $expectedShowGeneration -Name "$name.show_response"
        Assert-Issue154ReadySnapshot -Snapshot $run.ready_snapshot -RequestId $expectedRequestId -Generation $expectedShowGeneration -Mode $mode -Name "$name.ready_snapshot"
        $previousShowRequestId = $expectedRequestId
        $expectedRequestId++
        $expectedGeneration = $expectedShowGeneration
    }

    $lastRun = @($document.memory_runs)[@($document.memory_runs).Count - 1]
    Assert-Issue154Semantic (Test-Issue154Equal -Left $document.memory_after_5s -Right $lastRun.sample_after_5s) "memory_after_5s must be the last 5-second sample"
    Assert-Issue154Semantic (Test-Issue154Equal -Left $document.memory_after_30s -Right $lastRun.sample_after_30s) "memory_after_30s must be the last 30-second sample"
    Assert-Issue154Semantic (Test-Issue154StringSetEqual -Left @($document.observed_process_identities) -Right @($allObservedIdentities)) "observed_process_identities is not the union of all samples"
    Assert-Issue154Semantic (@($document.observed_process_identities).Count -eq @($allObservedIdentities | Sort-Object -Unique).Count) "observed_process_identities contains duplicates"

    foreach ($horizon in @("5s", "30s")) {
        $samples = if ($horizon -ceq "5s") { @($document.memory_runs | ForEach-Object { $_.sample_after_5s }) } else { @($document.memory_runs | ForEach-Object { $_.sample_after_30s }) }
        $median = if ($horizon -ceq "5s") { $document.memory_medians.after_5s } else { $document.memory_medians.after_30s }
        $prefix = if ($horizon -ceq "5s") { "after_5s" } else { "after_30s" }
        $privateBytes = Get-Issue154Median -Values @($samples | ForEach-Object { [double]$_.private_working_set_bytes })
        $commitBytes = Get-Issue154Median -Values @($samples | ForEach-Object { [double]$_.commit_bytes })
        $workingBytes = Get-Issue154Median -Values @($samples | ForEach-Object { [double]$_.working_set_bytes })
        $processCount = Get-Issue154Median -Values @($samples | ForEach-Object { [double]$_.process_count })
        Assert-Issue154NumberEqual -Expected $privateBytes -Actual ([double]$median.private_working_set_bytes) -Name "memory_medians.$prefix.private_working_set_bytes" -Tolerance 0
        Assert-Issue154NumberEqual -Expected ([Math]::Round($privateBytes / 1048576.0, 1)) -Actual ([double]$median.private_working_set_mib) -Name "memory_medians.$prefix.private_working_set_mib"
        Assert-Issue154NumberEqual -Expected $commitBytes -Actual ([double]$median.commit_bytes) -Name "memory_medians.$prefix.commit_bytes" -Tolerance 0
        Assert-Issue154NumberEqual -Expected ([Math]::Round($commitBytes / 1048576.0, 1)) -Actual ([double]$median.commit_mib) -Name "memory_medians.$prefix.commit_mib"
        Assert-Issue154NumberEqual -Expected $workingBytes -Actual ([double]$median.working_set_bytes) -Name "memory_medians.$prefix.working_set_bytes" -Tolerance 0
        Assert-Issue154NumberEqual -Expected ([Math]::Round($workingBytes / 1048576.0, 1)) -Actual ([double]$median.working_set_mib) -Name "memory_medians.$prefix.working_set_mib"
        Assert-Issue154NumberEqual -Expected $processCount -Actual ([double]$median.process_count) -Name "memory_medians.$prefix.process_count"
    }

    $medianLatency = [Math]::Round((Get-Issue154Median -Values $latencies), 1)
    $worstFive = @($latencies | Sort-Object -Descending | Select-Object -First 5)
    $maxLatency = [Math]::Round((($latencies | Measure-Object -Maximum).Maximum), 1)
    Assert-Issue154NumberEqual -Expected $medianLatency -Actual ([double]$document.summary.median_requested_visible_focused_hydrated_search_ready_ms) -Name "summary median latency"
    Assert-Issue154Semantic (Test-Issue154Equal -Left $worstFive -Right @($document.summary.worst_five_requested_visible_focused_hydrated_search_ready_ms)) "summary worst five latencies are incorrect"
    Assert-Issue154NumberEqual -Expected $maxLatency -Actual ([double]$document.summary.max_requested_visible_focused_hydrated_search_ready_ms) -Name "summary max latency"
    Assert-Issue154Semantic ($medianLatency -le [double]$document.protocol.median_latency_threshold_ms -and @($worstFive | Where-Object { $_ -gt [double]$document.protocol.worst_five_latency_threshold_ms }).Count -eq 0) "latency thresholds did not pass"
    foreach ($gate in @("latency_gate_pass", "lifecycle_gate_pass", "clipboard_gate_pass", "single_mode_functional_pass", "windows_11_real_machine_run")) {
        Assert-Issue154Boolean -Value $document.summary.$gate -Expected $true -Name "summary.$gate"
    }
    Assert-Issue154NumberEqual -Expected ([double]$document.memory_medians.after_5s.private_working_set_bytes) -Actual ([double]$document.summary.memory_median_after_5s_private_working_set_bytes) -Name "summary 5-second private median" -Tolerance 0
    Assert-Issue154NumberEqual -Expected ([double]$document.memory_medians.after_30s.private_working_set_bytes) -Actual ([double]$document.summary.memory_median_after_30s_private_working_set_bytes) -Name "summary 30-second private median" -Tolerance 0
    Assert-Issue154NumberEqual -Expected ([double]$document.memory_medians.after_5s.private_working_set_mib) -Actual ([double]$document.summary.memory_median_after_5s_private_working_set_mib) -Name "summary 5-second private MiB median"
    Assert-Issue154NumberEqual -Expected ([double]$document.memory_medians.after_30s.private_working_set_mib) -Actual ([double]$document.summary.memory_median_after_30s_private_working_set_mib) -Name "summary 30-second private MiB median"
    Assert-Issue154Semantic ([string]$document.summary.paired_hidden_vs_destroyed_comparison -ceq "pending" -and $null -eq $document.summary.paired_memory_gate_pass -and $null -eq $document.summary.overall_pass) "single report must not claim paired or overall pass"

    $finalRequestId = [uint64]$previousShowRequestId
    Assert-Issue154ReadySnapshot -Snapshot $document.final_snapshot -RequestId $finalRequestId -Generation $expectedGeneration -Mode $mode -Name "final_snapshot"
    Assert-Issue154Boolean -Value $document.final_snapshot.worker_running -Expected $false -Name "final_snapshot.worker_running"
    Assert-Issue154Array -Value $document.final_traces -Name "final_traces"
    Assert-Issue154FinalTraces -Traces @($document.final_traces) -FinalSnapshot $document.final_snapshot -FinalRequestId $finalRequestId -FinalGeneration $expectedGeneration -Mode $mode

    $finalStreamHash = Get-Issue154LockedFileSha256 -Stream $executableStream
    $finalPathHash = (Get-FileHash -LiteralPath $resolvedExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-Issue154Semantic ($finalStreamHash -ceq $hash -and $finalPathHash -ceq $hash) "measured executable changed or was replaced during validation"

    $result = [ordered]@{
        valid = $true
        path = $resolvedReport
        sha256 = (Get-FileHash -LiteralPath $resolvedReport -Algorithm SHA256).Hash.ToLowerInvariant()
        mode = $mode
        executable_sha256 = $hash
        executable_file_sha256_at_validation = $actualExecutableHash
        cycles = @($document.cycles).Count
        memory_runs = @($document.memory_runs).Count
        webview2_user_data_folders = $baselineWebView2UserDataFolders
        process_identity_key = [string]$document.protocol.process_identity_key
        process_attribution_rule = [string]$document.protocol.process_attribution_rule
        median_after_5s_private_working_set_bytes = [int64]$document.memory_medians.after_5s.private_working_set_bytes
        median_after_30s_private_working_set_bytes = [int64]$document.memory_medians.after_30s.private_working_set_bytes
        median_after_5s_commit_bytes = [int64]$document.memory_medians.after_5s.commit_bytes
        median_after_30s_commit_bytes = [int64]$document.memory_medians.after_30s.commit_bytes
    }
} finally {
    $executableStream.Dispose()
}

if ($PassThru) {
    $result
} else {
    ConvertTo-Json -InputObject $result -Depth 10
}
