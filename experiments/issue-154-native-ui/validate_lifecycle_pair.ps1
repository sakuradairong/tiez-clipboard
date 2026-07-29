[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$PairReport
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Issue154PairSemantic {
    param([object]$Condition, [string]$Message)
    if ($Condition -isnot [bool] -or -not $Condition) {
        throw "Issue #154 paired report semantic validation failed: $Message"
    }
}

function Assert-Issue154PairBoolean {
    param([object]$Value, [bool]$Expected, [string]$Name)
    Assert-Issue154PairSemantic ($Value -is [bool]) "$Name must be a JSON boolean"
    Assert-Issue154PairSemantic ($Value -eq $Expected) "$Name must be $($Expected.ToString().ToLowerInvariant())"
}

function Assert-Issue154PairNumberEqual {
    param([double]$Expected, [double]$Actual, [string]$Name, [double]$Tolerance = 0.05)
    Assert-Issue154PairSemantic ([Math]::Abs($Expected - $Actual) -le $Tolerance) "$Name must be $Expected, found $Actual"
}

function Test-Issue154PairEqual {
    param([object]$Left, [object]$Right)
    $leftJson = ConvertTo-Json -InputObject $Left -Compress -Depth 50
    $rightJson = ConvertTo-Json -InputObject $Right -Compress -Depth 50
    return $leftJson -ceq $rightJson
}

function Test-Issue154PairStringSetEqual {
    param([object[]]$Left, [object[]]$Right)
    $leftValues = @($Left | ForEach-Object { ([string]$_).ToLowerInvariant() } | Sort-Object -Unique)
    $rightValues = @($Right | ForEach-Object { ([string]$_).ToLowerInvariant() } | Sort-Object -Unique)
    return Test-Issue154PairEqual -Left $leftValues -Right $rightValues
}

function Read-Issue154PairInput {
    param([object]$Reference, [string]$ExpectedMode)
    $path = (Resolve-Path -LiteralPath ([string]$Reference.path)).Path
    $actualHash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-Issue154PairSemantic ($actualHash -ceq [string]$Reference.sha256) "$ExpectedMode input report SHA-256 differs from referenced file"
    $singleValidatorPath = Join-Path $PSScriptRoot "validate_lifecycle_report.ps1"
    $validation = & $singleValidatorPath -Report $path -PassThru
    Assert-Issue154PairBoolean -Value $validation.valid -Expected $true -Name "$ExpectedMode validation.valid"
    Assert-Issue154PairSemantic ([string]$validation.mode -ceq $ExpectedMode) "$ExpectedMode input mode is incorrect"
    $document = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($path, [Text.Encoding]::UTF8))
    Assert-Issue154PairSemantic ([string]$Reference.captured_at_utc -ceq [string]$document.captured_at_utc) "$ExpectedMode captured_at_utc differs from source"
    return [ordered]@{ path = $path; hash = $actualHash; validation = $validation; document = $document }
}

function Assert-Issue154Reduction {
    param(
        [object]$Reduction,
        [ValidateSet("private_working_set", "commit")]
        [string]$Metric,
        [int64]$HiddenBytes,
        [int64]$DestroyedBytes,
        [double]$PercentThreshold,
        [double]$MiBThreshold,
        [bool]$RequireMiBThreshold,
        [string]$Name
    )
    Assert-Issue154PairSemantic ($HiddenBytes -gt 0) "$Name hidden bytes must be positive"
    $reductionBytes = $HiddenBytes - $DestroyedBytes
    $rawReductionMiB = $reductionBytes / 1048576.0
    $rawReductionPercent = ($reductionBytes / [double]$HiddenBytes) * 100.0
    $reductionMiB = [Math]::Round($rawReductionMiB, 1)
    $reductionPercent = [Math]::Round($rawReductionPercent, 1)
    $pass = ($rawReductionPercent -ge $PercentThreshold -and (-not $RequireMiBThreshold -or $rawReductionMiB -ge $MiBThreshold))
    Assert-Issue154PairSemantic ([string]$Reduction.metric -ceq $Metric) "$Name metric is incorrect"
    Assert-Issue154PairSemantic ([int64]$Reduction.hidden_bytes -eq $HiddenBytes) "$Name hidden bytes are incorrect"
    Assert-Issue154PairSemantic ([int64]$Reduction.destroyed_bytes -eq $DestroyedBytes) "$Name destroyed bytes are incorrect"
    Assert-Issue154PairSemantic ([int64]$Reduction.reduction_bytes -eq $reductionBytes) "$Name reduction bytes are incorrect"
    Assert-Issue154PairNumberEqual -Expected $reductionMiB -Actual ([double]$Reduction.reduction_mib) -Name "$Name reduction MiB"
    Assert-Issue154PairNumberEqual -Expected $reductionPercent -Actual ([double]$Reduction.reduction_percent) -Name "$Name reduction percent"
    Assert-Issue154PairNumberEqual -Expected $PercentThreshold -Actual ([double]$Reduction.percent_threshold) -Name "$Name percent threshold" -Tolerance 0
    Assert-Issue154PairNumberEqual -Expected $MiBThreshold -Actual ([double]$Reduction.mib_threshold) -Name "$Name MiB threshold" -Tolerance 0
    Assert-Issue154PairBoolean -Value $Reduction.mib_threshold_required -Expected $RequireMiBThreshold -Name "$Name MiB threshold requirement"
    Assert-Issue154PairBoolean -Value $Reduction.pass -Expected $pass -Name "$Name pass"
    return $pass
}

$resolvedPair = (Resolve-Path -LiteralPath $PairReport).Path
$pair = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($resolvedPair, [Text.Encoding]::UTF8))
Assert-Issue154PairSemantic ([int]$pair.schema_version -eq 2) "schema_version must be 2"
$hidden = Read-Issue154PairInput -Reference $pair.hidden_report -ExpectedMode "hidden"
$destroyed = Read-Issue154PairInput -Reference $pair.destroyed_report -ExpectedMode "destroyed"
Assert-Issue154PairSemantic ([string]$hidden.validation.executable_sha256 -ceq [string]$destroyed.validation.executable_sha256) "input executable hashes differ"
Assert-Issue154PairSemantic ([string]$pair.executable_sha256 -ceq [string]$hidden.validation.executable_sha256) "top-level executable hash differs from inputs"

$comparabilityProperties = @(
    "executable_arguments",
    "main_window_title",
    "host_os_version",
    "host_os_build",
    "host_architecture",
    "host_processors",
    "host_physical_memory_bytes",
    "cycles",
    "memory_runs",
    "memory_samples_seconds",
    "memory_scope",
    "process_identity_key",
    "process_attribution_rule",
    "webview2_user_data_folders"
)
foreach ($property in $comparabilityProperties) {
    $hiddenSource = @(switch ($property) {
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
        "process_identity_key" { $hidden.document.protocol.process_identity_key }
        "process_attribution_rule" { $hidden.document.protocol.process_attribution_rule }
        "webview2_user_data_folders" { $hidden.validation.webview2_user_data_folders }
    })
    $destroyedSource = @(switch ($property) {
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
        "process_identity_key" { $destroyed.document.protocol.process_identity_key }
        "process_attribution_rule" { $destroyed.document.protocol.process_attribution_rule }
        "webview2_user_data_folders" { $destroyed.validation.webview2_user_data_folders }
    })
    if ($property -notin @("executable_arguments", "host_processors", "memory_samples_seconds", "webview2_user_data_folders")) {
        $hiddenSource = $hiddenSource[0]
        $destroyedSource = $destroyedSource[0]
    }
    $sourceComparable = if ($property -ceq "webview2_user_data_folders") {
        Test-Issue154PairStringSetEqual -Left @($hiddenSource) -Right @($destroyedSource)
    } else {
        Test-Issue154PairEqual -Left $hiddenSource -Right $destroyedSource
    }
    Assert-Issue154PairSemantic $sourceComparable "$property differs between source reports"
    $pairMatches = if ($property -ceq "webview2_user_data_folders") {
        Test-Issue154PairStringSetEqual -Left @($pair.comparability.$property) -Right @($hiddenSource)
    } else {
        Test-Issue154PairEqual -Left $pair.comparability.$property -Right $hiddenSource
    }
    Assert-Issue154PairSemantic $pairMatches "comparability.$property differs from source"
}

Assert-Issue154PairBoolean -Value $pair.comparability.exact_executable_hash -Expected $true -Name "comparability.exact_executable_hash"
Assert-Issue154PairBoolean -Value $pair.comparability.root_command_lines_verified -Expected $true -Name "comparability.root_command_lines_verified"
Assert-Issue154PairBoolean -Value $pair.comparability.cross_run_process_identity_set_equality_required -Expected $false -Name "comparability.cross_run_process_identity_set_equality_required"
Assert-Issue154PairBoolean -Value $hidden.document.root_command_line.verified -Expected $true -Name "hidden root command line verification"
Assert-Issue154PairBoolean -Value $destroyed.document.root_command_line.verified -Expected $true -Name "destroyed root command line verification"
Assert-Issue154PairBoolean -Value $pair.comparability.requires_manual_same_data_features_services_power_state_confirmation -Expected $true -Name "comparability.requires_manual_same_data_features_services_power_state_confirmation"
Assert-Issue154PairBoolean -Value $pair.comparability.process_explorer_etw_cross_check_required -Expected $true -Name "comparability.process_explorer_etw_cross_check_required"

$manualFields = @("data_snapshot", "feature_flags", "service_configuration", "power_plan", "webview2_runtime_version", "foreground_application")
$manualComplete = $true
foreach ($field in $manualFields) {
    $hiddenValue = $hidden.document.manual_context.$field
    $destroyedValue = $destroyed.document.manual_context.$field
    Assert-Issue154PairSemantic (Test-Issue154PairEqual -Left $hiddenValue -Right $destroyedValue) "manual_context.$field differs"
    if ([string]::IsNullOrWhiteSpace([string]$hiddenValue)) {
        $manualComplete = $false
    }
    Assert-Issue154PairSemantic (Test-Issue154PairEqual -Left $pair.comparability.manual_context.$field -Right $hiddenValue) "paired manual_context.$field differs from source"
}
Assert-Issue154PairSemantic ([string]$hidden.document.manual_context.note -ceq [string]$destroyed.document.manual_context.note) "manual_context.note differs"
Assert-Issue154PairSemantic ([string]$pair.comparability.manual_context.note -ceq [string]$hidden.document.manual_context.note) "paired manual_context.note differs from source"
Assert-Issue154PairBoolean -Value $pair.comparability.manual_context_complete -Expected $manualComplete -Name "comparability.manual_context_complete"
Assert-Issue154PairBoolean -Value $pair.comparability.manual_comparability_confirmed -Expected $false -Name "comparability.manual_comparability_confirmed"
Assert-Issue154PairBoolean -Value $pair.comparability.process_explorer_etw_cross_check_confirmed -Expected $false -Name "comparability.process_explorer_etw_cross_check_confirmed"

$percentThreshold = [double]$pair.thresholds.reduction_percent
$mibThreshold = [double]$pair.thresholds.reduction_mib
Assert-Issue154PairSemantic ($percentThreshold -ge 40) "reduction_percent threshold must be at least 40"
Assert-Issue154PairSemantic ($mibThreshold -ge 50) "reduction_mib threshold must be at least 50"
Assert-Issue154PairSemantic ($percentThreshold -le 100) "reduction_percent threshold must not exceed 100"
Assert-Issue154PairSemantic ($mibThreshold -le 1048576) "reduction_mib threshold must not exceed 1048576"
Assert-Issue154PairSemantic (Test-Issue154PairEqual -Left @($pair.thresholds.horizons_seconds) -Right @(5, 30)) "threshold horizons must be exactly 5 and 30 seconds"
$privateAfter5Pass = Assert-Issue154Reduction -Reduction $pair.after_5s.private_working_set -Metric "private_working_set" -HiddenBytes ([int64]$hidden.validation.median_after_5s_private_working_set_bytes) -DestroyedBytes ([int64]$destroyed.validation.median_after_5s_private_working_set_bytes) -PercentThreshold $percentThreshold -MiBThreshold $mibThreshold -RequireMiBThreshold $true -Name "after_5s.private_working_set"
$privateAfter30Pass = Assert-Issue154Reduction -Reduction $pair.after_30s.private_working_set -Metric "private_working_set" -HiddenBytes ([int64]$hidden.validation.median_after_30s_private_working_set_bytes) -DestroyedBytes ([int64]$destroyed.validation.median_after_30s_private_working_set_bytes) -PercentThreshold $percentThreshold -MiBThreshold $mibThreshold -RequireMiBThreshold $true -Name "after_30s.private_working_set"
$commitAfter5Pass = Assert-Issue154Reduction -Reduction $pair.after_5s.commit -Metric "commit" -HiddenBytes ([int64]$hidden.validation.median_after_5s_commit_bytes) -DestroyedBytes ([int64]$destroyed.validation.median_after_5s_commit_bytes) -PercentThreshold $percentThreshold -MiBThreshold $mibThreshold -RequireMiBThreshold $false -Name "after_5s.commit"
$commitAfter30Pass = Assert-Issue154Reduction -Reduction $pair.after_30s.commit -Metric "commit" -HiddenBytes ([int64]$hidden.validation.median_after_30s_commit_bytes) -DestroyedBytes ([int64]$destroyed.validation.median_after_30s_commit_bytes) -PercentThreshold $percentThreshold -MiBThreshold $mibThreshold -RequireMiBThreshold $false -Name "after_30s.commit"
Assert-Issue154PairBoolean -Value $pair.paired_memory_gate_pass -Expected ($privateAfter5Pass -and $privateAfter30Pass -and $commitAfter5Pass -and $commitAfter30Pass) -Name "paired_memory_gate_pass"
Assert-Issue154PairSemantic ($null -eq $pair.overall_pass) "overall_pass must remain null until external gates are signed off"

ConvertTo-Json -InputObject ([ordered]@{
    valid = $true
    path = $resolvedPair
    sha256 = (Get-FileHash -LiteralPath $resolvedPair -Algorithm SHA256).Hash.ToLowerInvariant()
    paired_memory_gate_pass = [bool]$pair.paired_memory_gate_pass
}) -Depth 10
