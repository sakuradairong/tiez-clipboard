using TieZ.Core;

internal static class CaptureSmoke
{
    private static T Read<T>(Task<T> task) => task.GetAwaiter().GetResult();
    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
    internal static void Run(NativeCore core)
    {
        try { Read(core.CopyEntryAsync(long.MaxValue)); throw new InvalidOperationException("Missing copy target accepted"); }
        catch (CoreException error) when (error.Code == "not_found") { }
        var copyFixture = Read(core.AddManualAsync("revision copy fixture"));
        Read(core.EditEntryAsync(copyFixture.Id, "revision copy changed"));
        ulong historyRevision = Read(core.GetHistoryAsync(new())).Revision;
        try { Read(core.CopyEntryAsync(copyFixture.Id, expectedRevision: copyFixture.Revision)); throw new InvalidOperationException("Stale copy accepted"); }
        catch (CoreException error) when (error.Code == "history_conflict") { }
        Check(Read(core.GetHistoryAsync(new())).Revision == historyRevision && Read(core.GetEntryAsync(copyFixture.Id)).Content == "revision copy changed",
            "Rejected copy mutated history");
        Read(core.DeleteEntryAsync(copyFixture.Id));
        var before = Read(core.GetCaptureStatusAsync());
        Check(!before.Running && before.Starts == 0, "Open must not capture implicitly");
        var started = Read(core.StartCaptureAsync());
        Check(started.Running && started.Starts == 1, "Start native listener");
        Check(Read(core.StartCaptureAsync()).Starts == 1, "Idempotent native start");
        var stopped = Read(core.StopCaptureAsync());
        Check(!stopped.Running && stopped.Starts == 1, "Stop native listener and join worker");
        Check(!Read(core.StopCaptureAsync()).Running, "Idempotent native stop");
        var changed = core.Update(new Dictionary<string, string> { ["app.capture_rich_text"] = "true" }, core.Snapshot.Revision);
        Check(changed && Read(core.GetCaptureStatusAsync()).PolicyRevision == core.Snapshot.Revision, "Acknowledged policy revision");
        Check(Read(core.StartCaptureAsync()).Starts == 2, "Restart listener");
        Read(core.StopCaptureAsync());
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();
        try { Read(core.StartCaptureAsync(cancellation.Token)); throw new InvalidOperationException("Cancelled start ran"); }
        catch (OperationCanceledException) { }
        Check(Read(core.GetCaptureStatusAsync()).Starts == 2, "Cancelled request did not start service");
    }
    internal static void DisposeActive(string database)
    {
        using (var active = new NativeCore(database))
        {
            Check(Read(active.StartCaptureAsync()).Running, "Start before disposal");
        }
        using var reopened = new NativeCore(database);
        var status = Read(reopened.GetCaptureStatusAsync());
        Check(!status.Running && status.Starts == 0, "Disposal joins native service before reopen");
        Check(Read(reopened.StartCaptureAsync()).Running, "Listener resources reusable after disposal");
    }
}
