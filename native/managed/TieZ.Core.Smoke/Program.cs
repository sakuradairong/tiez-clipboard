using TieZ.Core;

static void Check(bool condition, string message)
{
    if (!condition) throw new Exception(message);
}
static void Expect(string code, Action operation)
{
    try { operation(); }
    catch (CoreException e) when (e.Code == code) { return; }
    throw new Exception($"Expected {code}");
}

string directory = Path.Combine(Path.GetTempPath(), $"TieZ-原生-{Guid.NewGuid():N}");
Directory.CreateDirectory(directory);
try
{
    string database = Path.Combine(directory, "设置.db");
    ulong revision;
    long historyId;
    NativeCore disposed;
    using (var core = new NativeCore(database))
    {
        disposed = core;
        int notifications = 0;
        core.Changed += _ => notifications++;
        Check(core.Snapshot.Revision == 0 && core.Snapshot.Values["app.persistent"] == "false"
            && core.Snapshot.Values["app.privacy_protection"] == "true", "Initial snapshot");
        Expect("already_open", () => { using var duplicate = new NativeCore(database); });
        Check(!core.Update(new Dictionary<string, string> { ["app.language"] = "zh" }, 0), "No-op");
        Check(core.Update(new Dictionary<string, string> { ["app.language"] = "tw", ["app.color_mode"] = "dark" }, 0), "Save");
        Expect("revision_conflict", () => core.Update(new Dictionary<string, string> { ["app.language"] = "en" }, 0));
        Expect("invalid_value", () => core.Update(new Dictionary<string, string> { ["app.language"] = "invalid" }, 1));
        Check(core.Refresh().Values["app.language"] == "tw" && notifications == 1, "Canonical state and notification count");
        bool threadRejected = Task.Run(() =>
        {
            try { core.Refresh(); return false; }
            catch (InvalidOperationException) { return true; }
        }).GetAwaiter().GetResult();
        Check(threadRejected, "Thread ownership");
        historyId = HistorySmoke.Run(core);
        CaptureSmoke.Run(core);
        TargetSmoke.Run(core);
        revision = core.Snapshot.Revision;
    }
    disposed.Dispose();
    try { disposed.Refresh(); throw new Exception("Disposed handle accepted"); }
    catch (ObjectDisposedException) { }
    using (var reopened = new NativeCore(database))
    {
        Check(reopened.Snapshot.Revision == revision && reopened.Snapshot.Values["app.language"] == "tw", "Reopen persistence");
        Check(reopened.GetEntryAsync(historyId).GetAwaiter().GetResult().Content == " managed 中文 %_ \r\n ", "History reopen persistence");
    }
    HistorySmoke.DisposeQueued(database);
    CaptureSmoke.DisposeActive(database);
    TargetSmoke.DisposeActive(database);
    Console.WriteLine("PASS: real Rust DLL interop, Unicode path, shared ownership, validation, conflicts, no-op events, thread/dispose guards, persistence, native capture lifecycle and policy");
}
finally { Directory.Delete(directory, recursive: true); }
