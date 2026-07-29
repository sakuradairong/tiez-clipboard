[CmdletBinding()]
param(
    [string]$OutputDirectory,

    [switch]$SkipSchemaValidation
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Copy-Issue154Object {
    param([object]$Value)
    return (ConvertTo-Json -InputObject $Value -Depth 100 | ConvertFrom-Json)
}

function Write-Issue154Json {
    param([string]$Path, [object]$Value)
    $utf8NoBom = New-Object System.Text.UTF8Encoding $false
    [IO.File]::WriteAllText($Path, (ConvertTo-Json -InputObject $Value -Depth 100), $utf8NoBom)
}

function Get-Issue154Median {
    param([double[]]$Values)
    $sorted = @($Values | Sort-Object)
    $middle = [Math]::Floor($sorted.Count / 2)
    if (($sorted.Count % 2) -eq 1) { return [double]$sorted[$middle] }
    return ([double]$sorted[$middle - 1] + [double]$sorted[$middle]) / 2.0
}

function New-Issue154Snapshot {
    param(
        [string]$Mode,
        [string]$Phase,
        [uint64]$Generation,
        [Nullable[uint64]]$ActiveRequestId,
        [Nullable[uint64]]$CompletedRequestId,
        [uint64]$ClipboardEventCount,
        [Nullable[uint64]]$Latency = $null,
        [int]$TraceCount = 0
    )
    $ready = $Phase -ceq "ready"
    $present = $Mode -ceq "hidden" -or $ready
    return [ordered]@{
        enabled = $true
        mode = $Mode
        phase = $Phase
        generation = $Generation
        active_request_id = $ActiveRequestId
        completed_request_id = $CompletedRequestId
        failed_request_id = $null
        active_intent = if ($null -eq $ActiveRequestId) { $null } else { "test" }
        react_mounted = $ready
        hydrated = $ready
        search_ready = $ready
        search_results_settled = $ready
        focused = $ready
        requested_visible_focused_hydrated_search_ready_ms = $Latency
        main_window_count = if ($present) { 1 } else { 0 }
        main_window_present = $present
        main_window_visible = $ready
        main_window_focused = $ready
        clipboard_event_count = $ClipboardEventCount
        persisted_history_count = 1
        session_history_count = 0
        history_probe_available = $true
        explicit_exit_requested = $false
        worker_running = $false
        in_flight_target = $null
        pending_target = $null
        trace_count = $TraceCount
    }
}

function New-Issue154Process {
    param(
        [int]$ProcessId,
        [int]$ParentPid,
        [string]$Role,
        [string]$Attribution,
        [string]$StartedAtUtc,
        [int64]$PrivateBytes,
        [string]$UserDataFolder = $null,
        [string]$ApplicationExecutable = $script:FixtureExecutablePath
    )
    $webView2 = $Role -like "webview2-*"
    return [ordered]@{
        pid = $ProcessId
        parent_pid = $ParentPid
        name = if ($webView2) { "msedgewebview2.exe" } else { "TieZ" }
        role = $Role
        attribution = $Attribution
        started_at_utc = $StartedAtUtc
        executable_path = if ($webView2) { "C:\Program Files\WebView2\msedgewebview2.exe" } else { $ApplicationExecutable }
        webview2_user_data_folder = if ($webView2) { $UserDataFolder } else { $null }
        working_set_bytes = [int64]($PrivateBytes + 1048576)
        private_working_set_bytes = $PrivateBytes
        commit_bytes = [int64]($PrivateBytes + 2097152)
    }
}

function Get-Issue154IdentityKey {
    param([object]$Process)
    return "{0}|{1}|{2}|{3}" -f $Process.pid, $Process.role, $Process.executable_path, $Process.started_at_utc
}

function New-Issue154ProcessSample {
    param(
        [object[]]$Processes,
        [string[]]$ReferenceIdentities,
        [string]$UserDataFolder,
        [string]$CapturedAtUtc
    )
    $identities = @($Processes | ForEach-Object { Get-Issue154IdentityKey -Process $_ } | Sort-Object -Unique)
    $reference = if ($ReferenceIdentities.Count -gt 0) { @($ReferenceIdentities | Sort-Object -Unique) } else { $identities }
    $roleCounts = [ordered]@{}
    foreach ($process in $Processes) {
        if ($roleCounts.Contains([string]$process.role)) { $roleCounts[[string]$process.role]++ } else { $roleCounts[[string]$process.role] = 1 }
    }
    return [ordered]@{
        captured_at_utc = $CapturedAtUtc
        process_count = $Processes.Count
        descendant_process_count = @($Processes | Where-Object { $_.attribution -ceq "descendant" }).Count
        attributed_webview2_process_count = @($Processes | Where-Object { $_.attribution -ceq "webview2_user_data_dir" }).Count
        webview2_user_data_folders = @($UserDataFolder)
        process_identity_keys = $identities
        reference_process_identity_keys = $reference
        identities_added_from_reference = @($identities | Where-Object { $reference -cnotcontains $_ })
        identities_missing_from_reference = @($reference | Where-Object { $identities -cnotcontains $_ })
        role_counts = $roleCounts
        working_set_bytes = [int64](($Processes | ForEach-Object { [int64]$_.working_set_bytes } | Measure-Object -Sum).Sum)
        private_working_set_bytes = [int64](($Processes | ForEach-Object { [int64]$_.private_working_set_bytes } | Measure-Object -Sum).Sum)
        commit_bytes = [int64](($Processes | ForEach-Object { [int64]$_.commit_bytes } | Measure-Object -Sum).Sum)
        processes = $Processes
    }
}

function New-Issue154Trace {
    param(
        [uint64]$RequestId,
        [uint64]$Generation,
        [string]$Mode,
        [string]$Phase,
        [uint64]$BaseTimestamp,
        [uint64]$Elapsed
    )
    return [ordered]@{
        request_id = $RequestId
        generation = $Generation
        mode = $Mode
        intent = "test"
        phase = $Phase
        timestamp_unix_ms = $BaseTimestamp + $Elapsed
        elapsed_ms = $Elapsed
        main_window_count = if ($Phase -in @("visible", "focused", "hydrated", "search_ready", "ready")) { 1 } else { 0 }
        clipboard_event_count = 105
        persisted_history_count = 1
        session_history_count = 0
        detail = $null
    }
}

function New-Issue154Report {
    param(
        [string]$Mode,
        [string]$Executable,
        [string]$ExecutableSha256,
        [int64]$PrivateBytes,
        [int]$RootPid,
        [string]$CapturedAtUtc,
        [string]$UserDataFolder
    )
    $generation = [uint64]1
    $requestId = [uint64]1
    [Nullable[uint64]]$previousShowRequestId = $null
    $clipboardCount = [uint64]0
    $cycles = @()
    $latencies = @()
    for ($cycle = 1; $cycle -le 100; $cycle++) {
        $hideRequest = $requestId
        $down = New-Issue154Snapshot -Mode $Mode -Phase $Mode -Generation $generation -ActiveRequestId $previousShowRequestId -CompletedRequestId $hideRequest -ClipboardEventCount $clipboardCount
        $requestId++
        $showRequest = $requestId
        $readyGeneration = if ($Mode -ceq "destroyed") { $generation + 1 } else { $generation }
        $latency = [double](300 + $cycle)
        $clipboardCount++
        $ready = New-Issue154Snapshot -Mode $Mode -Phase "ready" -Generation $readyGeneration -ActiveRequestId $showRequest -CompletedRequestId $showRequest -ClipboardEventCount $clipboardCount -Latency ([uint64]$latency)
        $token = "issue154-$Mode-token-$cycle"
        $cycles += [ordered]@{
            cycle = $cycle
            requested_mode = $Mode
            hide_response = [ordered]@{ accepted = $true; request_id = $hideRequest; generation_before = $generation }
            down_phase = $Mode
            down_snapshot = $down
            clipboard_token = $token
            clipboard_probe = [ordered]@{
                attempts = 1
                elapsed_ms = 10.0
                verification = [ordered]@{
                    ok = $true
                    listener_event_increased = $true
                    persistent_history_exact_match = $true
                    session_history_exact_match = $false
                    exact_history_match_count = 1
                }
                payload = [ordered]@{
                    token = $token
                    clipboard_event_count = $clipboardCount
                    clipboard_event_count_before = $clipboardCount - 1
                    clipboard_event_delta = 1
                    listener_event_count_increased = $true
                    exact_history_match = $true
                    exact_history_match_count = 1
                    persisted_entry_id = $cycle
                    session_entry_id = $null
                }
            }
            show_response = [ordered]@{ accepted = $true; request_id = $showRequest; generation_before = $generation; expected_generation = $readyGeneration }
            ready_phase = "ready"
            ready_snapshot = $ready
            main_window_count = 1
            requested_visible_focused_hydrated_search_ready_ms = $latency
            show_to_ready_ms = $latency
            total_cycle_ms = $latency + 20
            clipboard_activity_history_consistent = $true
            generation = $readyGeneration
        }
        $latencies += $latency
        $previousShowRequestId = $showRequest
        $requestId++
        $generation = $readyGeneration
    }

    $rootStarted = if ($Mode -ceq "hidden") { "2026-01-02T00:00:00.0000000Z" } else { "2026-01-02T01:00:00.0000000Z" }
    $root = New-Issue154Process -ProcessId $RootPid -ParentPid 1 -Role "application" -Attribution "descendant" -StartedAtUtc $rootStarted -PrivateBytes 10485760
    $baselineWebView = New-Issue154Process -ProcessId ($RootPid + 1) -ParentPid $RootPid -Role "webview2-renderer" -Attribution "descendant" -StartedAtUtc $rootStarted -PrivateBytes ($PrivateBytes - 10485760) -UserDataFolder $UserDataFolder
    $baseline = New-Issue154ProcessSample -Processes @($root, $baselineWebView) -ReferenceIdentities @() -UserDataFolder $UserDataFolder -CapturedAtUtc $CapturedAtUtc
    $referenceIdentities = @($baseline.process_identity_keys)
    $memoryRuns = @()
    for ($run = 1; $run -le 5; $run++) {
        $hideRequest = $requestId
        $down = New-Issue154Snapshot -Mode $Mode -Phase $Mode -Generation $generation -ActiveRequestId $previousShowRequestId -CompletedRequestId $hideRequest -ClipboardEventCount $clipboardCount
        if ($run -eq 1) {
            $churnWebView = New-Issue154Process -ProcessId ($RootPid + 100) -ParentPid $RootPid -Role "webview2-renderer" -Attribution "webview2_user_data_dir" -StartedAtUtc "2026-01-02T02:00:00.0000000Z" -PrivateBytes ($PrivateBytes - 10485760) -UserDataFolder $UserDataFolder
            $processes5 = @($root, $churnWebView)
        } else {
            $processes5 = @($root, $baselineWebView)
        }
        $sample5 = New-Issue154ProcessSample -Processes $processes5 -ReferenceIdentities $referenceIdentities -UserDataFolder $UserDataFolder -CapturedAtUtc $CapturedAtUtc
        $sample30 = New-Issue154ProcessSample -Processes @($root, $baselineWebView) -ReferenceIdentities $referenceIdentities -UserDataFolder $UserDataFolder -CapturedAtUtc $CapturedAtUtc
        $requestId++
        $showRequest = $requestId
        $readyGeneration = if ($Mode -ceq "destroyed") { $generation + 1 } else { $generation }
        $ready = New-Issue154Snapshot -Mode $Mode -Phase "ready" -Generation $readyGeneration -ActiveRequestId $showRequest -CompletedRequestId $showRequest -ClipboardEventCount $clipboardCount -Latency 350
        $memoryRuns += [ordered]@{
            run = $run
            requested_mode = $Mode
            hide_response = [ordered]@{ accepted = $true; request_id = $hideRequest; generation_before = $generation }
            down_phase = $Mode
            down_snapshot = $down
            sample_after_5s = $sample5
            sample_after_30s = $sample30
            show_response = [ordered]@{ accepted = $true; request_id = $showRequest; generation_before = $generation; expected_generation = $readyGeneration }
            ready_phase = "ready"
            ready_snapshot = $ready
            show_to_ready_ms = 350.0
        }
        $previousShowRequestId = $showRequest
        $requestId++
        $generation = $readyGeneration
    }

    $private5 = @($memoryRuns | ForEach-Object { [double]$_.sample_after_5s.private_working_set_bytes })
    $private30 = @($memoryRuns | ForEach-Object { [double]$_.sample_after_30s.private_working_set_bytes })
    $working5 = @($memoryRuns | ForEach-Object { [double]$_.sample_after_5s.working_set_bytes })
    $working30 = @($memoryRuns | ForEach-Object { [double]$_.sample_after_30s.working_set_bytes })
    $commit5 = @($memoryRuns | ForEach-Object { [double]$_.sample_after_5s.commit_bytes })
    $commit30 = @($memoryRuns | ForEach-Object { [double]$_.sample_after_30s.commit_bytes })
    $process5 = @($memoryRuns | ForEach-Object { [double]$_.sample_after_5s.process_count })
    $process30 = @($memoryRuns | ForEach-Object { [double]$_.sample_after_30s.process_count })
    $private5Median = Get-Issue154Median $private5
    $private30Median = Get-Issue154Median $private30
    $memoryMedians = [ordered]@{
        after_5s = [ordered]@{
            private_working_set_bytes = [int64]$private5Median
            private_working_set_mib = [Math]::Round($private5Median / 1048576.0, 1)
            commit_bytes = [int64](Get-Issue154Median $commit5)
            commit_mib = [Math]::Round((Get-Issue154Median $commit5) / 1048576.0, 1)
            working_set_bytes = [int64](Get-Issue154Median $working5)
            working_set_mib = [Math]::Round((Get-Issue154Median $working5) / 1048576.0, 1)
            process_count = [Math]::Round((Get-Issue154Median $process5), 1)
        }
        after_30s = [ordered]@{
            private_working_set_bytes = [int64]$private30Median
            private_working_set_mib = [Math]::Round($private30Median / 1048576.0, 1)
            commit_bytes = [int64](Get-Issue154Median $commit30)
            commit_mib = [Math]::Round((Get-Issue154Median $commit30) / 1048576.0, 1)
            working_set_bytes = [int64](Get-Issue154Median $working30)
            working_set_mib = [Math]::Round((Get-Issue154Median $working30) / 1048576.0, 1)
            process_count = [Math]::Round((Get-Issue154Median $process30), 1)
        }
    }
    $finalRequest = [uint64]$previousShowRequestId
    $baseTimestamp = [uint64]1767312000000
    $finalTraces = @(
        New-Issue154Trace -RequestId $finalRequest -Generation $generation -Mode $Mode -Phase "requested" -BaseTimestamp $baseTimestamp -Elapsed 0
        New-Issue154Trace -RequestId $finalRequest -Generation $generation -Mode $Mode -Phase "visible" -BaseTimestamp $baseTimestamp -Elapsed 100
        New-Issue154Trace -RequestId $finalRequest -Generation $generation -Mode $Mode -Phase "focused" -BaseTimestamp $baseTimestamp -Elapsed 150
        New-Issue154Trace -RequestId $finalRequest -Generation $generation -Mode $Mode -Phase "hydrated" -BaseTimestamp $baseTimestamp -Elapsed 200
        New-Issue154Trace -RequestId $finalRequest -Generation $generation -Mode $Mode -Phase "search_ready" -BaseTimestamp $baseTimestamp -Elapsed 250
        New-Issue154Trace -RequestId $finalRequest -Generation $generation -Mode $Mode -Phase "ready" -BaseTimestamp $baseTimestamp -Elapsed 350
    )
    $finalSnapshot = New-Issue154Snapshot -Mode $Mode -Phase "ready" -Generation $generation -ActiveRequestId $finalRequest -CompletedRequestId $finalRequest -ClipboardEventCount $clipboardCount -Latency 350 -TraceCount $finalTraces.Count
    $observed = @(
        @($baseline.process_identity_keys)
        foreach ($memoryRun in $memoryRuns) {
            @($memoryRun.sample_after_5s.process_identity_keys)
            @($memoryRun.sample_after_30s.process_identity_keys)
        }
    ) | Sort-Object -Unique
    $observed = @($observed)
    $medianLatency = [Math]::Round((Get-Issue154Median $latencies), 1)
    $worstFive = @($latencies | Sort-Object -Descending | Select-Object -First 5)
    return [ordered]@{
        schema_version = 3
        label = "validator-fixture-$Mode"
        captured_at_utc = $CapturedAtUtc
        executable = $Executable
        executable_sha256 = $ExecutableSha256
        executable_hash_observations = [ordered]@{ before_start = $ExecutableSha256; after_start = $ExecutableSha256; after_run = $ExecutableSha256 }
        executable_arguments = @("--fixture")
        root_command_line = [ordered]@{ raw = "`"$Executable`" --fixture"; executable = $Executable; arguments = @("--fixture"); verified = $true }
        main_window_title = "TieZ"
        mode = $Mode
        protocol = [ordered]@{
            cycles = 100
            memory_runs = 5
            ipc_commands = @("lifecycle_test_hide", "lifecycle_test_show", "get_main_ui_lifecycle_clipboard_probe", "get_main_ui_lifecycle_snapshot", "get_main_ui_lifecycle_traces")
            transport = "fixture filesystem transport"
            harness_directory_runtime = "unique empty fixture directory"
            lifecycle_mode_env = "TIEZ_EXPERIMENT_MAIN_UI_LIFECYCLE"
            harness_dir_env = "TIEZ_INTERNAL_LIFECYCLE_HARNESS_DIR"
            exact_executable_hash_required_across_modes = $true
            max_visible_main_windows = 1
            latency_metric = "requested-visible-focused-hydrated-search_ready"
            median_latency_threshold_ms = 750
            worst_five_latency_threshold_ms = 1500
            memory_samples_seconds = @(5, 30)
            memory_sampling_state = "independent down-state fixture run"
            memory_scope = "root process plus recursively discovered descendants and msedgewebview2 processes sharing the baseline --user-data-dir"
            process_identity_key = "pid|role|executable_path|started_at_utc"
            process_attribution_rule = "root-descendants-plus-webview2-baseline-user-data-dir-v1"
            stable_root_identity_required = $true
            dynamic_process_identity_churn_allowed = $true
            descendant_tree_requires_process_explorer_etw_cross_check = $true
            paired_memory_gate = "pending; compare matched reports at >= 40 percent and >= 50 MiB"
            clipboard_check = "unique token while down"
            windows_11_required = $true
            linux_screening_only = $true
        }
        host = [ordered]@{
            os_caption = "Microsoft Windows 11 Pro"
            os_version = "10.0.22631"
            os_build = "22631"
            architecture = "64-bit"
            powershell = "fixture"
            processors = @("Fixture CPU")
            physical_memory_bytes = 17179869184
        }
        manual_context = [ordered]@{
            data_snapshot = "fixture-data"
            feature_flags = "fixture-flags"
            service_configuration = "fixture-services"
            power_plan = "balanced"
            webview2_runtime_version = "fixture-webview2"
            foreground_application = "fixture-editor"
            note = "Fixture-only comparability context; not Windows evidence."
        }
        baseline_ready_snapshot = New-Issue154Snapshot -Mode $Mode -Phase "ready" -Generation 1 -ActiveRequestId $null -CompletedRequestId $null -ClipboardEventCount 0
        baseline_memory_sample = $baseline
        cycles = $cycles
        memory_runs = $memoryRuns
        memory_medians = $memoryMedians
        memory_after_5s = $memoryRuns[-1].sample_after_5s
        memory_after_30s = $memoryRuns[-1].sample_after_30s
        observed_process_identities = $observed
        final_snapshot = $finalSnapshot
        final_traces = $finalTraces
        summary = [ordered]@{
            median_requested_visible_focused_hydrated_search_ready_ms = $medianLatency
            worst_five_requested_visible_focused_hydrated_search_ready_ms = $worstFive
            max_requested_visible_focused_hydrated_search_ready_ms = [Math]::Round((($latencies | Measure-Object -Maximum).Maximum), 1)
            memory_median_after_5s_private_working_set_bytes = [int64]$private5Median
            memory_median_after_5s_private_working_set_mib = [Math]::Round($private5Median / 1048576.0, 1)
            memory_median_after_30s_private_working_set_bytes = [int64]$private30Median
            memory_median_after_30s_private_working_set_mib = [Math]::Round($private30Median / 1048576.0, 1)
            paired_hidden_vs_destroyed_comparison = "pending"
            paired_memory_gate_pass = $null
            paired_memory_gate_note = "Fixture single-mode report."
            latency_gate_pass = $true
            lifecycle_gate_pass = $true
            clipboard_gate_pass = $true
            single_mode_functional_pass = $true
            overall_pass = $null
            windows_11_real_machine_run = $true
        }
    }
}

function Invoke-Issue154SchemaValidation {
    param([string]$Instance, [string]$Schema, [bool]$ShouldPass, [string]$Name)
    if ($SkipSchemaValidation) { return }
    $pythonCommand = Get-Command python3 -ErrorAction SilentlyContinue
    if ($null -eq $pythonCommand) {
        throw "python3 with jsonschema is required for schema validation; use -SkipSchemaValidation only when schemas will be checked separately."
    }
    $python = @'
import json, sys
from jsonschema import Draft7Validator, FormatChecker
with open(sys.argv[1], encoding="utf-8-sig") as f:
    instance = json.load(f)
with open(sys.argv[2], encoding="utf-8-sig") as f:
    schema = json.load(f)
errors = sorted(Draft7Validator(schema, format_checker=FormatChecker()).iter_errors(instance), key=lambda e: list(e.path))
if errors:
    print("; ".join(f"{'.'.join(map(str, e.path))}: {e.message}" for e in errors[:5]))
    sys.exit(1)
'@
    $pythonPath = Join-Path $script:ScratchDirectory "validate_schema.py"
    if (-not [IO.File]::Exists($pythonPath)) { [IO.File]::WriteAllText($pythonPath, $python, (New-Object Text.UTF8Encoding $false)) }
    & $pythonCommand.Source $pythonPath $Instance $Schema *> $null
    $passed = $LASTEXITCODE -eq 0
    if ($passed -ne $ShouldPass) { throw "$Name schema expectation failed. expected=$ShouldPass actual=$passed" }
}

function Invoke-Issue154SemanticValidation {
    param([string]$Instance, [bool]$ShouldPass, [string]$Name, [switch]$Pair)
    $scriptPath = if ($Pair) { Join-Path $PSScriptRoot "validate_lifecycle_pair.ps1" } else { Join-Path $PSScriptRoot "validate_lifecycle_report.ps1" }
    $passed = $true
    $parameters = if ($Pair) { @{ PairReport = $Instance } } else { @{ Report = $Instance } }
    try { & $scriptPath @parameters *> $null } catch { $passed = $false }
    if ($passed -ne $ShouldPass) { throw "$Name semantic expectation failed. expected=$ShouldPass actual=$passed" }
}

function Invoke-Issue154SingleMutant {
    param([string]$Name, [scriptblock]$Mutate, [bool]$SchemaRejects = $false, [bool]$SemanticRejects = $true)
    $document = Copy-Issue154Object -Value $script:HiddenReport
    & $Mutate $document
    $path = Join-Path $script:ScratchDirectory ("single-{0}.json" -f $Name)
    Write-Issue154Json -Path $path -Value $document
    Invoke-Issue154SchemaValidation -Instance $path -Schema $script:SingleSchema -ShouldPass (-not $SchemaRejects) -Name $Name
    Invoke-Issue154SemanticValidation -Instance $path -ShouldPass (-not $SemanticRejects) -Name $Name
    $script:MutationCount++
}

function Invoke-Issue154PairMutant {
    param([string]$Name, [scriptblock]$Mutate, [bool]$SchemaRejects = $false, [bool]$SemanticRejects = $true)
    $document = Copy-Issue154Object -Value $script:PairReport
    & $Mutate $document
    $path = Join-Path $script:ScratchDirectory ("pair-{0}.json" -f $Name)
    Write-Issue154Json -Path $path -Value $document
    Invoke-Issue154SchemaValidation -Instance $path -Schema $script:PairSchema -ShouldPass (-not $SchemaRejects) -Name $Name
    Invoke-Issue154SemanticValidation -Instance $path -ShouldPass (-not $SemanticRejects) -Name $Name -Pair
    $script:MutationCount++
}

$ownsScratch = [string]::IsNullOrWhiteSpace($OutputDirectory)
$script:ScratchDirectory = if ($ownsScratch) { Join-Path ([IO.Path]::GetTempPath()) ("issue154-validator-fixtures-{0}" -f ([Guid]::NewGuid().ToString("N"))) } else { [IO.Path]::GetFullPath($OutputDirectory) }
[IO.Directory]::CreateDirectory($script:ScratchDirectory) | Out-Null
$script:SingleSchema = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../../docs/performance/issue-154-lifecycle-report.schema.json"))
$script:PairSchema = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../../docs/performance/issue-154-lifecycle-pair.schema.json"))
$script:MutationCount = 0

try {
    $dummyExecutable = Join-Path $script:ScratchDirectory "TieZ-fixture.exe"
    [IO.File]::WriteAllBytes($dummyExecutable, [Text.Encoding]::ASCII.GetBytes("Issue 154 stable dummy executable fixture`n"))
    $script:FixtureExecutablePath = (Resolve-Path -LiteralPath $dummyExecutable).Path
    $dummyHash = (Get-FileHash -LiteralPath $dummyExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    $capturedAt = "2026-01-02T03:04:05.0000000Z"
    $userData = "C:\fixture\WebView2"
    $script:HiddenReport = New-Issue154Report -Mode "hidden" -Executable $dummyExecutable -ExecutableSha256 $dummyHash -PrivateBytes 209715200 -RootPid 4100 -CapturedAtUtc $capturedAt -UserDataFolder $userData
    $script:DestroyedReport = New-Issue154Report -Mode "destroyed" -Executable $dummyExecutable -ExecutableSha256 $dummyHash -PrivateBytes 94371840 -RootPid 5100 -CapturedAtUtc $capturedAt -UserDataFolder $userData
    $hiddenPath = Join-Path $script:ScratchDirectory "hidden.json"
    $destroyedPath = Join-Path $script:ScratchDirectory "destroyed.json"
    $pairPath = Join-Path $script:ScratchDirectory "pair.json"
    Write-Issue154Json -Path $hiddenPath -Value $script:HiddenReport
    Write-Issue154Json -Path $destroyedPath -Value $script:DestroyedReport
    Invoke-Issue154SchemaValidation -Instance $hiddenPath -Schema $script:SingleSchema -ShouldPass $true -Name "positive hidden"
    Invoke-Issue154SchemaValidation -Instance $destroyedPath -Schema $script:SingleSchema -ShouldPass $true -Name "positive destroyed"
    Invoke-Issue154SemanticValidation -Instance $hiddenPath -ShouldPass $true -Name "positive hidden"
    Invoke-Issue154SemanticValidation -Instance $destroyedPath -ShouldPass $true -Name "positive destroyed"
    & (Join-Path $PSScriptRoot "compare_lifecycle_reports.ps1") -HiddenReport $hiddenPath -DestroyedReport $destroyedPath -Output $pairPath *> $null
    Invoke-Issue154SchemaValidation -Instance $pairPath -Schema $script:PairSchema -ShouldPass $true -Name "positive pair"
    Invoke-Issue154SemanticValidation -Instance $pairPath -ShouldPass $true -Name "positive pair" -Pair
    $script:PairReport = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($pairPath, [Text.Encoding]::UTF8))

    Invoke-Issue154SingleMutant "top-mode" { param($d) $d.cycles[0].requested_mode = "destroyed" } $true
    Invoke-Issue154SingleMutant "protocol-length" { param($d) $d.protocol.cycles = 101 }
    Invoke-Issue154SingleMutant "cycle-sequence" { param($d) $d.cycles[1].cycle = 3 }
    Invoke-Issue154SingleMutant "request-id" { param($d) $d.cycles[0].hide_response.request_id = 9 }
    Invoke-Issue154SingleMutant "first-down-active" { param($d) $d.cycles[0].down_snapshot.active_request_id = 2; $d.cycles[0].down_snapshot.active_intent = "test" }
    Invoke-Issue154SingleMutant "hidden-generation" { param($d) $d.cycles[0].show_response.expected_generation++ }
    Invoke-Issue154SingleMutant "stale-ready" { param($d) $d.cycles[0].ready_snapshot.active_request_id = 999 }
    Invoke-Issue154SingleMutant "clipboard-token" { param($d) $d.cycles[0].clipboard_probe.payload.token = "different" }
    Invoke-Issue154SingleMutant "clipboard-baseline" { param($d) $d.cycles[0].clipboard_probe.payload.clipboard_event_count_before++ }
    Invoke-Issue154SingleMutant "clipboard-delta" { param($d) $d.cycles[0].clipboard_probe.payload.clipboard_event_delta = 2 }
    Invoke-Issue154SingleMutant "clipboard-backends" { param($d) $d.cycles[0].clipboard_probe.payload.session_entry_id = -1 }
    Invoke-Issue154SingleMutant "root-missing" { param($d) $d.memory_runs[0].sample_after_5s.processes[0].pid = 9999 }
    Invoke-Issue154SingleMutant "root-executable" { param($d) $d.memory_runs[0].sample_after_5s.processes[0].executable_path = "C:\other\TieZ.exe" }
    Invoke-Issue154SingleMutant "parent-tree" { param($d) $d.memory_runs[0].sample_after_30s.processes[1].parent_pid = 9999 }
    Invoke-Issue154SingleMutant "webview-scope" { param($d) $d.memory_runs[0].sample_after_5s.webview2_user_data_folders[0] = "C:\other" }
    Invoke-Issue154SingleMutant "identity-keys" { param($d) $d.memory_runs[0].sample_after_5s.process_identity_keys[0] = "forged" }
    Invoke-Issue154SingleMutant "identity-diff" { param($d) $d.memory_runs[0].sample_after_5s.identities_added_from_reference = @() }
    Invoke-Issue154SingleMutant "role-count" { param($d) $d.memory_runs[0].sample_after_5s.role_counts.application = 2 }
    Invoke-Issue154SingleMutant "memory-sum" { param($d) $d.memory_runs[0].sample_after_5s.private_working_set_bytes++ }
    Invoke-Issue154SingleMutant "median" { param($d) $d.memory_medians.after_5s.private_working_set_bytes++ }
    Invoke-Issue154SingleMutant "summary" { param($d) $d.summary.max_requested_visible_focused_hydrated_search_ready_ms++ }
    Invoke-Issue154SingleMutant "final-snapshot" { param($d) $d.final_snapshot.completed_request_id-- }
    Invoke-Issue154SingleMutant "final-traces" { param($d) $d.final_traces[5].phase = "visible" }
    Invoke-Issue154SingleMutant "host-build" { param($d) $d.host.os_build = "19045" } $true
    Invoke-Issue154SingleMutant "extra-property" { param($d) $d.cycles[0] | Add-Member -NotePropertyName forged -NotePropertyValue 1 } $true $false
    Invoke-Issue154SingleMutant "string-boolean" { param($d) $d.summary.single_mode_functional_pass = "false" } $true
    Invoke-Issue154SingleMutant "single-element-array" { param($d) $d.executable_arguments = @("--different") }
    Invoke-Issue154SingleMutant "executable-hash-observation" { param($d) $d.executable_hash_observations.after_run = ("0" * 64) }

    $replacementPath = Join-Path $script:ScratchDirectory "single-executable-replaced.json"
    Write-Issue154Json -Path $replacementPath -Value $script:HiddenReport
    [IO.File]::WriteAllBytes($dummyExecutable, [Text.Encoding]::ASCII.GetBytes("replacement executable bytes`n"))
    Invoke-Issue154SemanticValidation -Instance $replacementPath -ShouldPass $false -Name "executable replaced before validation"
    [IO.File]::WriteAllBytes($dummyExecutable, [Text.Encoding]::ASCII.GetBytes("Issue 154 stable dummy executable fixture`n"))
    $script:MutationCount++

    Invoke-Issue154PairMutant "source-hash" { param($d) $d.hidden_report.sha256 = ("0" * 64) }
    Invoke-Issue154PairMutant "executable-hash" { param($d) $d.executable_sha256 = ("1" * 64) }
    Invoke-Issue154PairMutant "user-data-scope" { param($d) $d.comparability.webview2_user_data_folders[0] = "C:\other" }
    Invoke-Issue154PairMutant "cross-run-identity-equality" { param($d) $d.comparability.cross_run_process_identity_set_equality_required = $true } $true
    Invoke-Issue154PairMutant "reduction-bytes" { param($d) $d.after_5s.private_working_set.reduction_bytes++ }
    Invoke-Issue154PairMutant "commit-reduction" { param($d) $d.after_30s.commit.reduction_percent++ }
    Invoke-Issue154PairMutant "threshold" { param($d) $d.thresholds.reduction_percent = 39 } $true
    Invoke-Issue154PairMutant "manual-completeness" { param($d) $d.comparability.manual_context_complete = $false }
    Invoke-Issue154PairMutant "manual-confirmed" { param($d) $d.comparability.manual_comparability_confirmed = $true } $true
    Invoke-Issue154PairMutant "overall-pass" { param($d) $d.overall_pass = $true } $true
    Invoke-Issue154PairMutant "pair-string-boolean" { param($d) $d.paired_memory_gate_pass = "true" } $true
    Invoke-Issue154PairMutant "pair-extra-property" { param($d) $d.comparability | Add-Member -NotePropertyName forged -NotePropertyValue 1 } $true $false

    ConvertTo-Json -InputObject ([ordered]@{
        valid = $true
        positive_reports = 2
        positive_pairs = 1
        adversarial_mutations = $script:MutationCount
        dynamic_identity_churn_positive = $true
        output_directory = if ($ownsScratch) { $null } else { $script:ScratchDirectory }
        note = "Generated fixtures are logic/schema tests only and are not Windows 11/WebView2 evidence."
    }) -Depth 5
} finally {
    if ($ownsScratch -and [IO.Directory]::Exists($script:ScratchDirectory)) {
        Remove-Item -LiteralPath $script:ScratchDirectory -Recurse -Force
    }
}
