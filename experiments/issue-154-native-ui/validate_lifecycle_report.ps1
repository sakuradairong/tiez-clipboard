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
        [bool]$Condition,
        [string]$Message
    )
    if (-not $Condition) {
        throw "Issue #154 lifecycle report semantic validation failed: $Message"
    }
}

function Test-Issue154Equal {
    param(
        [object]$Left,
        [object]$Right
    )
    return (($Left | ConvertTo-Json -Compress -Depth 50) -ceq ($Right | ConvertTo-Json -Compress -Depth 50))
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
        [string[]]$Left,
        [string[]]$Right
    )
    $leftJson = @($Left | Sort-Object -Unique) | ConvertTo-Json -Compress
    $rightJson = @($Right | Sort-Object -Unique) | ConvertTo-Json -Compress
    return $leftJson -ceq $rightJson
}

function Get-Issue154ProcessIdentityKey {
    param([object]$Process)
    return "{0}|{1}|{2}" -f $Process.role, $Process.executable_path, $Process.started_at_utc
}

function Assert-Issue154ProcessSample {
    param(
        [object]$Sample,
        [string[]]$ExpectedReferenceIdentities,
        [string]$Name
    )

    $processes = @($Sample.processes)
    Assert-Issue154Semantic ($processes.Count -gt 0) "$Name.processes must not be empty"
    $identityKeys = @($processes | ForEach-Object { Get-Issue154ProcessIdentityKey -Process $_ } | Sort-Object -Unique)
    Assert-Issue154Semantic (Test-Issue154StringSetEqual $identityKeys @($Sample.process_identity_keys)) "$Name.process_identity_keys do not match processes"
    Assert-Issue154Semantic (Test-Issue154StringSetEqual @($Sample.reference_process_identity_keys) $ExpectedReferenceIdentities) "$Name.reference_process_identity_keys do not match baseline"

    $added = @($identityKeys | Where-Object { $ExpectedReferenceIdentities -cnotcontains $_ })
    $missing = @($ExpectedReferenceIdentities | Where-Object { $identityKeys -cnotcontains $_ })
    Assert-Issue154Semantic (Test-Issue154StringSetEqual $added @($Sample.identities_added_from_reference)) "$Name.identities_added_from_reference is incorrect"
    Assert-Issue154Semantic (Test-Issue154StringSetEqual $missing @($Sample.identities_missing_from_reference)) "$Name.identities_missing_from_reference is incorrect"

    $descendantCount = @($processes | Where-Object { $_.attribution -ceq "descendant" }).Count
    $attributedWebView2Count = @($processes | Where-Object { $_.attribution -ceq "webview2_user_data_dir" }).Count
    Assert-Issue154Semantic ([int]$Sample.process_count -eq $processes.Count) "$Name.process_count is incorrect"
    Assert-Issue154Semantic ([int]$Sample.descendant_process_count -eq $descendantCount) "$Name.descendant_process_count is incorrect"
    Assert-Issue154Semantic ([int]$Sample.attributed_webview2_process_count -eq $attributedWebView2Count) "$Name.attributed_webview2_process_count is incorrect"

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
    Assert-Issue154Semantic ([string]$Snapshot.mode -ceq $Mode) "$Name.mode is incorrect"
    Assert-Issue154Semantic ([string]$Snapshot.phase -ceq "ready") "$Name.phase must be ready"
    Assert-Issue154Semantic ([uint64]$Snapshot.generation -eq $Generation) "$Name.generation is incorrect"
    Assert-Issue154Semantic ([uint64]$Snapshot.completed_request_id -eq $RequestId) "$Name.completed_request_id is incorrect"
    Assert-Issue154Semantic ($null -eq $Snapshot.failed_request_id) "$Name.failed_request_id must be null"
    Assert-Issue154Semantic ([bool]$Snapshot.main_window_present -and [bool]$Snapshot.main_window_visible -and [bool]$Snapshot.main_window_focused) "$Name native window is not present, visible, and focused"
    Assert-Issue154Semantic ([bool]$Snapshot.hydrated -and [bool]$Snapshot.search_ready -and [bool]$Snapshot.focused) "$Name readiness barriers are incomplete"
}

function Assert-Issue154DownSnapshot {
    param(
        [object]$Snapshot,
        [uint64]$RequestId,
        [uint64]$Generation,
        [string]$Mode,
        [string]$Name
    )
    Assert-Issue154Semantic ([string]$Snapshot.mode -ceq $Mode) "$Name.mode is incorrect"
    Assert-Issue154Semantic ([string]$Snapshot.phase -ceq $Mode) "$Name.phase is incorrect"
    Assert-Issue154Semantic ([uint64]$Snapshot.generation -eq $Generation) "$Name.generation is incorrect"
    Assert-Issue154Semantic ([uint64]$Snapshot.completed_request_id -eq $RequestId) "$Name.completed_request_id is incorrect"
    Assert-Issue154Semantic ($null -eq $Snapshot.failed_request_id) "$Name.failed_request_id must be null"
    Assert-Issue154Semantic (-not [bool]$Snapshot.main_window_visible -and -not [bool]$Snapshot.main_window_focused) "$Name must not be visible or focused"
    if ($Mode -ceq "hidden") {
        Assert-Issue154Semantic ([bool]$Snapshot.main_window_present) "$Name hidden window must remain present"
    } else {
        Assert-Issue154Semantic (-not [bool]$Snapshot.main_window_present) "$Name destroyed window must be absent"
    }
}

$resolvedReport = (Resolve-Path -LiteralPath $Report).Path
$document = [IO.File]::ReadAllText($resolvedReport, [Text.Encoding]::UTF8) | ConvertFrom-Json
Assert-Issue154Semantic ([int]$document.schema_version -eq 2) "schema_version must be 2"
$mode = [string]$document.mode
Assert-Issue154Semantic ($mode -ceq "hidden" -or $mode -ceq "destroyed") "mode must be hidden or destroyed"

$hash = [string]$document.executable_sha256
$hashObservations = $document.executable_hash_observations
Assert-Issue154Semantic ([string]$hashObservations.before_start -ceq $hash) "before_start executable hash differs"
Assert-Issue154Semantic ([string]$hashObservations.after_start -ceq $hash) "after_start executable hash differs"
Assert-Issue154Semantic ([string]$hashObservations.after_run -ceq $hash) "after_run executable hash differs"
Assert-Issue154Semantic ([int]$document.protocol.cycles -eq @($document.cycles).Count) "protocol.cycles does not match cycles length"
Assert-Issue154Semantic ([int]$document.protocol.memory_runs -eq @($document.memory_runs).Count) "protocol.memory_runs does not match memory_runs length"
Assert-Issue154Semantic ([int]$document.protocol.cycles -ge 100) "at least 100 cycles are required"
Assert-Issue154Semantic ([int]$document.protocol.memory_runs -ge 5) "at least five memory runs are required"
Assert-Issue154Semantic (Test-Issue154Equal @($document.protocol.memory_samples_seconds) @(5, 30)) "memory sample horizons must be exactly 5 and 30 seconds"

$baselineReferenceIdentities = @($document.baseline_memory_sample.process_identity_keys)
Assert-Issue154ProcessSample -Sample $document.baseline_memory_sample -ExpectedReferenceIdentities $baselineReferenceIdentities -Name "baseline_memory_sample"
Assert-Issue154Semantic (Test-Issue154StringSetEqual @($document.baseline_memory_sample.reference_process_identity_keys) $baselineReferenceIdentities) "baseline reference identities must be self-referential"

$latencies = @()
for ($index = 0; $index -lt @($document.cycles).Count; $index++) {
    $cycle = @($document.cycles)[$index]
    $cycleNumber = $index + 1
    $name = "cycles[$index]"
    Assert-Issue154Semantic ([int]$cycle.cycle -eq $cycleNumber) "$name.cycle must be consecutive"
    Assert-Issue154Semantic ([string]$cycle.requested_mode -ceq $mode) "$name.requested_mode differs from report mode"
    Assert-Issue154Semantic ([string]$cycle.down_phase -ceq $mode) "$name.down_phase differs from report mode"
    Assert-Issue154Semantic ([string]$cycle.ready_phase -ceq "ready") "$name.ready_phase must be ready"
    Assert-Issue154DownSnapshot -Snapshot $cycle.down_snapshot -RequestId ([uint64]$cycle.hide_response.request_id) -Generation ([uint64]$cycle.hide_response.generation_before) -Mode $mode -Name "$name.down_snapshot"
    Assert-Issue154Semantic ([uint64]$cycle.show_response.generation_before -eq [uint64]$cycle.down_snapshot.generation) "$name.show generation_before is incorrect"
    Assert-Issue154ReadySnapshot -Snapshot $cycle.ready_snapshot -RequestId ([uint64]$cycle.show_response.request_id) -Generation ([uint64]$cycle.show_response.expected_generation) -Mode $mode -Name "$name.ready_snapshot"
    Assert-Issue154Semantic ([uint64]$cycle.generation -eq [uint64]$cycle.show_response.expected_generation) "$name.generation is incorrect"
    Assert-Issue154Semantic ([string]$cycle.clipboard_token -ceq [string]$cycle.clipboard_probe.payload.token) "$name clipboard token differs from probe payload"
    Assert-Issue154Semantic ([uint64]$cycle.clipboard_probe.payload.clipboard_event_count_before -eq [uint64]$cycle.down_snapshot.clipboard_event_count) "$name clipboard baseline count is incorrect"
    Assert-Issue154Semantic ([uint64]$cycle.clipboard_probe.payload.clipboard_event_delta -eq ([uint64]$cycle.clipboard_probe.payload.clipboard_event_count - [uint64]$cycle.clipboard_probe.payload.clipboard_event_count_before)) "$name clipboard delta is incorrect"
    Assert-Issue154Semantic ([bool]$cycle.clipboard_activity_history_consistent -and [bool]$cycle.clipboard_probe.verification.ok) "$name clipboard proof did not pass"
    Assert-Issue154Semantic ([int]$cycle.clipboard_probe.payload.exact_history_match_count -eq 1 -and [bool]$cycle.clipboard_probe.payload.exact_history_match) "$name clipboard history match is not unique"
    $latency = [double]$cycle.ready_snapshot.requested_visible_focused_hydrated_search_ready_ms
    Assert-Issue154NumberEqual $latency ([double]$cycle.requested_visible_focused_hydrated_search_ready_ms) "$name latency"
    $latencies += $latency
}

$allObservedIdentities = @($baselineReferenceIdentities)
for ($index = 0; $index -lt @($document.memory_runs).Count; $index++) {
    $run = @($document.memory_runs)[$index]
    $runNumber = $index + 1
    $name = "memory_runs[$index]"
    Assert-Issue154Semantic ([int]$run.run -eq $runNumber) "$name.run must be consecutive"
    Assert-Issue154Semantic ([string]$run.requested_mode -ceq $mode -and [string]$run.down_phase -ceq $mode) "$name mode/phase differs from report mode"
    Assert-Issue154DownSnapshot -Snapshot $run.down_snapshot -RequestId ([uint64]$run.hide_response.request_id) -Generation ([uint64]$run.hide_response.generation_before) -Mode $mode -Name "$name.down_snapshot"
    Assert-Issue154ReadySnapshot -Snapshot $run.ready_snapshot -RequestId ([uint64]$run.show_response.request_id) -Generation ([uint64]$run.show_response.expected_generation) -Mode $mode -Name "$name.ready_snapshot"
    Assert-Issue154ProcessSample -Sample $run.sample_after_5s -ExpectedReferenceIdentities $baselineReferenceIdentities -Name "$name.sample_after_5s"
    Assert-Issue154ProcessSample -Sample $run.sample_after_30s -ExpectedReferenceIdentities $baselineReferenceIdentities -Name "$name.sample_after_30s"
    $allObservedIdentities += @($run.sample_after_5s.process_identity_keys)
    $allObservedIdentities += @($run.sample_after_30s.process_identity_keys)
}

$lastRun = @($document.memory_runs)[@($document.memory_runs).Count - 1]
Assert-Issue154Semantic (Test-Issue154Equal $document.memory_after_5s $lastRun.sample_after_5s) "memory_after_5s must be the last 5-second sample"
Assert-Issue154Semantic (Test-Issue154Equal $document.memory_after_30s $lastRun.sample_after_30s) "memory_after_30s must be the last 30-second sample"
Assert-Issue154Semantic (Test-Issue154StringSetEqual @($document.observed_process_identities) @($allObservedIdentities)) "observed_process_identities is not the union of all samples"

foreach ($horizon in @("5s", "30s")) {
    $samples = if ($horizon -ceq "5s") { @($document.memory_runs | ForEach-Object { $_.sample_after_5s }) } else { @($document.memory_runs | ForEach-Object { $_.sample_after_30s }) }
    $median = if ($horizon -ceq "5s") { $document.memory_medians.after_5s } else { $document.memory_medians.after_30s }
    $prefix = if ($horizon -ceq "5s") { "after_5s" } else { "after_30s" }
    $privateBytes = Get-Issue154Median @($samples | ForEach-Object { [double]$_.private_working_set_bytes })
    $commitBytes = Get-Issue154Median @($samples | ForEach-Object { [double]$_.commit_bytes })
    $workingBytes = Get-Issue154Median @($samples | ForEach-Object { [double]$_.working_set_bytes })
    $processCount = Get-Issue154Median @($samples | ForEach-Object { [double]$_.process_count })
    Assert-Issue154NumberEqual $privateBytes ([double]$median.private_working_set_bytes) "memory_medians.$prefix.private_working_set_bytes" 0
    Assert-Issue154NumberEqual ([Math]::Round($privateBytes / 1048576.0, 1)) ([double]$median.private_working_set_mib) "memory_medians.$prefix.private_working_set_mib"
    Assert-Issue154NumberEqual $commitBytes ([double]$median.commit_bytes) "memory_medians.$prefix.commit_bytes" 0
    Assert-Issue154NumberEqual ([Math]::Round($commitBytes / 1048576.0, 1)) ([double]$median.commit_mib) "memory_medians.$prefix.commit_mib"
    Assert-Issue154NumberEqual $workingBytes ([double]$median.working_set_bytes) "memory_medians.$prefix.working_set_bytes" 0
    Assert-Issue154NumberEqual ([Math]::Round($workingBytes / 1048576.0, 1)) ([double]$median.working_set_mib) "memory_medians.$prefix.working_set_mib"
    Assert-Issue154NumberEqual $processCount ([double]$median.process_count) "memory_medians.$prefix.process_count"
}

$medianLatency = [Math]::Round((Get-Issue154Median $latencies), 1)
$worstFive = @($latencies | Sort-Object -Descending | Select-Object -First 5)
$maxLatency = [Math]::Round((($latencies | Measure-Object -Maximum).Maximum), 1)
Assert-Issue154NumberEqual $medianLatency ([double]$document.summary.median_requested_visible_focused_hydrated_search_ready_ms) "summary median latency"
Assert-Issue154Semantic (Test-Issue154Equal $worstFive @($document.summary.worst_five_requested_visible_focused_hydrated_search_ready_ms)) "summary worst five latencies are incorrect"
Assert-Issue154NumberEqual $maxLatency ([double]$document.summary.max_requested_visible_focused_hydrated_search_ready_ms) "summary max latency"
Assert-Issue154Semantic ($medianLatency -le [double]$document.protocol.median_latency_threshold_ms -and $maxLatency -le [double]$document.protocol.worst_five_latency_threshold_ms) "latency thresholds did not pass"
Assert-Issue154Semantic ([bool]$document.summary.latency_gate_pass -and [bool]$document.summary.lifecycle_gate_pass -and [bool]$document.summary.clipboard_gate_pass -and [bool]$document.summary.single_mode_functional_pass) "single-mode functional gates must pass"
Assert-Issue154NumberEqual ([double]$document.memory_medians.after_5s.private_working_set_bytes) ([double]$document.summary.memory_median_after_5s_private_working_set_bytes) "summary 5-second private median" 0
Assert-Issue154NumberEqual ([double]$document.memory_medians.after_30s.private_working_set_bytes) ([double]$document.summary.memory_median_after_30s_private_working_set_bytes) "summary 30-second private median" 0

$result = [ordered]@{
    valid = $true
    path = $resolvedReport
    sha256 = (Get-FileHash -LiteralPath $resolvedReport -Algorithm SHA256).Hash.ToLowerInvariant()
    mode = $mode
    executable_sha256 = $hash
    cycles = @($document.cycles).Count
    memory_runs = @($document.memory_runs).Count
    median_after_5s_private_working_set_bytes = [int64]$document.memory_medians.after_5s.private_working_set_bytes
    median_after_30s_private_working_set_bytes = [int64]$document.memory_medians.after_30s.private_working_set_bytes
}

if ($PassThru) {
    $result
} else {
    $result | ConvertTo-Json -Depth 10
}
