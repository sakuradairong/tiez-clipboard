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
            Rename(core);
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
    private static void Rename(NativeCore core)
    {
        const string source = "zz-rename-source", target = "zz-rename-target";
        var entry = core.AddManualAsync("  rename\r\n ", "rich_text", "<b>original</b>", new[] { source, target }).GetAwaiter().GetResult();
        var firstColor = core.SetTagColorAsync(source, "red", entry.Revision).GetAwaiter().GetResult();
        var secondColor = core.SetTagColorAsync(target, "navy", firstColor.Revision).GetAwaiter().GetResult();
        var result = core.RenameTagAsync(source, " " + target + " ", secondColor.Revision).GetAwaiter().GetResult();
        Check(result.Changed && result.Revision == secondColor.Revision + 1 && result.PersistentSyncPending, "Tag rename mutation DTO/revision");
        Check(!All(core).Any(tag => tag.Name == source) && All(core).Single(tag => tag.Name == target).Color == "navy", "Rename merge color/source removal");
        var stored = core.GetEntryAsync(entry.Id).GetAwaiter().GetResult();
        Check(stored.Tags.SequenceEqual(new[] { target }) && stored.Content == "  rename\r\n " && stored.HtmlContent == "<b>original</b>", "Rename changed entry payload or duplicated merged tag");
        Expect("history_conflict", () => core.RenameTagAsync(target, source, secondColor.Revision).GetAwaiter().GetResult());
        Expect("not_found", () => core.RenameTagAsync("no-such-tag", source, result.Revision).GetAwaiter().GetResult());
        Expect("invalid_tag", () => core.RenameTagAsync(target, " ", result.Revision).GetAwaiter().GetResult());
        Check(!core.RenameTagAsync(target, target, result.Revision).GetAwaiter().GetResult().Changed, "Rename same-name no-op advanced revision");
        using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
        try { core.RenameTagAsync(target, source, result.Revision, cancelled.Token).GetAwaiter().GetResult(); throw new Exception("Cancelled rename ran"); }
        catch (OperationCanceledException) { }
        var sensitive = core.RenameTagAsync(target, "PASSWORD", result.Revision).GetAwaiter().GetResult();
        Check(All(core).Single(tag => tag.Name == "PASSWORD").IsSensitive, "Renamed sensitive alias classification");
        Expect("sensitive_confirmation", () => core.GetPreviewEntryAsync(entry.Id, sensitive.Revision, false).GetAwaiter().GetResult());
        Check(core.GetEntryAsync(entry.Id).GetAwaiter().GetResult().HtmlContent == "<b>original</b>", "Sensitive rename lost original HTML");
        var ordinary = core.RenameTagAsync("PASSWORD", source, sensitive.Revision).GetAwaiter().GetResult();
        Check(core.GetPreviewEntryAsync(entry.Id, ordinary.Revision, false).GetAwaiter().GetResult().Content == "  rename\r\n ", "Rename out of sensitive failed");
        core.DeleteEntryAsync(entry.Id).GetAwaiter().GetResult();
        Check(All(core).Single(tag => tag.Name == source).Count == 0, "Renamed tag disappeared after its last entry was deleted");
        Console.WriteLine("Tag rename/merge: colors, original formats, sensitive aliases, no-op/stale/invalid/cancellation passed.");
    }

}
