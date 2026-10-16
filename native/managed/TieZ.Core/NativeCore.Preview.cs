namespace TieZ.Core;

public sealed record ImagePreviewData(long Id, ulong Revision, int OriginalWidth, int OriginalHeight,
    int Width, int Height, bool AnimationSource, string PngBase64)
{
    public override string ToString() => $"ImagePreview {Id}: {Width}x{Height} ({OriginalWidth}x{OriginalHeight})";
}

public sealed partial class NativeCore
{
    public Task<ClipboardEntry> GetPreviewEntryAsync(long id, ulong expectedRevision, bool allowSensitive = false,
        CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<ClipboardEntry>(new { op = "history_preview_entry", handle, id,
            expected_revision = expectedRevision, allow_sensitive = allowSensitive }, cancellationToken);

    public Task<ImagePreviewData> GetImagePreviewAsync(long id, ulong expectedRevision, bool allowSensitive = false,
        uint maxEdge = 512, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<ImagePreviewData>(new { op = "history_image_preview", handle, id,
            expected_revision = expectedRevision, allow_sensitive = allowSensitive, max_edge = maxEdge }, cancellationToken);
}
