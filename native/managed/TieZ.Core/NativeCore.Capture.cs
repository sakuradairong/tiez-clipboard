namespace TieZ.Core;

public sealed record CaptureStatus(bool Running, ulong Starts, ulong Notifications, ulong Accepted,
    ulong Ignored, ulong Failed, ulong PolicyRevision, string? LastError);

public sealed partial class NativeCore
{
    // Capture belongs to the shared application core, never to a settings window.
    // These use the same serialized, cancellable native request queue as history.
    public Task<CaptureStatus> StartCaptureAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<CaptureStatus>(new { op = "capture_start", handle }, cancellationToken);

    public Task<CaptureStatus> StopCaptureAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<CaptureStatus>(new { op = "capture_stop", handle }, cancellationToken);

    public Task<CaptureStatus> GetCaptureStatusAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<CaptureStatus>(new { op = "capture_status", handle }, cancellationToken);
}
