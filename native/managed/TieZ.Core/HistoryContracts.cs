namespace TieZ.Core;

// Transport names are serialized with the shared snake_case ABI policy.
public sealed record HistoryQuery(
    uint Limit = 50, uint Offset = 0, ulong? ExpectedRevision = null,
    string Search = "", string? ContentType = null, string? Tag = null,
    bool TagOnly = false, bool PinnedOnly = false);

public sealed record HistorySummary(
    long Id, string ContentType, string Preview, string SourceApp, long Timestamp,
    bool IsPinned, IReadOnlyList<string> Tags, int UseCount, bool IsExternal)
{
    public bool IsSensitive => Tags.Any(tag =>
        tag.Equals("sensitive", StringComparison.OrdinalIgnoreCase) ||
        tag.Equals("password", StringComparison.OrdinalIgnoreCase) || tag == "密码");
}

public sealed record ClipboardEntry(
    long Id, string ContentType, string Content, string? HtmlContent,
    string SourceApp, string? SourceAppPath, long Timestamp, string Preview,
    bool IsPinned, IReadOnlyList<string> Tags, int UseCount, bool IsExternal,
    long PinnedOrder, bool FilePreviewExists);

public sealed record HistoryPage(
    ulong Revision, IReadOnlyList<HistorySummary> Items, uint NextOffset,
    bool HasMore, long StoredCount, int SessionCount);

public sealed record HistoryChange(
    ulong Revision, long Id, long? ReplacedId, IReadOnlyList<long> RemovedIds);

public sealed record HistoryChanges(
    ulong Revision, bool ResyncRequired, IReadOnlyList<HistoryChange> Changes,
    bool PersistentSyncPending);

public sealed record HistoryMutation(
    ulong Revision, bool Changed, long Id, long? ReplacedId,
    IReadOnlyList<long> RemovedIds, bool PersistentSyncPending);

public sealed record HistoryClearPreview(ulong Revision, long StoredCount, int SessionCount)
{
    public long EligibleCount => StoredCount + SessionCount;
}

public sealed record HistoryClearResult(ulong Revision, bool Changed, long RemovedCount, bool PersistentSyncPending,
    string? CompactionError = null);
