namespace TieZ.Core;

public sealed record ClipboardPasteResult(long Id, uint ClipboardSequence, ulong TargetGeneration, uint InputEvents);

public sealed partial class NativeCore
{
    public Task<ClipboardPasteResult> PasteEntryAsync(long id, ulong expectedGeneration, bool plainText = false,
        bool allowSensitive = false, CancellationToken cancellationToken = default, ulong? expectedRevision = null) =>
        HistoryRequestAsync<ClipboardPasteResult>(new { op = "clipboard_paste", handle, id,
            expected_generation = expectedGeneration, expected_revision = expectedRevision,
            plain_text = plainText, allow_sensitive = allowSensitive }, cancellationToken);
}
