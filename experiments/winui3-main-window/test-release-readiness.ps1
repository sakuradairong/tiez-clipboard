[CmdletBinding()]
param(
    [ValidateSet("Release")]
    [string]$Configuration = "Release",

    [ValidateRange(5, 50)]
    [int]$RunCount = 5,

    [ValidateRange(100, 1000)]
    [int]$LifecycleCycles = 100,

    [ValidateRange(1, 10000)]
    [double]$MaxMedianReadyMs = 750,

    [ValidateRange(1, 10000)]
    [double]$MaxWorstReadyMs = 1500,

    [ValidateRange(1, 4096)]
    [double]$MaxPeakWorkingSetMiB = 512,

    [ValidateRange(1, 1024)]
    [double]$MaxPrivateMemoryGrowthMiB = 64
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$LifecycleScript = Join-Path $Root "test-single-instance.ps1"
if (-not (Test-Path -LiteralPath $LifecycleScript)) {
    throw "Lifecycle test script was not found: $LifecycleScript"
}

$Results = @()
for ($run = 1; $run -le $RunCount; $run++) {
    $cycles = if ($run -eq 1) { $LifecycleCycles } else { 1 }
    Write-Host "Release readiness run $run/$RunCount ($cycles lifecycle cycles)..."
    $sample = & $LifecycleScript `
        -Configuration $Configuration `
        -LifecycleCycles $cycles
    if ($null -eq $sample -or -not $sample.PrimaryAlive -or -not $sample.DatabaseCreated) {
        throw "Release readiness run $run did not return a healthy lifecycle sample."
    }
    $Results += [pscustomobject]@{
        Run = $run
        PrimaryPid = $sample.PrimaryPid
        LifecycleCycles = $sample.LifecycleCycles
        RequestedToReadyMs = [double]$sample.RequestedToReadyMs
        InitialWorkingSetMiB = [double]$sample.InitialWorkingSetMiB
        FinalWorkingSetMiB = [double]$sample.FinalWorkingSetMiB
        PeakWorkingSetMiB = [double]$sample.PeakWorkingSetMiB
        InitialPrivateMemoryMiB = [double]$sample.InitialPrivateMemoryMiB
        FinalPrivateMemoryMiB = [double]$sample.FinalPrivateMemoryMiB
        PrivateMemoryGrowthMiB = [double]$sample.PrivateMemoryGrowthMiB
        HandleCount = [int]$sample.HandleCount
    }
}

$ReadySamples = @($Results.RequestedToReadyMs | Sort-Object)
$middle = [Math]::Floor($ReadySamples.Count / 2)
if (($ReadySamples.Count % 2) -eq 0) {
    $MedianReadyMs = ($ReadySamples[$middle - 1] + $ReadySamples[$middle]) / 2
}
else {
    $MedianReadyMs = $ReadySamples[$middle]
}
$WorstFiveReadyMs = @($ReadySamples | Sort-Object -Descending | Select-Object -First 5)
$WorstReadyMs = ($WorstFiveReadyMs | Measure-Object -Maximum).Maximum
$PeakWorkingSetMiB = ($Results.PeakWorkingSetMiB | Measure-Object -Maximum).Maximum
$MaxObservedPrivateGrowthMiB = (
    $Results.PrivateMemoryGrowthMiB | Measure-Object -Maximum
).Maximum
$TotalLifecycleCycles = ($Results.LifecycleCycles | Measure-Object -Sum).Sum

$Results |
    Format-Table `
        Run, `
        PrimaryPid, `
        LifecycleCycles, `
        RequestedToReadyMs, `
        InitialWorkingSetMiB, `
        FinalWorkingSetMiB, `
        PeakWorkingSetMiB, `
        PrivateMemoryGrowthMiB, `
        HandleCount `
        -AutoSize |
    Out-Host

if ($MedianReadyMs -gt $MaxMedianReadyMs) {
    throw "Median requested-to-ready time $MedianReadyMs ms exceeds $MaxMedianReadyMs ms."
}
if ($WorstReadyMs -gt $MaxWorstReadyMs) {
    throw "Worst requested-to-ready sample $WorstReadyMs ms exceeds $MaxWorstReadyMs ms."
}
if ($PeakWorkingSetMiB -gt $MaxPeakWorkingSetMiB) {
    throw "Peak working set $PeakWorkingSetMiB MiB exceeds $MaxPeakWorkingSetMiB MiB."
}
if ($MaxObservedPrivateGrowthMiB -gt $MaxPrivateMemoryGrowthMiB) {
    throw "Private-memory growth $MaxObservedPrivateGrowthMiB MiB exceeds $MaxPrivateMemoryGrowthMiB MiB."
}

[pscustomobject]@{
    IndependentRuns = $Results.Count
    TotalLifecycleCycles = $TotalLifecycleCycles
    MedianRequestedToReadyMs = [Math]::Round($MedianReadyMs, 1)
    WorstFiveRequestedToReadyMs = ($WorstFiveReadyMs -join ", ")
    PeakWorkingSetMiB = [Math]::Round($PeakWorkingSetMiB, 2)
    MaxPrivateMemoryGrowthMiB = [Math]::Round($MaxObservedPrivateGrowthMiB, 2)
    Result = "passed"
}
