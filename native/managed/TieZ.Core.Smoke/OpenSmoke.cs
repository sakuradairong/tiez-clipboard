using System.Diagnostics;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using TieZ.Core;

internal static class OpenSmoke
{
    private static void Check(bool ok, string message) { if (!ok) throw new Exception(message); }
    private static T Read<T>(Task<T> task) => task.GetAwaiter().GetResult();
    private static void Expect(string code, Action action)
    {
        try { action(); } catch (CoreException e) when (e.Code == code) { return; }
        throw new Exception($"Expected {code}");
    }
    [DllImport("tiez_native_core", EntryPoint = "tiez_core_request", CallingConvention = CallingConvention.Cdecl)]
    private static extern nint RawRequest([In] byte[] bytes, nuint length);
    [DllImport("tiez_native_core", EntryPoint = "tiez_core_free", CallingConvention = CallingConvention.Cdecl)]
    private static extern void RawFree(nint value);
    private static JsonElement Raw(object command)
    {
        byte[] bytes = JsonSerializer.SerializeToUtf8Bytes(command);
        nint response = RawRequest(bytes, (nuint)bytes.Length);
        try { using var result = JsonDocument.Parse(Marshal.PtrToStringUTF8(response)!); return result.RootElement.Clone(); }
        finally { RawFree(response); }
    }
    internal static bool TryFixture(string[] arguments)
    {
        if (!File.Exists(Path.Combine(AppContext.BaseDirectory, "open-fixture.marker"))) return false;
        Check(arguments.Length == 1, "Fixture requires one unchanged target argument");
        byte[] body = File.ReadAllBytes(arguments[0]);
        string report = JsonSerializer.Serialize(new { Path = arguments[0], Body = Convert.ToBase64String(body), Pid = Environment.ProcessId });
        string pending = Path.Combine(AppContext.BaseDirectory, "open-observed.pending");
        File.WriteAllText(pending, report);
        File.Move(pending, Path.Combine(AppContext.BaseDirectory, "open-observed.json"));
        return true;
    }
    private static (string Path, byte[] Body) Receive(string directory)
    {
        string report = Path.Combine(directory, "open-observed.json");
        var watch = Stopwatch.StartNew();
        while (!File.Exists(report) && watch.Elapsed < TimeSpan.FromSeconds(8)) Thread.Sleep(10);
        Check(File.Exists(report), "Native open did not reach the owned fixture");
        using var data = JsonDocument.Parse(File.ReadAllText(report));
        int pid = data.RootElement.GetProperty("Pid").GetInt32();
        try { using var process = Process.GetProcessById(pid); Check(process.WaitForExit(5000), "Fixture did not exit"); }
        catch (ArgumentException) { }
        var result = (data.RootElement.GetProperty("Path").GetString()!, Convert.FromBase64String(data.RootElement.GetProperty("Body").GetString()!));
        File.Delete(report); return result;
    }
    internal static void Run(string directory)
    {
        Directory.CreateDirectory(directory);
        string fixture = Path.Combine(directory, "编辑器 %20 ' &"); Directory.CreateDirectory(fixture);
        foreach (string name in new[] { "TieZ.Core.Smoke.exe", "TieZ.Core.Smoke.dll", "TieZ.Core.Smoke.deps.json", "TieZ.Core.Smoke.runtimeconfig.json", "TieZ.Core.dll" })
            File.Copy(Path.Combine(AppContext.BaseDirectory, name), Path.Combine(fixture, name));
        File.WriteAllText(Path.Combine(fixture, "open-fixture.marker"), "owned test child");
        string database = Path.Combine(directory, "open.db");
        var core = new NativeCore(database);
        var exported = new List<string>();
        try
        {
            var initial = Read(core.PollOpenContentAsync());
            Check(initial.ActiveExports == 0 && initial.ExportBytes == 0 && !initial.Closed && initial.CleanupError is null, "Initial open status");
            Check(core.Update(new Dictionary<string, string> { ["app.text"] = Path.Combine(fixture, "TieZ.Core.Smoke.exe") }, core.Snapshot.Revision), "Save opener selection");
            ulong settingsRevision = core.Snapshot.Revision;
            Expect("invalid_value", () => core.Update(new Dictionary<string, string> { ["app.text"] = "cmd.exe /c anything" }, settingsRevision));
            foreach (bool sensitive in new[] { false, true })
            {
                string body = $"  managed 原始 {sensitive}\r\n\t  ";
                long id = Read(core.AddManualAsync(body, sensitive ? "code" : "text", tags: sensitive ? ["sensitive"] : [])).Id;
                ulong revision = Read(core.GetHistoryAsync(new())).Revision;
                Expect("history_conflict", () => Read(core.OpenContentAsync(id, revision - 1, sensitive)));
                if (sensitive) Expect("sensitive_confirmation", () => Read(core.OpenContentAsync(id, revision)));
                var result = Read(core.OpenContentAsync(id, revision, sensitive));
                var observed = Receive(fixture);
                Check(result.Id == id && result.HistoryRevision == revision && result.Route == "desktop" && result.Exported && result.EditWatchRequired
                    && !result.EditWritebackAvailable && !result.UsageUpdated, "Open result contract");
                Check(observed.Body.SequenceEqual(Encoding.UTF8.GetBytes(body)), "Exported bytes changed");
                var entry = Read(core.GetEntryAsync(id));
                Check(entry.Content == body && entry.UseCount == 0 && Read(core.GetHistoryAsync(new())).Revision == revision, "Open changed source/history");
                exported.Add(observed.Path);
            }
            Check(Read(core.PollOpenContentAsync()).ActiveExports == 2 && core.Refresh().Revision == settingsRevision, "Poll saved settings or lost exports");
            // The separate open gate must not stall synchronous settings work or queued cancellation.
            var gate = (SemaphoreSlim)typeof(NativeCore).GetField("openGate", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(core)!;
            gate.Wait();
            try
            {
                using var cancel = new CancellationTokenSource();
                var pending = core.PollOpenContentAsync(cancel.Token);
                Check(!pending.IsCompleted, "Open queue was not held");
                core.Refresh(); cancel.Cancel();
                try { Read(pending); throw new Exception("Queued open poll ignored cancellation"); } catch (OperationCanceledException) { }
            }
            finally { gate.Release(); }
            using (var locked = new FileStream(exported[0], FileMode.Open, FileAccess.Read, FileShare.None))
            {
                Expect("open_cleanup", core.Dispose);
                Check(core.IsClosing && File.Exists(exported[0]), "Failed close lost ownership/state");
                ulong handle = (ulong)typeof(NativeCore).GetField("handle", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(core)!;
                Check(Raw(new { op = "snapshot", handle }).GetProperty("error").GetProperty("code").GetString() == "core_closing", "Rust closing guard allowed ordinary work");
                var status = Raw(new { op = "content_open_poll", handle }).GetProperty("result");
                Check(status.GetProperty("closed").GetBoolean() && status.GetProperty("active_exports").GetInt32() == 1
                    && status.GetProperty("cleanup_error").GetProperty("code").GetString() == "open_cleanup", "Failed close poll lost tracked error");
                try { core.Refresh(); throw new Exception("Closing core accepted new work"); } catch (ObjectDisposedException) { }
                Expect("already_open", () => { using var duplicate = new NativeCore(database); });
            }
            core.Dispose(); Check(!core.IsClosing && exported.All(p => !File.Exists(p)), "Dispose retry did not clean exports");
            using var reopened = new NativeCore(database);
            Check(reopened.Snapshot.Values["app.text"] == Path.Combine(fixture, "TieZ.Core.Smoke.exe") && Read(reopened.PollOpenContentAsync()).ActiveExports == 0, "Reopen created exports or changed opener");
        }
        finally { core.Dispose(); }
        // A held open queue proves close initiation returns immediately and duplicate
        // async requests borrow one task, while ordinary work is rejected as shutdown starts.
        var asyncCore = new NativeCore(Path.Combine(directory, "async-close.db"));
        asyncCore.Update(new Dictionary<string, string> { ["app.text"] = Path.Combine(fixture, "TieZ.Core.Smoke.exe") }, asyncCore.Snapshot.Revision);
        long asyncId = Read(asyncCore.AddManualAsync("  async 原始\r\n ")).Id;
        ulong asyncRevision = Read(asyncCore.GetHistoryAsync(new())).Revision;
        Read(asyncCore.OpenContentAsync(asyncId, asyncRevision));
        string asyncExport = Receive(fixture).Path;
        using var asyncLocked = new FileStream(asyncExport, FileMode.Open, FileAccess.Read, FileShare.None);
        var asyncGate = (SemaphoreSlim)typeof(NativeCore).GetField("openGate", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(asyncCore)!;
        asyncGate.Wait();
        Task close = asyncCore.DisposeAsync().AsTask();
        Check(!close.IsCompleted && asyncCore.IsClosing, "Async close blocked initiation or lost closing state");
        Check(ReferenceEquals(close, asyncCore.DisposeAsync().AsTask()), "Duplicate async close did not borrow its task");
        try { asyncCore.Refresh(); throw new Exception("Async closing client accepted work"); } catch (ObjectDisposedException) { }
        asyncGate.Release(); Expect("open_cleanup", () => close.GetAwaiter().GetResult());
        Check(asyncCore.IsClosing && File.Exists(asyncExport), "Async cleanup failure lost handle/export ownership");
        asyncLocked.Dispose();
        asyncCore.DisposeAsync().GetAwaiter().GetResult();
        Check(!asyncCore.IsClosing && !File.Exists(asyncExport), "Async close retry did not release handle/export");
        asyncCore.DisposeAsync().GetAwaiter().GetResult(); asyncCore.Dispose();
        Console.WriteLine("PASS: real DLL native open DTOs, saved handlers, sensitive/obsolete guards, actual desktop fixture bytes, independent queue/cancellation, locked close ownership and retry");
    }
}
