[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$PairReport
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Issue154PairSemantic {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) {
        throw "Issue #154 paired report semantic validation failed: $Message"
    }
}

function Assert-Issue154PairNumberEqual {
    param([double]$Expected, [double]$Actual, [string]$Name, [double]$Tolerance = 0.05)
    Assert-Issue154PairSemantic ([Math]::Abs($Expected - $Actual) -le $Tolerance) "$Name must be $Expected, found $Actual"
}

function Read-Issue154PairInput {
    param([object]$Reference, [string]$ExpectedMode)
    $path = (Resolve-Path -LiteralPath ([string]$Reference.path)).Path
    $actualHash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-Issue154PairSemantic ($actualHash -ceq [string]$Reference.sha256) "$ExpectedMode input report SHA-256 differs from referenced file"
    $singleValidatorPath = Join-Path $PSScriptRoot "validate_lifecycle_report.ps1"
    $validation = & $singleValidatorPath -Report $path -PassThru
    Assert-Issue154PairSemantic ([bool]$validation.valid) "$ExpectedMode input failed single semantic validation"
    Assert-Issue154PairSemantic ([string]$validation.mode -ceq $ExpectedMode) "$ExpectedMode input mode is incorrect"
    $document = [IO.File]::ReadAllText($path, [Text.Encoding]::UTF8) | ConvertFrom-Json
    Assert-Issue154PairSemantic ([string]$Reference.captured_at_utc -ceq [string]$document.captured_at_utc) "$ExpectedMode captured_at_utc differs from source"
    return [ordered]@{ path = $path; hash = $actualHash; validation = $validation; document = $document }
}

function Assert-Issue154Reduction {
    param(
        [object]$Reduction,
        [int64]$HiddenBytes,
        [int64]$DestroyedBytes,
        [double]$PercentThreshold,
        [double]$MiBThreshold,
        [string]$Name
    )
    $reductionBytes = $HiddenBytes - $DestroyedBytes
    $rawReductionMiB = $reductionBytes / 1048576.0
    $rawReductionPercent = ($reductionBytes / [double]$HiddenBytes) * 100.0
    $reductionMiB = [Math]::Round($rawReductionMiB, 1)
    $reductionPercent = [Math]::Round($rawReductionPercent, 1)
    $pass = ($rawReductionPercent -ge $PercentThreshold -and $rawReductionMiB -ge $MiBThreshold)
    Assert-Issue154PairSemantic ([int64]$Reduction.hidden_private_working_set_bytes -eq $HiddenBytes) "$Name hidden bytes are incorrect"
    Assert-Issue154PairSemantic ([int64]$Reduction.destroyed_private_working_set_bytes -eq $DestroyedBytes) "$Name destroyed bytes are incorrect"
    Assert-Issue154PairSemantic ([int64]$Reduction.reduction_bytes -eq $reductionBytes) "$Name reduction bytes are incorrect"
    Assert-Issue154PairNumberEqual $reductionMiB ([double]$Reduction.reduction_mib) "$Name reduction MiB"
    Assert-Issue154PairNumberEqual $reductionPercent ([double]$Reduction.reduction_percent) "$Name reduction percent"
    Assert-Issue154PairNumberEqual $PercentThreshold ([double]$Reduction.percent_threshold) "$Name percent threshold" 0
    Assert-Issue154PairNumberEqual $MiBThreshold ([double]$Reduction.mib_threshold) "$Name MiB threshold" 0
    Assert-Issue154PairSemantic ([bool]$Reduction.pass -eq $pass) "$Name pass is incorrect"
    return $pass
}

function Test-Issue154PairEqual {
    param([object]$Left, [object]$Right)
    return (($Left | ConvertTo-Json -Compress -Depth 50) -ceq ($Right | ConvertTo-Json -Compress -Depth 50))
}

$resolvedPair = (Resolve-Path -LiteralPath $PairReport).Path
$pair = [IO.File]::ReadAllText($resolvedPair, [Text.Encoding]::UTF8) | ConvertFrom-Json
Assert-Issue154PairSemantic ([int]$pair.schema_version -eq 1) "schema_version must be 1"
$hidden = Read-Issue154PairInput -Reference $pair.hidden_report -ExpectedMode "hidden"
$destroyed = Read-Issue154PairInput -Reference $pair.destroyed_report -ExpectedMode "destroyed"
Assert-Issue154PairSemantic ([string]$hidden.validation.executable_sha256 -ceq [string]$destroyed.validation.executable_sha256) "input executable hashes differ"
Assert-Issue154PairSemantic ([string]$pair.executable_sha256 -ceq [string]$hidden.validation.executable_sha256) "top-level executable hash differs from inputs"

$comparabilitySources = [ordered]@{
    executable_arguments = $hidden.document.executable_arguments
    main_window_title = $hidden.document.main_window_title
    host_os_version = $hidden.document.host.os_version
    host_os_build = $hidden.document.host.os_build
    host_architecture = $hidden.document.host.architecture
    host_processors = $hidden.document.host.processors
    host_physical_memory_bytes = [int64]$hidden.document.host.physical_memory_bytes
    cycles = [int]$hidden.document.protocol.cycles
    memory_runs = [int]$hidden.document.protocol.memory_runs
    memory_samples_seconds = $hidden.document.protocol.memory_samples_seconds
    memory_scope = $hidden.document.protocol.memory_scope
}
foreach ($property in $comparabilitySources.Keys) {
    $hiddenSource = switch ($property) {
        "executable_arguments" { $hidden.document.executable_arguments }
        "main_window_title" { $hidden.document.main_window_title }
        "host_os_version" { $hidden.document.host.os_version }
        "host_os_build" { $hidden.document.host.os_build }
        "host_architecture" { $hidden.document.host.architecture }
        "host_processors" { $hidden.document.host.processors }
        "host_physical_memory_bytes" { [int64]$hidden.document.host.physical_memory_bytes }
        "cycles" { [int]$hidden.document.protocol.cycles }
        "memory_runs" { [int]$hidden.document.protocol.memory_runs }
        "memory_samples_seconds" { $hidden.document.protocol.memory_samples_seconds }
        "memory_scope" { $hidden.document.protocol.memory_scope }
    }
    $destroyedSource = switch ($property) {
        "executable_arguments" { $destroyed.document.executable_arguments }
        "main_window_title" { $destroyed.document.main_window_title }
        "host_os_version" { $destroyed.document.host.os_version }
        "host_os_build" { $destroyed.document.host.os_build }
        "host_architecture" { $destroyed.document.host.architecture }
        "host_processors" { $destroyed.document.host.processors }
        "host_physical_memory_bytes" { [int64]$destroyed.document.host.physical_memory_bytes }
        "cycles" { [int]$destroyed.document.protocol.cycles }
        "memory_runs" { [int]$destroyed.document.protocol.memory_runs }
        "memory_samples_seconds" { $destroyed.document.protocol.memory_samples_seconds }
        "memory_scope" { $destroyed.document.protocol.memory_scope }
    }
    Assert-Issue154PairSemantic (Test-Issue154PairEqual $hiddenSource $destroyedSource) "$property differs between source reports"
    Assert-Issue154PairSemantic (Test-Issue154PairEqual $pair.comparability.$property $hiddenSource) "comparability.$property differs from source"
}
Assert-Issue154PairSemantic ([bool]$pair.comparability.exact_executable_hash) "exact_executable_hash must be true"
Assert-Issue154PairSemantic ([bool]$pair.comparability.requires_manual_same_data_features_services_power_state_confirmation) "manual comparability requirement must be true"
Assert-Issue154PairSemantic ([bool]$pair.comparability.process_explorer_etw_cross_check_required) "process cross-check requirement must be true"

$manualFields = @("data_snapshot", "feature_flags", "service_configuration", "power_plan", "webview2_runtime_version", "foreground_application")
$manualComplete = $true
foreach ($field in $manualFields) {
    $hiddenValue = $hidden.document.manual_context.$field
    $destroyedValue = $destroyed.document.manual_context.$field
    Assert-Issue154PairSemantic (($hiddenValue | ConvertTo-Json -Compress) -ceq ($destroyedValue | ConvertTo-Json -Compress)) "manual_context.$field differs"
    if ([string]::IsNullOrWhiteSpace([string]$hiddenValue)) { $manualComplete = $false }
    Assert-Issue154PairSemantic (Test-Issue154PairEqual $pair.comparability.manual_context.$field $hiddenValue) "paired manual_context.$field differs from source"
}
Assert-Issue154PairSemantic ([string]$hidden.document.manual_context.note -ceq [string]$destroyed.document.manual_context.note) "manual_context.note differs"
Assert-Issue154PairSemantic ([string]$pair.comparability.manual_context.note -ceq [string]$hidden.document.manual_context.note) "paired manual_context.note differs from hidden source"
Assert-Issue154PairSemantic ([bool]$pair.comparability.manual_context_complete -eq $manualComplete) "manual_context_complete is incorrect"
Assert-Issue154PairSemantic (-not [bool]$pair.comparability.manual_comparability_confirmed) "manual comparability must remain unconfirmed"
Assert-Issue154PairSemantic (-not [bool]$pair.comparability.process_explorer_etw_cross_check_confirmed) "process cross-check must remain unconfirmed"

$percentThreshold = [double]$pair.thresholds.reduction_percent
$mibThreshold = [double]$pair.thresholds.reduction_mib
$after5Pass = Assert-Issue154Reduction -Reduction $pair.after_5s -HiddenBytes ([int64]$hidden.validation.median_after_5s_private_working_set_bytes) -DestroyedBytes ([int64]$destroyed.validation.median_after_5s_private_working_set_bytes) -PercentThreshold $percentThreshold -MiBThreshold $mibThreshold -Name "after_5s"
$after30Pass = Assert-Issue154Reduction -Reduction $pair.after_30s -HiddenBytes ([int64]$hidden.validation.median_after_30s_private_working_set_bytes) -DestroyedBytes ([int64]$destroyed.validation.median_after_30s_private_working_set_bytes) -PercentThreshold $percentThreshold -MiBThreshold $mibThreshold -Name "after_30s"
Assert-Issue154PairSemantic ([bool]$pair.paired_memory_gate_pass -eq ($after5Pass -and $after30Pass)) "paired_memory_gate_pass is incorrect"
Assert-Issue154PairSemantic ($null -eq $pair.overall_pass) "overall_pass must remain null until external gates are signed off"

[ordered]@{
    valid = $true
    path = $resolvedPair
    sha256 = (Get-FileHash -LiteralPath $resolvedPair -Algorithm SHA256).Hash.ToLowerInvariant()
    paired_memory_gate_pass = [bool]$pair.paired_memory_gate_pass
} | ConvertTo-Json -Depth 10
