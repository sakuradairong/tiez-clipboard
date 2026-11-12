namespace TieZ.Core;

public sealed record PasteQueueSnapshot(ulong Revision, ulong HistoryRevision, IReadOnlyList<long> Ids,
    bool RequiresReset, string? LastError);

public sealed partial class NativeCore
{
    public Task<PasteQueueSnapshot> GetPasteQueueAsync(CancellationToken cancellationToken = default) =>
        HistoryRequestAsync<PasteQueueSnapshot>(new { op = "queue_snapshot", handle }, cancellationToken);

    public Task<PasteQueueSnapshot> ReplacePasteQueueAsync(IReadOnlyList<long> ids, ulong expectedQueueRevision,
        ulong expectedHistoryRevision, CancellationToken cancellationToken = default)
    {
        VerifyAccess(); ArgumentNullException.ThrowIfNull(ids);
        long[] copied = ids.ToArray();
        if (copied.Length > 1000 || copied.Contains(0)) throw new ArgumentException("Queue requires at most 1000 nonzero IDs.", nameof(ids));
        return HistoryRequestAsync<PasteQueueSnapshot>(new { op = "queue_replace", handle, ids = copied,
            expected_queue_revision = expectedQueueRevision, expected_history_revision = expectedHistoryRevision }, cancellationToken);
    }

    public Task<QuickPasteResult> PasteNextAsync(PasteQueueSnapshot queue, ulong expectedGeneration,
        bool allowSensitive = false, CancellationToken cancellationToken = default)
    {
        VerifyAccess(); ArgumentNullException.ThrowIfNull(queue);
        return HistoryRequestAsync<QuickPasteResult>(new { op = "queue_paste", handle,
            expected_queue_revision = queue.Revision, expected_history_revision = queue.HistoryRevision,
            expected_generation = expectedGeneration, allow_sensitive = allowSensitive }, cancellationToken);
    }
}
