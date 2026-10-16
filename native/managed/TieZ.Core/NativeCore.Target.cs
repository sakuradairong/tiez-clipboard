namespace TieZ.Core;

public sealed record TargetTrackingStatus(bool Running, ulong Starts, ulong Generation,
    bool HasTarget, string? Application, string? LastError);
public sealed record TargetFocusResult(ulong Generation, bool Restored);

public sealed partial class NativeCore
{
    public Task<TargetTrackingStatus> StartTargetTrackingAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<TargetTrackingStatus>(new { op = "target_start", handle }, cancellationToken);
    public Task<TargetTrackingStatus> GetTargetStatusAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<TargetTrackingStatus>(new { op = "target_status", handle }, cancellationToken);
    public Task<TargetTrackingStatus> StopTargetTrackingAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<TargetTrackingStatus>(new { op = "target_stop", handle }, cancellationToken);
    public Task<TargetFocusResult> RestoreTargetFocusAsync(ulong expectedGeneration, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<TargetFocusResult>(new { op = "target_restore", handle, expected_generation = expectedGeneration }, cancellationToken);
}
