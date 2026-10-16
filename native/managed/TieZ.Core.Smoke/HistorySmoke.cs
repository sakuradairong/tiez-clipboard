using TieZ.Core;

internal static class HistorySmoke
{
    private static T Read<T>(Task<T> task) => task.GetAwaiter().GetResult();
    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
    private static void Expect(string code, Action action)
    {
        try { action(); }
        catch (CoreException e) when (e.Code == code) { return; }
        throw new InvalidOperationException($"Expected {code}");
    }

    internal static long Run(NativeCore core)
    {
        string body = " managed 中文 %_ \r\n ";
        var saved = Read(core.AddManualAsync(body));
        var rich = Read(core.AddManualAsync("managed rich", "rich_text", "<b>managed rich</b>"));
        Check(saved.Id > 0 && rich.Id > saved.Id && saved.PersistentSyncPending, "Mutation contract");
        var page = Read(core.GetHistoryAsync(new(Limit: 1, Search: "managed")));
        Check(page.Items.Count == 1 && page.Items[0].Id == rich.Id && page.HasMore, "First page");
        var next = Read(core.GetHistoryAsync(new(Limit: 1, Offset: page.NextOffset,
            ExpectedRevision: page.Revision, Search: "managed")));
        Check(next.Items[0].Id == saved.Id, "Page continuation");
        var entry = Read(core.GetEntryAsync(saved.Id));
        Check(entry.Content == body && Read(core.GetEntryAsync(rich.Id)).HtmlContent == "<b>managed rich</b>", "Original formats");
        Check(!Read(core.EditEntryAsync(saved.Id, body)).Changed, "No-op edit");
        Read(core.SetPinnedAsync(saved.Id, true));
        Expect("history_conflict", () => Read(core.GetHistoryAsync(new(Limit: 1, Offset: page.NextOffset,
            ExpectedRevision: page.Revision))));
        Read(core.SetTagsAsync(saved.Id, ["密码", "work"]));
        var sensitive = Read(core.GetHistoryAsync(new(Search: "%_", Tag: "work")));
        Check(sensitive.Items.Count == 1 && sensitive.Items[0].IsSensitive, "Sensitive metadata and literal search");
        Read(core.EditEntryAsync(rich.Id, "managed plain"));
        var edited = Read(core.GetEntryAsync(rich.Id));
        Check(edited.ContentType == "text" && edited.HtmlContent is null, "Rich to plain transition");
        Read(core.DeleteEntryAsync(rich.Id));
        Expect("not_found", () => Read(core.GetEntryAsync(rich.Id)));
        Expect("invalid_page", () => Read(core.GetHistoryAsync(new(Limit: 201))));
        var changes = Read(core.GetHistoryChangesAsync(0));
        Check(changes.Revision >= 6 && changes.PersistentSyncPending &&
            changes.Changes.Any(c => c.RemovedIds.Contains(rich.Id)), "Change feed");

        using var cancelled = new CancellationTokenSource();
        cancelled.Cancel();
        try { Read(core.AddManualAsync("must not save", cancellationToken: cancelled.Token)); throw new Exception("Cancelled work ran"); }
        catch (OperationCanceledException) { }
        Check(Read(core.GetHistoryAsync(new(Search: "must not save"))).Items.Count == 0, "Cancelled mutation persisted");

        using var racing = new CancellationTokenSource();
        var task = core.AddManualAsync("managed cancellation race", cancellationToken: racing.Token);
        racing.Cancel();
        bool accepted;
        try { Read(task); accepted = true; }
        catch (OperationCanceledException) { accepted = false; }
        var raced = Read(core.GetHistoryAsync(new(Search: "managed cancellation race")));
        Check((raced.Items.Count == 1) == accepted, "Cancellation hid a committed mutation");
        if (accepted) Read(core.DeleteEntryAsync(raced.Items[0].Id));

        bool rejected = Task.Run(() =>
        {
            try { Read(core.GetHistoryAsync(new())); return false; }
            catch (InvalidOperationException) { return true; }
        }).GetAwaiter().GetResult();
        Check(rejected, "History API owner thread guard");
        long batchFirst = Read(core.AddManualAsync("batch first")).Id;
        long batchSecond = Read(core.AddManualAsync("batch second")).Id;
        ulong batchRevision = Read(core.GetHistoryAsync(new())).Revision;
        Expect("history_conflict", () => Read(core.DeleteEntriesAsync([batchFirst, batchSecond], batchRevision - 1)));
        Expect("not_found", () => Read(core.DeleteEntriesAsync([batchFirst, long.MaxValue], batchRevision)));
        Check(Read(core.GetEntryAsync(batchFirst)).Content == "batch first", "Rejected batch partially deleted");
        var deleted = Read(core.DeleteEntriesAsync([batchSecond, batchFirst, batchFirst], batchRevision));
        Check(deleted.Changed && deleted.Revision == batchRevision + 1
            && deleted.RemovedIds.Order().SequenceEqual(new[] { batchFirst, batchSecond }), "Atomic batch result");
        Check(Read(core.GetHistoryChangesAsync(batchRevision)).Changes.Count == 1, "Batch emitted multiple revisions");
        Expect("not_found", () => Read(core.GetEntryAsync(batchFirst)));
        Expect("not_found", () => Read(core.GetEntryAsync(batchSecond)));
        Check(!Read(core.DeleteEntriesAsync([], deleted.Revision)).Changed, "Empty batch changed history");
        long clearCandidate = Read(core.AddManualAsync("clear candidate")).Id;
        var clearPreview = Read(core.GetClearHistoryPreviewAsync());
        Check(clearPreview.EligibleCount == 1 && clearPreview.StoredCount == 1 && clearPreview.SessionCount == 0,
            "Clear preview included protected history");
        Check(Read(core.GetHistoryAsync(new())).Revision == clearPreview.Revision, "Clear preview mutated history");
        Read(core.EditEntryAsync(clearCandidate, "clear candidate changed"));
        Expect("history_conflict", () => Read(core.ClearHistoryAsync(clearPreview.Revision)));
        var currentPreview = Read(core.GetClearHistoryPreviewAsync());
        var clearResult = Read(core.ClearHistoryAsync(currentPreview.Revision));
        Check(clearResult.Changed && clearResult.RemovedCount == 1 && clearResult.Revision == currentPreview.Revision + 1
            && clearResult.CompactionError is null, "Clear result/compaction contract");
        Check(Read(core.GetEntryAsync(saved.Id)).Content == body, "Clear removed protected history");
        Expect("not_found", () => Read(core.GetEntryAsync(clearCandidate)));
        Check(Read(core.GetHistoryChangesAsync(currentPreview.Revision)).ResyncRequired, "Clear did not invalidate old change feeds");
        Check(!Read(core.ClearHistoryAsync(clearResult.Revision)).Changed, "Empty clear advanced revision");
        const string png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAYAAAC56t6BAAAAEUlEQVR4nGP4z8DQAMIMGAwAfcEI+4thakwAAAAASUVORK5CYII=";
        long imageId = Read(core.AddManualAsync(png, "image")).Id;
        string originalImage = Read(core.GetEntryAsync(imageId)).Content;
        ulong imageRevision = Read(core.GetHistoryAsync(new())).Revision;
        var previewImage = Read(core.GetImagePreviewAsync(imageId, imageRevision));
        Check(previewImage.Id == imageId && previewImage.Revision == imageRevision
            && previewImage.Width == 2 && previewImage.Height == 3
            && Convert.FromBase64String(previewImage.PngBase64).Take(4).SequenceEqual(new byte[] { 137, 80, 78, 71 }), "Native preview DTO/PNG");
        Check(Read(core.GetHistoryAsync(new())).Revision == imageRevision
            && Read(core.GetEntryAsync(imageId)).Content == originalImage, "Preview mutated stored content");
        Read(core.SetTagsAsync(imageId, ["sensitive"]));
        ulong sensitiveRevision = Read(core.GetHistoryAsync(new())).Revision;
        Expect("history_conflict", () => Read(core.GetImagePreviewAsync(imageId, imageRevision, true)));
        Expect("sensitive_confirmation", () => Read(core.GetImagePreviewAsync(imageId, sensitiveRevision)));
        Check(Read(core.GetImagePreviewAsync(imageId, sensitiveRevision, true)).Width == 2, "Sensitive confirmed preview");
        Expect("preview_dimensions", () => Read(core.GetImagePreviewAsync(imageId, sensitiveRevision, true, maxEdge: 1025)));
        Read(core.DeleteEntriesAsync([imageId], sensitiveRevision));
        Console.WriteLine("PASS: real DLL history DTOs, paging, full formats, pin/tags/edit/delete/batch/clear/image previews, conflicts, change feed, cancellation");
        return saved.Id;
    }

    internal static void DisposeQueued(string database)
    {
        using var core = new NativeCore(database);
        var queued = Enumerable.Range(0, 32).Select(_ => core.GetHistoryAsync(new())).ToArray();
        core.Dispose();
        foreach (var task in queued)
        {
            try { Read(task); }
            catch (OperationCanceledException) { }
            catch (ObjectDisposedException) { }
        }
        using var reopened = new NativeCore(database);
        Check(Read(reopened.GetHistoryAsync(new())).StoredCount > 0, "Reopen after queued disposal");
        Console.WriteLine("PASS: queued reads finish or cancel before native close; immediate reopen succeeds");
    }
}
