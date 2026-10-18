namespace TieZ.Core;

public sealed record CaptureStatus(bool Running, ulong Starts, ulong Notifications, ulong Accepted,
    ulong Ignored, ulong Failed, ulong PolicyRevision, string? LastError);

public sealed record ClipboardWriteResult(long Id, uint ClipboardSequence);

public sealed partial class NativeCore
{
    public Task<ClipboardWriteResult> CopyEntryAsync(long id, bool plainText = false, bool allowSensitive = false, CancellationToken cancellationToken = default,
        ulong? expectedRevision = null) =>
        HistoryRequestAsync<ClipboardWriteResult>(new { op = "clipboard_copy", handle, id, expected_revision = expectedRevision, plain_text = plainText, allow_sensitive = allowSensitive }, cancellationToken);

    // Capture belongs to the shared application core, never to a settings window.
    // These use the same serialized, cancellable native request queue as history.
    public Task<CaptureStatus> StartCaptureAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<CaptureStatus>(new { op = "capture_start", handle }, cancellationToken);

    public Task<CaptureStatus> StopCaptureAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<CaptureStatus>(new { op = "capture_stop", handle }, cancellationToken);

    public Task<CaptureStatus> GetCaptureStatusAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<CaptureStatus>(new { op = "capture_status", handle }, cancellationToken);
}
