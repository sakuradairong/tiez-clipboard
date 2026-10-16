namespace TieZ.Core;

public sealed record DocumentTextRun(string Text, bool Bold, bool Italic, bool Underline,
    bool Strike, bool Code, string? Link)
{
    public override string ToString() => "DocumentTextRun (content omitted)";
}
public sealed record DocumentCell(int Row, int Column, int RowSpan, int ColumnSpan, bool Header,
    IReadOnlyList<DocumentTextRun> Runs)
{
    public override string ToString() => $"DocumentCell {Row},{Column}";
}
public sealed record DocumentBlock(string Kind, IReadOnlyList<DocumentTextRun>? Runs, int Heading,
    bool Preformatted, int Rows, int Columns, IReadOnlyList<DocumentCell>? Cells)
{
    public override string ToString() => $"DocumentBlock {Kind}";
}
public sealed record DocumentPreviewData(long Id, ulong Revision, IReadOnlyList<DocumentBlock> Blocks,
    bool Truncated, bool OmittedMedia)
{
    public override string ToString() => $"DocumentPreview {Id}: {Blocks.Count} blocks";
}
public sealed partial class NativeCore
{
    public Task<DocumentPreviewData> GetDocumentPreviewAsync(long id, ulong expectedRevision,
        bool allowSensitive = false, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<DocumentPreviewData>(new { op = "history_document_preview", handle, id,
            expected_revision = expectedRevision, allow_sensitive = allowSensitive }, cancellationToken);
}
