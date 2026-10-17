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
            Delete(core);
            Ordering(core);
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

    private static void Delete(NativeCore core)
    {
        const string name = "zz-delete-fixture", other = "zz-delete-other";
        var first = core.AddManualAsync("pinned", tags: new[] { name, other }).GetAwaiter().GetResult();
        core.SetPinnedAsync(first.Id, true).GetAwaiter().GetResult();
        var second = core.AddManualAsync("secret", tags: new[] { name, "密码" }).GetAwaiter().GetResult();
        var kept = core.AddManualAsync("keep", tags: new[] { other }).GetAwaiter().GetResult();
        core.CreateTagAsync(name, kept.Revision).GetAwaiter().GetResult();
        var preview = core.GetTagDeletePreviewAsync(name).GetAwaiter().GetResult();
        Check(preview.Name == name && preview.Count == 2 && preview.StoredCount == 2 && preview.SessionCount == 0 && preview.SavedTag,
            "Tag deletion preview DTO/counts");
        Check(core.GetTagsAsync().GetAwaiter().GetResult().Revision == preview.Revision, "Tag deletion preview wrote history");
        var late = core.CreateTagAsync("zz-delete-late", preview.Revision).GetAwaiter().GetResult();
        Expect("history_conflict", () => core.DeleteTagAsync(name, preview.Revision).GetAwaiter().GetResult());
        Check(core.GetEntryAsync(first.Id).GetAwaiter().GetResult().IsPinned, "Stale deletion removed pinned content");
        using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
        try { core.DeleteTagAsync(name, late.Revision, cancelled.Token).GetAwaiter().GetResult(); throw new Exception("Cancelled delete ran"); }
        catch (OperationCanceledException) { }
        preview = core.GetTagDeletePreviewAsync(name).GetAwaiter().GetResult();
        var result = core.DeleteTagAsync(name, preview.Revision).GetAwaiter().GetResult();
        Check(result.Changed && result.Revision == preview.Revision + 1 && result.RemovedCount == 2 && result.StoredCount == 2
            && result.SessionCount == 0 && result.PersistentSyncPending && result.CleanupError is null, "Tag deletion result DTO");
        Expect("not_found", () => core.GetEntryAsync(first.Id).GetAwaiter().GetResult());
        Expect("not_found", () => core.GetEntryAsync(second.Id).GetAwaiter().GetResult());
        Check(core.GetEntryAsync(kept.Id).GetAwaiter().GetResult().Content == "keep", "Tag deletion removed unrelated record");
        Check(core.GetHistoryChangesAsync(preview.Revision).GetAwaiter().GetResult().ResyncRequired, "Tag deletion failed to invalidate old consumers");
        Expect("not_found", () => core.GetTagDeletePreviewAsync(name).GetAwaiter().GetResult());
        Check(!core.DeleteTagAsync(name, result.Revision).GetAwaiter().GetResult().Changed, "Repeated missing-tag deletion wrote state");
        var zero = core.CreateTagAsync(name, result.Revision).GetAwaiter().GetResult();
        var empty = core.GetTagDeletePreviewAsync(name).GetAwaiter().GetResult();
        Check(empty.Count == 0 && empty.Revision == zero.Revision && empty.SavedTag, "Zero-use saved tag preview");
        Check(core.DeleteTagAsync(name, empty.Revision).GetAwaiter().GetResult().RemovedCount == 0, "Zero-use tag delete removed history");
        core.DeleteEntryAsync(kept.Id).GetAwaiter().GetResult();
        Console.WriteLine("Tag delete: read-only preview, whole-entry scope, pins/sensitive, stale/cancel/no-op, zero count and resync passed.");
    }

    private static void Ordering(NativeCore core)
    {
        ulong revision = core.GetTagsAsync().GetAwaiter().GetResult().Revision;
        for (int index = 0; index < 205; index++)
            revision = core.CreateTagAsync($"sort-{index:000}", revision).GetAwaiter().GetResult().Revision;
        var first = core.AddManualAsync("ordered first", tags: new[] { "sort-204" }).GetAwaiter().GetResult();
        var second = core.AddManualAsync("ordered second", tags: new[] { "sort-204", "sort-203" }).GetAwaiter().GetResult();
        var expected = All(core).OrderByDescending(tag => tag.Count).ThenBy(tag => tag.Name, StringComparer.Ordinal).Select(tag => tag.Name).ToArray();
        var names = new List<string>(); string? after = null; ulong? accepted = null;
        do
        {
            var page = core.GetTagsAsync(new TagQuery(7, after, accepted, "count")).GetAwaiter().GetResult();
            Check(page.Items.Count <= 7 && page.Revision == second.Revision, "Count paging limits/revision");
            names.AddRange(page.Items.Select(tag => tag.Name)); accepted = page.Revision; after = page.NextAfter;
            Check(page.HasMore == (after is not null), "Count paging cursor contract");
        } while (after is not null);
        Check(names.SequenceEqual(expected) && names[0] == "sort-204", "Count paging sorted only the current page or skipped ties");
        Check(core.GetTagsAsync(new TagQuery(uint.MaxValue, Sort: "count")).GetAwaiter().GetResult().Items.Count == 200, "Count limit was not clamped");
        Expect("invalid_tag_cursor", () => core.GetTagsAsync(new TagQuery(After: "missing-sort", ExpectedRevision: accepted, Sort: "count")).GetAwaiter().GetResult());
        Expect("invalid_request", () => core.GetTagsAsync(new TagQuery(Sort: "unknown")).GetAwaiter().GetResult());
        core.DeleteEntryAsync(first.Id).GetAwaiter().GetResult();
        Expect("history_conflict", () => core.GetTagsAsync(new TagQuery(After: names[0], ExpectedRevision: accepted, Sort: "count")).GetAwaiter().GetResult());
        core.DeleteEntryAsync(second.Id).GetAwaiter().GetResult();
        Search(core);
        Console.WriteLine("Tag count sorting: full-catalog order, deterministic ties, bounded paging, invalid/stale cursors and compatibility default passed.");
    }

    private static void Search(NativeCore core)
    {
        ulong revision = core.GetTagsAsync().GetAwaiter().GetResult().Revision;
        const string label = "search-É ' %_ 标签";
        core.CreateTagAsync(label, revision).GetAwaiter().GetResult();
        var page = core.GetTagsAsync(new TagQuery(Search: "SEARCH-é ' %_", Sort: "count")).GetAwaiter().GetResult();
        Check(page.Items.Count == 1 && page.Items[0].Name == label, "Unicode/literal tag search DTO");
        var allExpected = All(core).Where(tag => tag.Name.Contains("%_", StringComparison.Ordinal)).Select(tag => tag.Name).ToArray();
        Check(core.GetTagsAsync(new TagQuery(Search: "%_")).GetAwaiter().GetResult().Items.Select(tag => tag.Name).SequenceEqual(allExpected),
            "Search treated percent/underscore as wildcard");
        var expected = All(core).Where(tag => tag.Name.Contains("sort-20", StringComparison.Ordinal)).Select(tag => tag.Name).ToArray();
        var names = new List<string>(); string? after = null; ulong? accepted = null;
        do
        {
            var next = core.GetTagsAsync(new TagQuery(2, after, accepted, "name", "SORT-20")).GetAwaiter().GetResult();
            names.AddRange(next.Items.Select(tag => tag.Name)); after = next.NextAfter; accepted = next.Revision;
        } while (after is not null);
        Check(names.SequenceEqual(expected) && names.Count == 5, "Search did not reach matches beyond the first 200 tags");
        var missing = core.GetTagsAsync(new TagQuery(Search: "no-such-tag-filter")).GetAwaiter().GetResult();
        Check(!missing.HasMore && missing.NextAfter is null && missing.Items.Count == 0, "Empty tag search retained results/cursor");
        Check(core.GetTagsAsync().GetAwaiter().GetResult().Revision == accepted, "Search wrote history metadata");
        Console.WriteLine("Tag search: Unicode/literal matches, whole-catalog filtered paging, exact names, empty results and readonly state passed.");
    }

}
