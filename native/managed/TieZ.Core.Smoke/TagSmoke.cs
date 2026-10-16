using TieZ.Core;

internal static class TagSmoke
{
    private const string Name = "原生目录 ' %_ 标签";
    private const string Color = "rgba(1, 2, 3, .5)";
    private static void Check(bool value, string message) { if (!value) throw new Exception(message); }
    private static void Expect(string code, Action operation)
    {
        try { operation(); }
        catch (CoreException error) when (error.Code == code) { return; }
        throw new Exception($"Expected {code}");
    }
    private static List<TagInfo> All(NativeCore core)
    {
        var tags = new List<TagInfo>(); string? after = null; ulong? revision = null;
        do
        {
            var page = core.GetTagsAsync(new TagQuery(Limit: 1, After: after, ExpectedRevision: revision)).GetAwaiter().GetResult();
            Check(page.Items.Count <= 1 && (revision is null || page.Revision == revision), "Tag paging bounds/revision");
            revision = page.Revision; tags.AddRange(page.Items); after = page.NextAfter;
            Check(page.HasMore == (after is not null), "Tag next-cursor contract");
        } while (after is not null);
        Check(tags.Select(tag => tag.Name).Distinct(StringComparer.Ordinal).Count() == tags.Count, "Tag paging duplicated names");
        return tags;
    }
    internal static void Run(string directory)
    {
        Directory.CreateDirectory(directory);
        string database = Path.Combine(directory, "标签.db");
        using (var core = new NativeCore(database))
        {
            int notifications = 0; core.Changed += _ => notifications++;
            Check(All(core).Any(tag => tag.Name == "密码" && tag.IsSensitive && tag.Count == 0), "Built-in tags were omitted");
            var initial = core.GetTagsAsync().GetAwaiter().GetResult();
            Check(initial.Revision == 0 && core.Snapshot.Revision == 0 && notifications == 0, "Tag read wrote settings/history");
            var created = core.CreateTagAsync(Name, 0).GetAwaiter().GetResult();
            Check(created.Changed && created.Revision == 1 && created.PersistentSyncPending, "Tag creation contract");
            Check(!core.CreateTagAsync(Name, 1).GetAwaiter().GetResult().Changed, "Duplicate tag changed revision");
            var colored = core.SetTagColorAsync(Name, Color, 1).GetAwaiter().GetResult();
            Check(colored.Changed && colored.Revision == 2, "Tag color save");
            Check(!core.SetTagColorAsync(Name, Color, 2).GetAwaiter().GetResult().Changed, "Unchanged color wrote data");
            var tag = All(core).Single(tag => tag.Name == Name);
            Check(tag.Color == Color && tag.Count == 0 && !tag.IsSensitive, "Tag color/zero count DTO");
            Expect("history_conflict", () => core.CreateTagAsync("stale", 1).GetAwaiter().GetResult());
            Expect("history_conflict", () => core.GetTagsAsync(new TagQuery(After: "", ExpectedRevision: 1)).GetAwaiter().GetResult());
            Expect("history_conflict", () => core.GetTagsAsync(new TagQuery(After: "")).GetAwaiter().GetResult());
            Expect("invalid_tag", () => core.CreateTagAsync(" \n ", 2).GetAwaiter().GetResult());
            using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
            try { core.CreateTagAsync("cancelled", 2, cancelled.Token).GetAwaiter().GetResult(); throw new Exception("Cancelled tag save ran"); }
            catch (OperationCanceledException) { }
            Check(core.GetTagsAsync().GetAwaiter().GetResult().Revision == 2, "Cancelled/failed tag save changed revision");
            var added = core.AddManualAsync("  tag count\r\n ", tags: new[] { Name }).GetAwaiter().GetResult();
            tag = All(core).Single(tag => tag.Name == Name);
            Check(tag.StoredCount == 1 && tag.SessionCount == 0 && tag.Count == 1 && tag.Color == Color, "Tag count after entry save");
            core.DeleteEntryAsync(added.Id).GetAwaiter().GetResult();
            var feed = core.GetHistoryChangesAsync(0).GetAwaiter().GetResult();
            Check(feed.ResyncRequired && feed.PersistentSyncPending, "Tag metadata did not invalidate stale consumers");
            Check(core.Snapshot.Revision == 0 && notifications == 0, "Tag metadata wrote settings or emitted setting events");
        }
        using var reopened = new NativeCore(database);
        var stored = All(reopened).Single(tag => tag.Name == Name);
        Check(stored.Color == Color && stored.Count == 0, "Tag metadata persistence on reopen");
        var current = reopened.GetTagsAsync().GetAwaiter().GetResult();
        Check(reopened.SetTagColorAsync(Name, null, current.Revision).GetAwaiter().GetResult().Changed, "Clear saved tag color");
        Check(All(reopened).Single(tag => tag.Name == Name).Color is null, "Cleared tag color remained set");
        Console.WriteLine("Tag catalog/create/color: paging, counts, conflicts, cancellation, no-op/settings and reopen passed.");
    }
}
