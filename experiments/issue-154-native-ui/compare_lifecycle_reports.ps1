[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$HiddenReport,

    [Parameter(Mandatory = $true)]
    [string]$DestroyedReport,

    [ValidateRange(40, 100)]
    [double]$MemoryReductionPercentThreshold = 40,

    [ValidateRange(50, 1048576)]
    [double]$MemoryReductionMiBThreshold = 50,

    [Parameter(Mandatory = $true)]
    [string]$Output
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Read-Issue154Report {
    param([string]$Path)

    $resolved = (Resolve-Path -LiteralPath $Path).Path
    $validatorPath = Join-Path $PSScriptRoot "validate_lifecycle_report.ps1"
    $validation = & $validatorPath -Report $resolved -PassThru
    if ($validation.valid -isnot [bool] -or -not $validation.valid) {
        throw "Report '$resolved' failed semantic validation."
    }
    $report = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($resolved, [Text.Encoding]::UTF8))
    if ([int]$report.schema_version -ne 3) {
        throw "Report '$resolved' must use schema_version 3."
    }
    return [ordered]@{
        path = $resolved
        sha256 = (Get-FileHash -LiteralPath $resolved -Algorithm SHA256).Hash.ToLowerInvariant()
        report = $report
        validation = $validation
    }
}

function Assert-Issue154Equal {
    param(
        [string]$Name,
        [object]$HiddenValue,
        [object]$DestroyedValue
    )

    $hiddenJson = ConvertTo-Json -InputObject $HiddenValue -Compress -Depth 20
    $destroyedJson = ConvertTo-Json -InputObject $DestroyedValue -Compress -Depth 20
    if ($hiddenJson -cne $destroyedJson) {
        throw "Reports are not comparable: '$Name' differs. hidden=$hiddenJson destroyed=$destroyedJson"
    }
}

function Get-Issue154Reduction {
    param(
        [ValidateSet("private_working_set", "commit")]
        [string]$Metric,
        [double]$HiddenBytes,
        [double]$DestroyedBytes,
        [double]$PercentThreshold,
        [double]$MiBThreshold,
        [bool]$RequireMiBThreshold
    )

    if ($HiddenBytes -le 0) {
        throw "Hidden $Metric bytes must be greater than zero."
    }
    $reductionBytes = $HiddenBytes - $DestroyedBytes
    $reductionMiB = $reductionBytes / 1048576.0
    $reductionPercent = ($reductionBytes / $HiddenBytes) * 100.0
    return [ordered]@{
        metric = $Metric
        hidden_bytes = [int64]$HiddenBytes
        destroyed_bytes = [int64]$DestroyedBytes
        reduction_bytes = [int64]$reductionBytes
        reduction_mib = [Math]::Round($reductionMiB, 1)
        reduction_percent = [Math]::Round($reductionPercent, 1)
        percent_threshold = $PercentThreshold
        mib_threshold = $MiBThreshold
        mib_threshold_required = $RequireMiBThreshold
        pass = [bool]($reductionPercent -ge $PercentThreshold -and (-not $RequireMiBThreshold -or $reductionMiB -ge $MiBThreshold))
    }
}

$hidden = Read-Issue154Report -Path $HiddenReport
$destroyed = Read-Issue154Report -Path $DestroyedReport
$hiddenReportData = $hidden.report
$destroyedReportData = $destroyed.report

if ([string]$hiddenReportData.mode -cne "hidden") {
    throw "HiddenReport must have mode 'hidden'."
}
if ([string]$destroyedReportData.mode -cne "destroyed") {
    throw "DestroyedReport must have mode 'destroyed'."
}
if ($hiddenReportData.summary.single_mode_functional_pass -isnot [bool] -or -not $hiddenReportData.summary.single_mode_functional_pass) {
    throw "Hidden report did not pass its single-mode functional gates."
}
if ($destroyedReportData.summary.single_mode_functional_pass -isnot [bool] -or -not $destroyedReportData.summary.single_mode_functional_pass) {
    throw "Destroyed report did not pass its single-mode functional gates."
}
if ([int]$hiddenReportData.protocol.cycles -lt 100 -or [int]$destroyedReportData.protocol.cycles -lt 100) {
    throw "Both reports must contain at least 100 lifecycle cycles."
}
if ([int]$hiddenReportData.protocol.memory_runs -lt 5 -or [int]$hiddenReportData.protocol.memory_runs -gt 99 -or ([int]$hiddenReportData.protocol.memory_runs % 2) -ne 1 -or
    [int]$destroyedReportData.protocol.memory_runs -lt 5 -or [int]$destroyedReportData.protocol.memory_runs -gt 99 -or ([int]$destroyedReportData.protocol.memory_runs % 2) -ne 1) {
    throw "Both reports must contain an odd number of complete memory runs from five through 99."
}
if (@($hiddenReportData.cycles).Count -ne [int]$hiddenReportData.protocol.cycles -or
    @($destroyedReportData.cycles).Count -ne [int]$destroyedReportData.protocol.cycles) {
    throw "A report's cycle array length does not match protocol.cycles."
}
if (@($hiddenReportData.memory_runs).Count -ne [int]$hiddenReportData.protocol.memory_runs -or
    @($destroyedReportData.memory_runs).Count -ne [int]$destroyedReportData.protocol.memory_runs) {
    throw "A report's memory_runs array length does not match protocol.memory_runs."
}

Assert-Issue154Equal -Name "executable_sha256" -HiddenValue $hiddenReportData.executable_sha256 -DestroyedValue $destroyedReportData.executable_sha256
Assert-Issue154Equal -Name "executable_arguments" -HiddenValue $hiddenReportData.executable_arguments -DestroyedValue $destroyedReportData.executable_arguments
if ($hiddenReportData.root_command_line.verified -isnot [bool] -or -not $hiddenReportData.root_command_line.verified -or
    $destroyedReportData.root_command_line.verified -isnot [bool] -or -not $destroyedReportData.root_command_line.verified) {
    throw "Both reports must contain verified root command-line evidence."
}
Assert-Issue154Equal -Name "main_window_title" -HiddenValue $hiddenReportData.main_window_title -DestroyedValue $destroyedReportData.main_window_title
Assert-Issue154Equal -Name "host.os_version" -HiddenValue $hiddenReportData.host.os_version -DestroyedValue $destroyedReportData.host.os_version
Assert-Issue154Equal -Name "host.os_build" -HiddenValue $hiddenReportData.host.os_build -DestroyedValue $destroyedReportData.host.os_build
Assert-Issue154Equal -Name "host.architecture" -HiddenValue $hiddenReportData.host.architecture -DestroyedValue $destroyedReportData.host.architecture
Assert-Issue154Equal -Name "host.processors" -HiddenValue $hiddenReportData.host.processors -DestroyedValue $destroyedReportData.host.processors
Assert-Issue154Equal -Name "host.physical_memory_bytes" -HiddenValue $hiddenReportData.host.physical_memory_bytes -DestroyedValue $destroyedReportData.host.physical_memory_bytes
Assert-Issue154Equal -Name "protocol.cycles" -HiddenValue $hiddenReportData.protocol.cycles -DestroyedValue $destroyedReportData.protocol.cycles
Assert-Issue154Equal -Name "protocol.memory_runs" -HiddenValue $hiddenReportData.protocol.memory_runs -DestroyedValue $destroyedReportData.protocol.memory_runs
Assert-Issue154Equal -Name "protocol.memory_samples_seconds" -HiddenValue $hiddenReportData.protocol.memory_samples_seconds -DestroyedValue $destroyedReportData.protocol.memory_samples_seconds
Assert-Issue154Equal -Name "protocol.memory_scope" -HiddenValue $hiddenReportData.protocol.memory_scope -DestroyedValue $destroyedReportData.protocol.memory_scope
Assert-Issue154Equal -Name "protocol.process_identity_key" -HiddenValue $hiddenReportData.protocol.process_identity_key -DestroyedValue $destroyedReportData.protocol.process_identity_key
Assert-Issue154Equal -Name "protocol.process_attribution_rule" -HiddenValue $hiddenReportData.protocol.process_attribution_rule -DestroyedValue $destroyedReportData.protocol.process_attribution_rule
$hiddenWebView2Folders = @($hidden.validation.webview2_user_data_folders | ForEach-Object { ([string]$_).ToLowerInvariant() } | Sort-Object -Unique)
$destroyedWebView2Folders = @($destroyed.validation.webview2_user_data_folders | ForEach-Object { ([string]$_).ToLowerInvariant() } | Sort-Object -Unique)
Assert-Issue154Equal -Name "baseline WebView2 user-data-dir scope" -HiddenValue $hiddenWebView2Folders -DestroyedValue $destroyedWebView2Folders

$manualContextFieldNames = @(
    "data_snapshot",
    "feature_flags",
    "service_configuration",
    "power_plan",
    "webview2_runtime_version",
    "foreground_application"
)
$manualContextComplete = $true
foreach ($fieldName in $manualContextFieldNames) {
    $hiddenValue = $hiddenReportData.manual_context.$fieldName
    $destroyedValue = $destroyedReportData.manual_context.$fieldName
    Assert-Issue154Equal -Name "manual_context.$fieldName" -HiddenValue $hiddenValue -DestroyedValue $destroyedValue
    if ([string]::IsNullOrWhiteSpace([string]$hiddenValue)) {
        $manualContextComplete = $false
    }
}

$privateAfter5s = Get-Issue154Reduction `
    -Metric "private_working_set" `
    -HiddenBytes ([double]$hidden.validation.median_after_5s_private_working_set_bytes) `
    -DestroyedBytes ([double]$destroyed.validation.median_after_5s_private_working_set_bytes) `
    -PercentThreshold $MemoryReductionPercentThreshold `
    -MiBThreshold $MemoryReductionMiBThreshold `
    -RequireMiBThreshold $true
$privateAfter30s = Get-Issue154Reduction `
    -Metric "private_working_set" `
    -HiddenBytes ([double]$hidden.validation.median_after_30s_private_working_set_bytes) `
    -DestroyedBytes ([double]$destroyed.validation.median_after_30s_private_working_set_bytes) `
    -PercentThreshold $MemoryReductionPercentThreshold `
    -MiBThreshold $MemoryReductionMiBThreshold `
    -RequireMiBThreshold $true
$commitAfter5s = Get-Issue154Reduction `
    -Metric "commit" `
    -HiddenBytes ([double]$hidden.validation.median_after_5s_commit_bytes) `
    -DestroyedBytes ([double]$destroyed.validation.median_after_5s_commit_bytes) `
    -PercentThreshold $MemoryReductionPercentThreshold `
    -MiBThreshold $MemoryReductionMiBThreshold `
    -RequireMiBThreshold $false
$commitAfter30s = Get-Issue154Reduction `
    -Metric "commit" `
    -HiddenBytes ([double]$hidden.validation.median_after_30s_commit_bytes) `
    -DestroyedBytes ([double]$destroyed.validation.median_after_30s_commit_bytes) `
    -PercentThreshold $MemoryReductionPercentThreshold `
    -MiBThreshold $MemoryReductionMiBThreshold `
    -RequireMiBThreshold $false
$pairedMemoryGatePass = [bool]($privateAfter5s.pass -and $privateAfter30s.pass -and $commitAfter5s.pass -and $commitAfter30s.pass)

$document = [ordered]@{
    schema_version = 2
    captured_at_utc = [DateTime]::UtcNow.ToString("o")
    executable_sha256 = [string]$hiddenReportData.executable_sha256
    hidden_report = [ordered]@{
        path = $hidden.path
        sha256 = $hidden.sha256
        captured_at_utc = $hiddenReportData.captured_at_utc
    }
    destroyed_report = [ordered]@{
        path = $destroyed.path
        sha256 = $destroyed.sha256
        captured_at_utc = $destroyedReportData.captured_at_utc
    }
    comparability = [ordered]@{
        exact_executable_hash = $true
        root_command_lines_verified = $true
        executable_arguments = $hiddenReportData.executable_arguments
        main_window_title = $hiddenReportData.main_window_title
        host_os_version = $hiddenReportData.host.os_version
        host_os_build = $hiddenReportData.host.os_build
        host_architecture = $hiddenReportData.host.architecture
        host_processors = $hiddenReportData.host.processors
        host_physical_memory_bytes = [int64]$hiddenReportData.host.physical_memory_bytes
        cycles = [int]$hiddenReportData.protocol.cycles
        memory_runs = [int]$hiddenReportData.protocol.memory_runs
        memory_samples_seconds = $hiddenReportData.protocol.memory_samples_seconds
        memory_scope = $hiddenReportData.protocol.memory_scope
        process_identity_key = $hiddenReportData.protocol.process_identity_key
        process_attribution_rule = $hiddenReportData.protocol.process_attribution_rule
        cross_run_process_identity_set_equality_required = $false
        webview2_user_data_folders = $hidden.validation.webview2_user_data_folders
        requires_manual_same_data_features_services_power_state_confirmation = $true
        manual_comparability_confirmed = $false
        manual_context_complete = [bool]$manualContextComplete
        manual_context = $hiddenReportData.manual_context
        process_explorer_etw_cross_check_required = $true
        process_explorer_etw_cross_check_confirmed = $false
    }
    thresholds = [ordered]@{
        reduction_percent = $MemoryReductionPercentThreshold
        reduction_mib = $MemoryReductionMiBThreshold
        horizons_seconds = @(5, 30)
    }
    after_5s = [ordered]@{
        private_working_set = $privateAfter5s
        commit = $commitAfter5s
    }
    after_30s = [ordered]@{
        private_working_set = $privateAfter30s
        commit = $commitAfter30s
    }
    paired_memory_gate_pass = $pairedMemoryGatePass
    overall_pass = $null
    overall_pass_note = "Set only in signed-off evidence after manual comparability, Process Explorer/ETW process-set cross-check, and all native accessibility/IME/focus/paste/background-service gates are independently confirmed."
}

$outputPath = [IO.Path]::GetFullPath($Output)
$outputDirectory = Split-Path -Parent $outputPath
if (-not [string]::IsNullOrEmpty($outputDirectory)) {
    [IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
}
$json = $document | ConvertTo-Json -Depth 30
$utf8NoBom = New-Object System.Text.UTF8Encoding $false
$temporaryOutputPath = Join-Path $outputDirectory (".{0}.{1}.json" -f ([IO.Path]::GetFileName($outputPath)), [Guid]::NewGuid().ToString("N"))
[IO.File]::WriteAllText($temporaryOutputPath, $json, $utf8NoBom)
$pairValidatorPath = Join-Path $PSScriptRoot "validate_lifecycle_pair.ps1"
$published = $false
try {
    & $pairValidatorPath -PairReport $temporaryOutputPath | Out-Null
    Move-Item -LiteralPath $temporaryOutputPath -Destination $outputPath -Force
    $published = $true
} finally {
    if (-not $published -and (Test-Path -LiteralPath $temporaryOutputPath)) {
        Remove-Item -LiteralPath $temporaryOutputPath -Force -ErrorAction SilentlyContinue
    }
}
$json
