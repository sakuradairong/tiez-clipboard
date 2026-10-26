using TieZ.Core;

internal static class HistoryOrderingSmoke
{
    private static T Read<T>(Task<T> task) => task.GetAwaiter().GetResult();
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    private static void Expect(string code, Action action)
    {
        try { action(); } catch (CoreException error) when (error.Code == code) { return; }
        throw new InvalidOperationException($"Expected {code}");
    }
    internal static void Run(NativeCore core)
    {
        const string tag = "managed-order-fixture";
        ulong settingsRevision = core.Snapshot.Revision;
        const string original = "  ordered-000 \r\n  ";
        const string html = "<b>unchanged</b>";
        var ids = new List<long>();
        for (int index = 0; index < 210; index++)
            ids.Add(Read(core.AddManualAsync(index == 0 ? original : $"ordered-{index:000}",
                index == 0 ? "rich_text" : "text", index == 0 ? html : null, new[] { tag })).Id);
        Read(core.SetPinnedAsync(ids[0], true));
        ulong revision = Read(core.GetHistoryAsync(new())).Revision;
        Check(Read(core.GetHistoryAsync(new(Limit: 1, Tag: tag))).Items[0].Id == ids[0], "Default sort lost pinned priority");
        // Read the full fixture independently of sorted pages. Fast optimized inserts can
        // share timestamps; the established contract preserves pin priority inside those ties.
        var metadata = ids.Select(id => Read(core.GetEntryAsync(id))).ToArray();
        int pinnedTimeTies = metadata.Count(entry => entry.Timestamp == metadata[0].Timestamp);
        foreach (string sort in new[] { "time", "usage" })
        {
            uint offset = 0; var actual = new List<long>();
            while (true)
            {
                var page = Read(core.GetHistoryAsync(new(Limit: 7, Offset: offset, ExpectedRevision: revision, Tag: tag, Sort: sort)));
                Check(page.Items.Count <= 7 && page.Revision == revision, "Sorted history page bounds/revision");
                actual.AddRange(page.Items.Select(entry => entry.Id)); offset = page.NextOffset;
                if (!page.HasMore) break;
            }
            var expected = metadata.OrderByDescending(entry => sort == "usage" ? entry.UseCount : entry.Timestamp)
                .ThenByDescending(entry => entry.IsPinned).ThenByDescending(entry => entry.PinnedOrder)
                .ThenByDescending(entry => entry.Timestamp).ThenByDescending(entry => entry.Id).Select(entry => entry.Id);
            Check(actual.Count == ids.Count && actual.Distinct().Count() == ids.Count && actual.SequenceEqual(expected),
                $"Sorted history broke full-catalog/pin-tie order or repeated entries: sort={sort}, pinnedTimestampGroup={pinnedTimeTies}");
            var filtered = Read(core.GetHistoryAsync(new(Tag: tag, Search: "ordered-209", Sort: sort)));
            Check(filtered.Items.Count == 1 && filtered.Items[0].Id == ids[^1], "Sorted query lost literal content/tag filtering");
        }
        var preserved = Read(core.GetEntryAsync(ids[0]));
        Check(preserved.Content == original && preserved.HtmlContent == html, "Sorted reads rewrote whitespace/CRLF or original rich format");
        Expect("invalid_request", () => Read(core.GetHistoryAsync(new(Sort: "unknown"))));
        Expect("history_conflict", () => Read(core.GetHistoryAsync(new(Offset: 1, Sort: "usage"))));
        Check(core.Snapshot.Revision == settingsRevision && Read(core.GetHistoryAsync(new())).Revision == revision,
            "Sort reads wrote settings or history");
        Read(core.DeleteEntriesAsync(ids, revision));
        Expect("history_conflict", () => Read(core.GetHistoryAsync(new(Offset: 1, ExpectedRevision: revision, Sort: "time"))));
        Console.WriteLine($"PASS: real DLL history time/usage/default order, 210-record paging, literal tag/content filters, readonly and stale/invalid guards; pinned timestamp group={pinnedTimeTies}");
    }
}
