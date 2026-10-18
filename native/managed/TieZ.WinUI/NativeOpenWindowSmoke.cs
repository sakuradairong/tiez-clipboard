using System.Diagnostics;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

// Exercises actual XAML buttons against the real DLL and only copied, owned child editors.
internal sealed class NativeOpenWindowSmoke(MainWindow main, NativeCore core, string directory, string report, string fixtureSource)
{
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private readonly List<string> exports = [];
    private SemaphoreSlim? gate;
    private bool gateHeld, finished;
    private TaskCompletionSource<bool>? release;
    private int checks, launches, cancellations;
    private bool actualRevisionConflict;
    private string fixture = "";
    [DllImport("winsqlite3.dll", CallingConvention = CallingConvention.Cdecl)]
    private static extern int sqlite3_open_v2([MarshalAs(UnmanagedType.LPUTF8Str)] string path, out nint database, int flags, nint vfs);
    [DllImport("winsqlite3.dll", CallingConvention = CallingConvention.Cdecl)]
    private static extern int sqlite3_exec(nint database, [MarshalAs(UnmanagedType.LPUTF8Str)] string sql, nint callback, nint argument, out nint error);
    [DllImport("winsqlite3.dll", CallingConvention = CallingConvention.Cdecl)] private static extern void sqlite3_free(nint value);
    [DllImport("winsqlite3.dll", CallingConvention = CallingConvention.Cdecl)] private static extern int sqlite3_close(nint database);
    private void Check(bool value, string message) { checks++; if (!value) throw new InvalidOperationException(message); }
    private async Task WaitUntil(Func<bool> ready, string message)
    {
        var timeout = Stopwatch.StartNew();
        while (!ready() && timeout.Elapsed < TimeSpan.FromSeconds(8)) await Task.Delay(20);
        Check(ready(), message);
    }
    private void Inject(string sql)
    {
        nint database = 0;
        try
        {
            Check(sqlite3_open_v2(Path.Combine(directory, "settings.db"), out database, 2, 0) == 0, "Owned SQLite fixture open failed");
            int result = sqlite3_exec(database, sql, 0, 0, out nint error);
            if (error != 0) sqlite3_free(error);
            Check(result == 0, $"Owned SQLite fixture setup failed ({result})");
        }
        finally { if (database != 0) Check(sqlite3_close(database) == 0, "Fixture retained its SQLite connection"); }
    }
    private async Task<(string Path, byte[] Body)> Receive()
    {
        string path = Path.Combine(fixture, "open-observed.json");
        await WaitUntil(() => File.Exists(path), "Owned editor did not report");
        using var document = JsonDocument.Parse(File.ReadAllText(path));
        var root = document.RootElement;
        int pid = root.GetProperty("Pid").GetInt32();
        try { using var process = Process.GetProcessById(pid); await process.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(5)); }
        catch (ArgumentException) { }
        var observed = (root.GetProperty("Path").GetString()!, Convert.FromBase64String(root.GetProperty("Body").GetString()!));
        File.Delete(path); launches++;
        return observed;
    }
    private async Task Show(long id, bool select = false)
    {
        var view = main.History;
        await view.ReloadAsync();
        await WaitUntil(() => !view.IsBusy && view.Rows.Any(row => row.Id == id), "History did not load");
        view.ScrollIntoHistory(id);
        await WaitUntil(() => view.IsHistoryItemVisible(id), "Owned card was not visible");
        if (select)
        {
            view.Select(id);
            await WaitUntil(() => view.CanOpen || view.CanReveal, "Selected entry did not finish loading");
        }
    }
    private async Task<OpenContentResult> Open(long id, byte[] expected, bool card = true)
    {
        var view = main.History;
        await WaitUntil(() => card ? view.CardCanOpen(id) : view.CanOpen, "Native open control was disabled");
        ulong settingsRevision = core.Snapshot.Revision;
        var previous = view.LastOpenResult;
        if (card) view.InvokeCardOpen(id); else view.Invoke("open");
        await WaitUntil(() => !view.ContentOpenPending && view.LastOpenResult is { } result && !ReferenceEquals(previous, result), "Native open did not return success");
        var result = view.LastOpenResult!;
        var observed = await Receive();
        Check(result.Id == id && result.Route == "desktop" && observed.Body.SequenceEqual(expected), "Native open lost ID, route or exact bytes");
        if (result.Exported) { exports.Add(observed.Path); Check(view.OpenFeedback.Contains("修改不会回写历史"), "Temporary-copy limitation was not disclosed"); }
        Check(core.Refresh().Revision == settingsRevision, "Opening saved settings");
        Check(!view.ContentOpenPending && !view.HasError, "Accepted launch was presented as failure");
        return result;
    }
    private void HoldGate()
    {
        gate ??= (SemaphoreSlim)typeof(NativeCore).GetField("openGate", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(core)!;
        Check(gate.Wait(0), "Owned open gate was already held"); gateHeld = true;
    }
    private void ReleaseGate() { if (gateHeld) { gate!.Release(); gateHeld = false; } }
    private async Task CancelQueued(long id, Action invalidate, bool reveal = false)
    {
        var view = main.History;
        await Show(id, select: reveal);
        if (reveal) view.Invoke("reveal");
        await WaitUntil(() => view.CardCanOpen(id), "Queued-test card disabled");
        int before = (await core.GetEntryAsync(id)).UseCount;
        int activeExports = (await core.PollOpenContentAsync()).ActiveExports;
        HoldGate();
        try
        {
            view.InvokeCardOpen(id);
            await WaitUntil(() => view.ContentOpenPending, "Real request did not enter the owned gate");
            Check(!view.CanOpen && !view.CardCanOpen(id), "Duplicate open was not disabled");
            invalidate();
            await WaitUntil(() => !view.ContentOpenPending, "Invalidation did not cancel queued open");
            Check(view.LastOpenResult is null && view.OpenFeedback.Contains("取消"), "Queued cancellation claimed success");
        }
        finally { ReleaseGate(); }
        Check((await core.GetEntryAsync(id)).UseCount == before && (await core.PollOpenContentAsync()).ActiveExports == activeExports,
            "Cancelled queued open changed usage or exported content");
        Check(!File.Exists(Path.Combine(fixture, "open-observed.json")), "Cancelled queue launched an editor");
        cancellations++;
    }
    internal async void Start()
    {
        try
        {
            var view = main.History;
            await WaitUntil(() => main.ResidentMode && !view.IsBusy, "Main/core shell was not ready");
            fixture = Path.Combine(directory, "原生编辑器 %20 ' &"); Directory.CreateDirectory(fixture);
            foreach (string file in new[] { "TieZ.Core.Smoke.exe", "TieZ.Core.Smoke.dll", "TieZ.Core.Smoke.deps.json", "TieZ.Core.Smoke.runtimeconfig.json", "TieZ.Core.dll" })
                File.Copy(Path.Combine(fixtureSource, file), Path.Combine(fixture, file));
            File.WriteAllText(Path.Combine(fixture, "open-fixture.marker"), "owned child");
            string executable = Path.Combine(fixture, "TieZ.Core.Smoke.exe");
            Check(core.Update(new Dictionary<string,string> { ["app.text"] = executable, ["app.rich_text"] = executable,
                ["app.image"] = executable, ["app.file"] = executable }, core.Snapshot.Revision), "Opener settings not accepted");
            for (int index = 0; index < 56; index++) await core.AddManualAsync($"filler {index:000}");
            const string body = "  原样文本\r\n\t  ";
            const string html = "<table><tr><td><b>原始</b></td></tr></table>";
            const string png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGMQVDL+DwACFAFmBODefwAAAABJRU5ErkJggg==";
            long publicId = (await core.AddManualAsync(body)).Id;
            long otherId = (await core.AddManualAsync("other selection")).Id;
            long richId = (await core.AddManualAsync("rich fallback", "rich_text", html)).Id;
            long privateId = (await core.AddManualAsync("private 原样\r\n ", tags: ["password"])).Id;
            long imageId = (await core.AddManualAsync("data:image/png;base64," + png, "image")).Id;
            string sourceFile = Path.Combine(directory, "原始 文件 %20.txt"); File.WriteAllText(sourceFile, body);
            long fileId = (await core.AddManualAsync(sourceFile, "file")).Id;
            long badUrlId = (await core.AddManualAsync("file:///C:/Windows/cmd.exe", "url")).Id;
            main.AppWindow.Resize(new SizeInt32(1680, 1080));
            await view.ReloadAsync(); await WaitUntil(() => !view.IsBusy && view.CanChangeLayout, "Initial history busy");
            view.ChooseLayout("grid"); await Show(publicId);
            view.SelectMany(otherId, richId);
            await WaitUntil(() => !view.IsBusy && view.CardCanOpen(publicId), "Public direct action unavailable under multi-selection");
            long[] selection = view.SelectedHistoryIds;
            var opened = await Open(publicId, Encoding.UTF8.GetBytes(body));
            Check(opened.UsageUpdated && opened.UsageError is null && view.SelectedHistoryIds.Order().SequenceEqual(selection.Order()), "Direct card open changed multi-selection or missed usage");
            await Show(richId); await Open(richId, Encoding.UTF8.GetBytes(html));
            await Show(imageId); await Open(imageId, Convert.FromBase64String(png));
            await Show(fileId); opened = await Open(fileId, Encoding.UTF8.GetBytes(body));
            Check(!opened.Exported && File.ReadAllText(sourceFile) == body, "Existing file was exported or changed");
            await Show(privateId, select: true);
            Check(view.CanReveal && !view.CanOpen && !view.CardCanOpen(privateId), "Sensitive entry opened without reveal");
            view.Invoke("reveal"); await WaitUntil(() => view.CanOpen && view.CardCanOpen(privateId), "Sensitive reveal did not authorize open");
            await Open(privateId, Encoding.UTF8.GetBytes("private 原样\r\n "));
            await Show(privateId, select: true);
            Check(!view.CanOpen && !view.CardCanOpen(privateId), "Reread retained sensitive consent");
            await Show(publicId, select: true);
            view.ChooseLayout("list"); await WaitUntil(() => !view.IsBusy && view.CanOpen, "List toolbar did not load");
            await Open(publicId, Encoding.UTF8.GetBytes(body), card: false);
            await view.ReloadAsync(); await WaitUntil(() => !view.IsBusy && view.CanChangeLayout, "Layout remained busy");
            view.ChooseLayout("grid");
            core.Update(new Dictionary<string,string> { ["app.compact_mode"] = "true" }, core.Snapshot.Revision);
            await Show(publicId); await Open(publicId, Encoding.UTF8.GetBytes(body));
            await Show(badUrlId); int beforeLaunches = launches;
            await WaitUntil(() => view.CardCanOpen(badUrlId), "Invalid URL control was not enabled");
            view.InvokeCardOpen(badUrlId);
            await WaitUntil(() => !view.ContentOpenPending && view.HasError, "Invalid URL did not fail visibly");
            Check(view.LastOpenResult is null && launches == beforeLaunches && (await core.GetEntryAsync(badUrlId)).UseCount == 0,
                "Invalid URL launched or counted");
            await Show(publicId);
            Inject("CREATE TRIGGER ui_usage_failure BEFORE UPDATE OF use_count ON clipboard_history BEGIN SELECT RAISE(ABORT,'private owned fixture'); END;");
            int beforeCount = (await core.GetEntryAsync(publicId)).UseCount;
            opened = await Open(publicId, Encoding.UTF8.GetBytes(body));
            Check(!opened.UsageUpdated && opened.UsageError is not null && view.OpenFeedbackTitle.Contains("已打开，使用次数未更新")
                && view.OpenFeedback.Contains("请勿") && !view.OpenFeedback.Contains("private"), "Real accounting failure was hidden or misreported as launch failure");
            Check((await core.GetEntryAsync(publicId)).UseCount == beforeCount, "Failed accounting changed count");
            Inject("DROP TRIGGER ui_usage_failure;");
            await CancelQueued(publicId, main.HideToTray); main.ShowMain();
            await CancelQueued(publicId, main.NavigateTags); main.NavigateHistory();
            await CancelQueued(privateId, () => view.Select(otherId), reveal: true);
            Check(!view.CardCanOpen(privateId), "Selection change retained sensitive authorization");
            await CancelQueued(publicId, () => core.Update(new Dictionary<string,string> { ["app.history_view_mode"] = "list" }, core.Snapshot.Revision));
            await WaitUntil(() => !view.IsBusy && view.CanChangeLayout, "Layout cancellation did not settle");
            view.ChooseLayout("grid");
            await CancelQueued(publicId, () => core.Update(new Dictionary<string,string> { ["app.compact_mode"] = "false" }, core.Snapshot.Revision));
            await CancelQueued(publicId, () => view.Search("not a match"));
            view.Search(""); await view.ReloadAsync();
            await CancelQueued(publicId, view.ScrollToEnd);
            await Show(otherId);
            await WaitUntil(() => view.CardCanOpen(otherId), "Conflict fixture card disabled");
            HoldGate();
            try
            {
                view.InvokeCardOpen(otherId);
                await WaitUntil(() => view.ContentOpenPending, "Conflict request did not queue");
                await core.EditEntryAsync(otherId, "changed while queued");
            }
            finally { ReleaseGate(); }
            await WaitUntil(() => !view.ContentOpenPending && view.HasError, "Real stale request was not rejected");
            actualRevisionConflict = view.OpenFeedback.Contains("记录已变化") && view.LastOpenResult is null;
            Check(actualRevisionConflict && (await core.GetEntryAsync(otherId)).UseCount == 0
                && !File.Exists(Path.Combine(fixture, "open-observed.json")), "Conflict launched or counted stale content");
            string missingHandler = Path.Combine(directory, "missing-owned-handler.exe");
            core.Update(new Dictionary<string,string> { ["app.text"] = missingHandler }, core.Snapshot.Revision);
            await Show(otherId);
            await WaitUntil(() => view.CardCanOpen(otherId), "Missing-handler fixture card disabled");
            int beforeExports = (await core.PollOpenContentAsync()).ActiveExports;
            view.InvokeCardOpen(otherId);
            await WaitUntil(() => !view.ContentOpenPending && view.HasError, "Handler failure was not visible");
            Check(view.LastOpenResult is null && !view.OpenFeedback.Contains(missingHandler)
                && (await core.GetEntryAsync(otherId)).UseCount == 0 && (await core.PollOpenContentAsync()).ActiveExports == beforeExports,
                "Handler failure counted, leaked its path or retained a failed export");
            core.Update(new Dictionary<string,string> { ["app.text"] = executable }, core.Snapshot.Revision);
            await Show(publicId);
            release = new(TaskCreationOptions.RunContinuationsAsynchronously);
            var accepted = new TaskCompletionSource<OpenContentResult>(TaskCreationOptions.RunContinuationsAsynchronously);
            CancellationToken submittedToken = default;
            view.ContentOpener = async (request, token) =>
            {
                submittedToken = token;
                var result = await core.OpenContentAsync(request.Id, request.Revision, request.AllowSensitive, token);
                accepted.TrySetResult(result);
                await release.Task; // Model a submitted non-abortable response; keep its actual outcome.
                return result;
            };
            await WaitUntil(() => view.CardCanOpen(publicId), "Accepted-race card disabled");
            view.InvokeCardOpen(publicId);
            var acceptedResult = await accepted.Task.WaitAsync(TimeSpan.FromSeconds(8));
            var acceptedExport = await Receive(); exports.Add(acceptedExport.Path);
            main.HideToTray(); Check(submittedToken.IsCancellationRequested, "Hide did not request cancellation");
            release.TrySetResult(true);
            await WaitUntil(() => !view.ContentOpenPending && ReferenceEquals(view.LastOpenResult, acceptedResult), "Submitted success was discarded after cancellation");
            Check(view.OpenFeedbackTitle == "已打开" && !view.HasError && acceptedExport.Body.SequenceEqual(Encoding.UTF8.GetBytes(body)), "Submitted cancellation became false failure");
            main.ShowMain(); view.ContentOpener = null;
            await Show(publicId); await WaitUntil(() => view.CardCanOpen(publicId), "Exit fixture did not become ready");
            HoldGate(); view.InvokeCardOpen(publicId);
            await WaitUntil(() => view.ContentOpenPending, "Exit fixture did not queue open");
            Task<bool> quit = main.Quit();
            Check(!view.CanOpen && view.ContentOpener is null, "Exit retained open controls or fixture delegate");
            ReleaseGate(); Check(await quit.WaitAsync(TimeSpan.FromSeconds(8)), "Exit failed after queued open cancellation");
            Check(!view.ContentOpenPending && view.LastOpenResult is null && exports.All(path => !File.Exists(path)), "Exit retained pending state, result or exports");
            Finish(true, null);
        }
        catch (Exception failure)
        {
            ReleaseGate(); release?.TrySetResult(true);
            main.History.ContentOpener = null;
            Finish(false, failure.ToString());
            await main.Quit();
        }
    }
    private void Finish(bool passed, string? error)
    {
        if (finished) return; finished = true;
        bool webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
            .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
        try
        {
            Check(!webView2ModuleLoaded, "WebView2 loaded during native open");
            if (passed)
            {
                string path = Path.GetFullPath(directory);
                Check(Path.GetDirectoryName(path) == Path.GetFullPath(Path.GetTempPath()).TrimEnd(Path.DirectorySeparatorChar)
                    && Path.GetFileName(path).StartsWith("TieZ-WinUI-原生-", StringComparison.Ordinal)
                    && (File.GetAttributes(path) & FileAttributes.ReparsePoint) == 0, "Cleanup target left owned fixture namespace");
                Directory.Delete(path, recursive: true);
            }
        }
        catch (Exception failure) { passed = false; error = $"{error}\n{failure}"; }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "history-content-open", checks,
            realNativeEditorLaunches = launches, queuedCancellations = cancellations, actualRevisionConflict, webView2ModuleLoaded, elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
