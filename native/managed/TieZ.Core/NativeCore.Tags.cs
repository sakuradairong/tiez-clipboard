namespace TieZ.Core;

public sealed record TagQuery(uint Limit = 100, string? After = null, ulong? ExpectedRevision = null);
public sealed record TagInfo(string Name, long StoredCount, int SessionCount, string? Color, bool IsSensitive)
{
    public long Count => StoredCount + SessionCount;
}
public sealed record TagPage(ulong Revision, IReadOnlyList<TagInfo> Items, string? NextAfter, bool HasMore);
public sealed record TagMutation(ulong Revision, bool Changed, bool PersistentSyncPending);

public sealed partial class NativeCore
{
    public Task<TagPage> GetTagsAsync(TagQuery? query = null, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<TagPage>(new { op = "tag_page", handle, query = query ?? new TagQuery() }, cancellationToken);
    public Task<TagMutation> CreateTagAsync(string name, ulong expectedRevision, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<TagMutation>(new { op = "tag_create", handle, name, expected_revision = expectedRevision }, cancellationToken);
    public Task<TagMutation> RenameTagAsync(string oldName, string newName, ulong expectedRevision, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<TagMutation>(new { op = "tag_rename", handle, old_name = oldName, new_name = newName,
            expected_revision = expectedRevision }, cancellationToken);
    public Task<TagMutation> SetTagColorAsync(string name, string? color, ulong expectedRevision, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<TagMutation>(new { op = "tag_color", handle, name, color, expected_revision = expectedRevision }, cancellationToken);
}
