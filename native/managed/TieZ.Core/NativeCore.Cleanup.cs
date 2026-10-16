namespace TieZ.Core;

public sealed record CleanupPreviewData(string Text, bool Ignored, int Rules, int Matches)
{
    public override string ToString() => $"CleanupPreview: {Rules} rules, {Matches} matches, ignored={Ignored}";
}
public sealed partial class NativeCore
{
    public Task<CleanupPreviewData> PreviewCleanupAsync(string rules, string text,
        CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<CleanupPreviewData>(new { op = "cleanup_preview", handle, rules, text }, cancellationToken);
}
