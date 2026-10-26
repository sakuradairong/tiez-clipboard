using System.Text.Json;

namespace TieZ.Core;

public sealed partial class NativeCore
{
    // Call from the owner thread; await on that thread before updating a view.
    public Task<HistoryPage> GetHistoryAsync(HistoryQuery query, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<HistoryPage>(new { op = "history_page", handle, query }, cancellationToken);

    public Task<ClipboardEntry> GetEntryAsync(long id, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<ClipboardEntry>(new { op = "history_entry", handle, id }, cancellationToken);

    public Task<HistoryChanges> GetHistoryChangesAsync(ulong afterRevision, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<HistoryChanges>(new { op = "history_changes", handle, after_revision = afterRevision }, cancellationToken);

    public Task<HistoryMutation> AddManualAsync(string content, string contentType = "text",
        string? htmlContent = null, IReadOnlyList<string>? tags = null, CancellationToken cancellationToken = default)
    {
        VerifyAccess(); ArgumentNullException.ThrowIfNull(content); ArgumentNullException.ThrowIfNull(contentType);
        string[] copiedTags = tags?.ToArray() ?? [];
        return NeedsHistoryStaging(content, htmlContent, contentType, copiedTags)
            ? LargeHistoryMutationAsync(new LargeHistoryPayload(content, contentType, htmlContent, copiedTags), null, cancellationToken)
            : HistoryRequestAsync<HistoryMutation>(new { op = "history_add", handle,
                content_type = contentType, content, html_content = htmlContent, tags = copiedTags }, cancellationToken);
    }

    public Task<HistoryMutation> SetPinnedAsync(long id, bool isPinned, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<HistoryMutation>(new { op = "history_pin", handle, id, is_pinned = isPinned }, cancellationToken);

    public Task<HistoryMutation> EditEntryAsync(long id, string content, CancellationToken cancellationToken = default)
    {
        VerifyAccess(); ArgumentNullException.ThrowIfNull(content);
        // This method is an explicit edit authorization, as in the existing small-edit API.
        // Views continue to require their ordinary sensitive reveal before enabling editing.
        return NeedsHistoryStaging(content)
            ? LargeHistoryMutationAsync(new LargeHistoryPayload(content, null, null, []), id, cancellationToken)
            : HistoryRequestAsync<HistoryMutation>(new { op = "history_edit", handle, id, content }, cancellationToken);
    }

    public Task<HistoryMutation> SetTagsAsync(long id, IReadOnlyList<string> tags, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<HistoryMutation>(new { op = "history_tags", handle, id, tags }, cancellationToken);

    public Task<HistoryMutation> DeleteEntryAsync(long id, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<HistoryMutation>(new { op = "history_delete", handle, id }, cancellationToken);

    public Task<HistoryMutation> DeleteEntriesAsync(IReadOnlyList<long> ids, ulong expectedRevision,
        CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<HistoryMutation>(new { op = "history_delete_many", handle, ids,
            expected_revision = expectedRevision }, cancellationToken);

    public Task<HistoryClearPreview> GetClearHistoryPreviewAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<HistoryClearPreview>(new { op = "history_clear_preview", handle }, cancellationToken);

    public Task<HistoryClearResult> ClearHistoryAsync(ulong expectedRevision, CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<HistoryClearResult>(new { op = "history_clear", handle,
            expected_revision = expectedRevision }, cancellationToken);

    private async Task<T> HistoryRequestAsync<T>(object request, CancellationToken cancellationToken)
    {
        VerifyAccess();
        // Snapshot input before yielding: a caller must not mutate a pending request's tags/query.
        byte[] bytes = SerializeRequest(request);
        using var cancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, shutdown.Token);
        await requestGate.WaitAsync(cancellation.Token).ConfigureAwait(false);
        try
        {
            ObjectDisposedException.ThrowIf(handle == 0, this);
            return await Task.Run(() =>
            {
                // Cancellation stops queued work. Once a mutation reaches Rust, report its actual
                // result even if cancellation arrives, so an accepted save is never called cancelled.
                cancellation.Token.ThrowIfCancellationRequested();
                return RequestBytes(bytes).Deserialize<T>(WireJson)
                    ?? throw new CoreException("invalid_response", "Native core returned an empty result.");
            }).ConfigureAwait(false);
        }
        finally { requestGate.Release(); }
    }
}
