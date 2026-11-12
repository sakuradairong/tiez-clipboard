using TieZ.Core;

internal static class QueueSmoke
{
    internal static void Run(string database)
    {
        int checks = 0;
        void Check(bool condition, string reason) { checks++; if (!condition) throw new Exception(reason); }
        void Error(string code, Action action)
        {
            try { action(); } catch (CoreException error) when (error.Code == code) { checks++; return; }
            throw new Exception($"Expected {code}");
        }
        using (var core = new NativeCore(database))
        {
            ulong settings = core.Snapshot.Revision;
            var a = core.AddManualAsync("  队列\r\n\t ").GetAwaiter().GetResult();
            var b = core.AddManualAsync("second").GetAwaiter().GetResult();
            var empty = core.GetPasteQueueAsync().GetAwaiter().GetResult();
            Check(empty.Ids.Count == 0 && !empty.RequiresReset, "Queue starts empty");
            long[] ids = [b.Id, a.Id, b.Id];
            var pending = core.ReplacePasteQueueAsync(ids, empty.Revision, empty.HistoryRevision);
            ids[0] = a.Id;
            var queue = pending.GetAwaiter().GetResult();
            Check(queue.Ids.SequenceEqual(new[] { b.Id, a.Id, b.Id }), "Input snapshot preserves order and duplicates");
            Error("queue_conflict", () => core.ReplacePasteQueueAsync([], empty.Revision, queue.HistoryRevision).GetAwaiter().GetResult());
            Error("not_found", () => core.ReplacePasteQueueAsync([long.MaxValue], queue.Revision, queue.HistoryRevision).GetAwaiter().GetResult());
            core.StopTargetTrackingAsync().GetAwaiter().GetResult();
            try { core.PasteNextAsync(queue, 0).GetAwaiter().GetResult(); throw new Exception("Stopped target was accepted"); }
            catch (CoreException error) { Check(error.Code.StartsWith("target", StringComparison.Ordinal), "Stopped target fails before clipboard I/O"); }
            Check(core.GetPasteQueueAsync().GetAwaiter().GetResult().Ids.SequenceEqual(queue.Ids), "Failed paste retains queue");
            using (var cancel = new CancellationTokenSource())
            {
                cancel.Cancel();
                try { core.ReplacePasteQueueAsync([], queue.Revision, queue.HistoryRevision, cancel.Token).GetAwaiter().GetResult(); throw new Exception("Cancellation ignored"); }
                catch (OperationCanceledException) { checks++; }
            }
            Check(core.GetPasteQueueAsync().GetAwaiter().GetResult().Ids.SequenceEqual(queue.Ids), "Cancelled clear retains queue");
            core.DeleteEntryAsync(b.Id).GetAwaiter().GetResult();
            queue = core.GetPasteQueueAsync().GetAwaiter().GetResult();
            Check(queue.Ids.SequenceEqual(new[] { a.Id }), "Deletion removes all duplicate occurrences");
            Check(core.GetEntryAsync(a.Id).GetAwaiter().GetResult().Content == "  队列\r\n\t ", "Queue never rewrites original whitespace");
            bool owner = Task.Run(() => { try { core.GetPasteQueueAsync().GetAwaiter().GetResult(); return false; } catch (InvalidOperationException) { return true; } }).GetAwaiter().GetResult();
            Check(owner, "Owner thread enforced");
            Check(core.Refresh().Revision == settings, "Queue reads and actions never save settings");
            Check(core.ReplacePasteQueueAsync([], queue.Revision, queue.HistoryRevision).GetAwaiter().GetResult().Ids.Count == 0, "Explicit clear");
        }
        using (var reopened = new NativeCore(database))
            Check(reopened.GetPasteQueueAsync().GetAwaiter().GetResult().Ids.Count == 0, "Reopen has no queue or extra service");
        Console.WriteLine($"PASS: queue {checks} real-DLL checks; no clipboard writes or injected input");
    }
}
