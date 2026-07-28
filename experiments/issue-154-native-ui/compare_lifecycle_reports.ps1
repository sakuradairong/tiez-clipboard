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
    if (-not [bool]$validation.valid) {
        throw "Report '$resolved' failed semantic validation."
    }
    $report = [IO.File]::ReadAllText($resolved, [Text.Encoding]::UTF8) | ConvertFrom-Json
    if ([int]$report.schema_version -ne 2) {
        throw "Report '$resolved' must use schema_version 2."
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

    $hiddenJson = $HiddenValue | ConvertTo-Json -Compress -Depth 20
    $destroyedJson = $DestroyedValue | ConvertTo-Json -Compress -Depth 20
    if ($hiddenJson -cne $destroyedJson) {
        throw "Reports are not comparable: '$Name' differs. hidden=$hiddenJson destroyed=$destroyedJson"
    }
}

function Get-Issue154Reduction {
    param(
        [double]$HiddenBytes,
        [double]$DestroyedBytes,
        [double]$PercentThreshold,
        [double]$MiBThreshold
    )

    if ($HiddenBytes -le 0) {
        throw "Hidden private working set must be greater than zero."
    }
    $reductionBytes = $HiddenBytes - $DestroyedBytes
    $reductionMiB = $reductionBytes / 1048576.0
    $reductionPercent = ($reductionBytes / $HiddenBytes) * 100.0
    return [ordered]@{
        hidden_private_working_set_bytes = [int64]$HiddenBytes
        destroyed_private_working_set_bytes = [int64]$DestroyedBytes
        reduction_bytes = [int64]$reductionBytes
        reduction_mib = [Math]::Round($reductionMiB, 1)
        reduction_percent = [Math]::Round($reductionPercent, 1)
        percent_threshold = $PercentThreshold
        mib_threshold = $MiBThreshold
        pass = [bool]($reductionPercent -ge $PercentThreshold -and $reductionMiB -ge $MiBThreshold)
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
if (-not [bool]$hiddenReportData.summary.single_mode_functional_pass) {
    throw "Hidden report did not pass its single-mode functional gates."
}
if (-not [bool]$destroyedReportData.summary.single_mode_functional_pass) {
    throw "Destroyed report did not pass its single-mode functional gates."
}
if ([int]$hiddenReportData.protocol.cycles -lt 100 -or [int]$destroyedReportData.protocol.cycles -lt 100) {
    throw "Both reports must contain at least 100 lifecycle cycles."
}
if ([int]$hiddenReportData.protocol.memory_runs -lt 5 -or [int]$destroyedReportData.protocol.memory_runs -lt 5) {
    throw "Both reports must contain at least five complete memory runs."
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

$after5s = Get-Issue154Reduction `
    -HiddenBytes ([double]$hidden.validation.median_after_5s_private_working_set_bytes) `
    -DestroyedBytes ([double]$destroyed.validation.median_after_5s_private_working_set_bytes) `
    -PercentThreshold $MemoryReductionPercentThreshold `
    -MiBThreshold $MemoryReductionMiBThreshold
$after30s = Get-Issue154Reduction `
    -HiddenBytes ([double]$hidden.validation.median_after_30s_private_working_set_bytes) `
    -DestroyedBytes ([double]$destroyed.validation.median_after_30s_private_working_set_bytes) `
    -PercentThreshold $MemoryReductionPercentThreshold `
    -MiBThreshold $MemoryReductionMiBThreshold
$pairedMemoryGatePass = [bool]($after5s.pass -and $after30s.pass)

$document = [ordered]@{
    schema_version = 1
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
    after_5s = $after5s
    after_30s = $after30s
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
[IO.File]::WriteAllText($outputPath, $json, $utf8NoBom)
$pairValidatorPath = Join-Path $PSScriptRoot "validate_lifecycle_pair.ps1"
& $pairValidatorPath -PairReport $outputPath | Out-Null
$json
