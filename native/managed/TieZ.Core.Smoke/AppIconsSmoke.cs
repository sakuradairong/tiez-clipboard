using System.Diagnostics;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text.Json;
using TieZ.Core;

internal static class AppIconsSmoke
{
    private static int checks;
    private static readonly List<string> notes = new();
    private static readonly TimeSpan Bound = TimeSpan.FromSeconds(30);
    private const BindingFlags Instance = BindingFlags.Instance | BindingFlags.NonPublic;
    private const BindingFlags Static = BindingFlags.Static | BindingFlags.NonPublic;

    private static void Check(bool value, string message) { checks++; if (!value) throw new Exception(message); }
    private static T Read<T>(Task<T> task) => task.WaitAsync(Bound).GetAwaiter().GetResult();
    private static void Await(Task task) => task.WaitAsync(Bound).GetAwaiter().GetResult();
    private static void Cancelled(Task task)
    {
        try { task.WaitAsync(Bound).GetAwaiter().GetResult(); }
        catch (OperationCanceledException) { checks++; return; }
        throw new Exception("Expected a cancelled icon request");
    }
    private static void Cancelled(Action operation)
    {
        try { operation(); }
        catch (OperationCanceledException) { checks++; return; }
        throw new Exception("Expected a cancelled icon request");
    }
    private static void Expect(string code, Action operation)
    {
        try { operation(); }
        catch (CoreException failure) when (failure.Code == code) { checks++; return; }
        throw new Exception($"Expected {code}");
    }
    private static ulong Handle(NativeCore core) => (ulong)typeof(NativeCore).GetField("handle", Instance)!.GetValue(core)!;
    private static SemaphoreSlim Gate(NativeCore core, string name) => (SemaphoreSlim)typeof(NativeCore).GetField(name, Instance)!.GetValue(core)!;
    private static int Counter(NativeCore core, string name) => (int)typeof(NativeCore).GetField(name, Instance)!.GetValue(core)!;
    private static MethodInfo Method(string name, params Type[] parameters) =>
        typeof(NativeCore).GetMethod(name, Static, null, parameters, null) ?? throw new Exception($"Missing private member {name}");
    private static object? Invoke(MethodInfo method, params object?[] arguments)
    {
        try { return method.Invoke(null, arguments); }
        catch (TargetInvocationException failure) { throw failure.InnerException ?? failure; }
    }
    private static void Invalid(MethodInfo method, params object?[] arguments) => Expect("invalid_response", () => Invoke(method, arguments));
    private static JsonElement Json(string text) { using var document = JsonDocument.Parse(text); return document.RootElement.Clone(); }
    private static string Key(InstalledApplication row) => $"{row.SourceId}\u0000{row.Handler}\u0000{row.Kind}\u0000{row.Unavailable}";
    private static object Wire(InstalledApplication row) => new { source_id = row.SourceId, handler = row.Handler, kind = row.Kind, unavailable = row.Unavailable };

    [DllImport("tiez_native_core", EntryPoint = "tiez_core_request", CallingConvention = CallingConvention.Cdecl)]
    private static extern nint RawRequest([In] byte[] bytes, nuint length);
    [DllImport("tiez_native_core", EntryPoint = "tiez_core_free", CallingConvention = CallingConvention.Cdecl)]
    private static extern void RawFree(nint value);
    private static JsonElement Raw(object command)
    {
        byte[] bytes = JsonSerializer.SerializeToUtf8Bytes(command);
        nint response = RawRequest(bytes, (nuint)bytes.Length);
        try { using var document = JsonDocument.Parse(Marshal.PtrToStringUTF8(response)!); return document.RootElement.Clone(); }
        finally { RawFree(response); }
    }
    // Native icon job IDs start at one per open runtime, so "icons_job" for job 1 proves that no
    // accepted job exists at all.
    private static string PollCode(ulong handle, ulong job)
    {
        var root = Raw(new { op = "apps_icon_poll", handle, job_id = job });
        return root.GetProperty("ok").GetBoolean() ? root.GetProperty("result").GetProperty("state").GetString()! : root.GetProperty("error").GetProperty("code").GetString()!;
    }
    private static ulong RawIconStart(ulong handle, ulong catalogJobId, InstalledApplication row, uint edge)
    {
        var root = Raw(new { op = "apps_icon_start", handle, catalog_job_id = catalogJobId, identity = Wire(row), edge });
        if (!root.GetProperty("ok").GetBoolean()) throw new Exception($"Raw icon start failed: {root.GetProperty("error").GetProperty("code").GetString()}");
        return root.GetProperty("result").GetProperty("job_id").GetUInt64();
    }

    // Controlled parsing vectors: every field of a native response is revalidated against the
    // request before a byte can be adopted.
    private static void Parsers()
    {
        var identity = new ApplicationIconIdentity("Opaque.Editor", "editor.exe", "desktop", null);
        var jobId = Method("IconJobId", typeof(JsonElement));
        var start = Method("ValidateIconStart", typeof(JsonElement), typeof(ulong), typeof(ulong), typeof(uint));
        var poll = Method("ParseIconPoll", typeof(JsonElement), typeof(ulong), typeof(ulong), typeof(uint), typeof(ApplicationIconIdentity));
        string running = "{\"job_id\":5,\"catalog_job_id\":7,\"edge\":32,\"state\":\"running\"}";
        ulong job = (ulong)Invoke(jobId, Json(running))!;
        Check(job == 5, "Icon job ID read");
        Invoke(start, Json(running), job, 7UL, 32U);
        Invalid(start, Json("{\"job_id\":0,\"catalog_job_id\":7,\"edge\":32,\"state\":\"running\"}"), 0UL, 7UL, 32U);
        Invalid(start, Json("{\"catalog_job_id\":7,\"edge\":32,\"state\":\"running\"}"), 0UL, 7UL, 32U);
        Invalid(start, Json("{\"job_id\":\"5\",\"catalog_job_id\":7,\"edge\":32,\"state\":\"running\"}"), 0UL, 7UL, 32U);
        Invalid(start, Json("{\"job_id\":5,\"catalog_job_id\":8,\"edge\":32,\"state\":\"running\"}"), 5UL, 7UL, 32U);
        Invalid(start, Json("{\"job_id\":5,\"catalog_job_id\":7,\"edge\":16,\"state\":\"running\"}"), 5UL, 7UL, 32U);
        Invalid(start, Json("{\"job_id\":5,\"catalog_job_id\":7,\"edge\":32,\"state\":\"ready\"}"), 5UL, 7UL, 32U);
        Invalid(start, Json("{\"job_id\":5,\"catalog_job_id\":7,\"edge\":32,\"state\":7}"), 5UL, 7UL, 32U);
        Invalid(start, Json("{\"job_id\":5,\"catalog_job_id\":7,\"edge\":32}"), 5UL, 7UL, 32U);

        Check(Invoke(poll, Json("{\"job_id\":1,\"catalog_job_id\":7,\"edge\":32,\"state\":\"running\"}"), 1UL, 7UL, 32U, identity) is null,
            "Running poll was not reported as running");
        string ready = "{\"job_id\":1,\"catalog_job_id\":7,\"edge\":32,\"state\":\"ready\","
            + "\"identity\":{\"source_id\":\"Opaque.Editor\",\"handler\":\"editor.exe\",\"kind\":\"desktop\",\"unavailable\":null},"
            + "\"width\":2,\"height\":1,\"pixels\":[1,2,3,4,5,6,7,255]}";
        var raster = (ApplicationIconRaster)Invoke(poll, Json(ready), 1UL, 7UL, 32U, identity)!;
        Check(raster.Width == 2 && raster.Height == 1 && raster.Pixels.Length == 8 && raster.Pixels.Span[7] == 255
            && raster.Identity == identity && raster.CatalogJobId == 7 && raster.RequestedEdge == 32, "Ready poll raster");
        string prefix = "{\"job_id\":1,\"catalog_job_id\":7,\"edge\":32,\"state\":\"ready\",";
        string head = prefix
            + "\"identity\":{\"source_id\":\"Opaque.Editor\",\"handler\":\"editor.exe\",\"kind\":\"desktop\",\"unavailable\":null},";
        Invalid(poll, Json("{\"job_id\":2,\"catalog_job_id\":7,\"edge\":32,\"state\":\"running\"}"), 1UL, 7UL, 32U, identity);
        Invalid(poll, Json("{\"job_id\":1,\"catalog_job_id\":8,\"edge\":32,\"state\":\"running\"}"), 1UL, 7UL, 32U, identity);
        Invalid(poll, Json("{\"job_id\":1,\"catalog_job_id\":7,\"edge\":16,\"state\":\"running\"}"), 1UL, 7UL, 32U, identity);
        Invalid(poll, Json("{\"job_id\":1,\"catalog_job_id\":7,\"edge\":32,\"state\":\"busy\"}"), 1UL, 7UL, 32U, identity);
        Invalid(poll, Json("{\"job_id\":1,\"catalog_job_id\":7,\"edge\":32,\"state\":\"ready\"}"), 1UL, 7UL, 32U, identity);
        Invalid(poll, Json(prefix + "\"width\":1,\"height\":1,\"pixels\":[0,0,0,0]}"), 1UL, 7UL, 32U, identity);
        foreach (string wrong in new[]
        {
            "\"identity\":{\"source_id\":\"Opaque.Other\",\"handler\":\"editor.exe\",\"kind\":\"desktop\",\"unavailable\":null},",
            "\"identity\":{\"source_id\":\"Opaque.Editor\",\"handler\":null,\"kind\":\"desktop\",\"unavailable\":null},",
            "\"identity\":{\"source_id\":\"Opaque.Editor\",\"handler\":\"editor.exe\",\"kind\":\"packaged\",\"unavailable\":null},",
            "\"identity\":{\"source_id\":\"Opaque.Editor\",\"handler\":\"editor.exe\",\"kind\":\"desktop\",\"unavailable\":\"missing_target\"},",
            "\"identity\":{\"source_id\":7,\"handler\":\"editor.exe\",\"kind\":\"desktop\",\"unavailable\":null},",
            "\"identity\":\"Opaque.Editor\",",
            "\"identity\":{\"handler\":\"editor.exe\",\"kind\":\"desktop\"},",
        })
            Invalid(poll, Json(prefix + wrong + "\"width\":1,\"height\":1,\"pixels\":[0,0,0,0]}"), 1UL, 7UL, 32U, identity);
        foreach (string body in new[]
        {
            "\"width\":0,\"height\":1,\"pixels\":[0,0,0,0]",
            "\"width\":1,\"height\":0,\"pixels\":[0,0,0,0]",
            "\"width\":33,\"height\":1,\"pixels\":[" + string.Join(',', new int[132]) + "]",
            "\"width\":1,\"height\":33,\"pixels\":[" + string.Join(',', new int[132]) + "]",
            "\"width\":\"1\",\"height\":1,\"pixels\":[0,0,0,0]",
            "\"width\":1,\"height\":1",
            "\"width\":2,\"height\":1,\"pixels\":[0,0,0,0]",
            "\"width\":1,\"height\":1,\"pixels\":\"AAAAAA==\"",
            "\"width\":1,\"height\":1,\"pixels\":[0,0,0,256]",
            "\"width\":1,\"height\":1,\"pixels\":[0,0,0,-1]",
            "\"width\":1,\"height\":1,\"pixels\":[0,0,0,1.5]",
            "\"width\":1,\"height\":1,\"pixels\":[0,0,0,\"0\"]",
            "\"width\":1,\"height\":1,\"pixels\":null",
        })
            Invalid(poll, Json(head + body + "}"), 1UL, 7UL, 32U, identity);
        Console.WriteLine($"PASS icon response parsing: {checks} controlled malformed job/catalog/edge/state/identity/dimension/byte-array vectors rejected");
    }

    internal static void Run(string directory)
    {
        checks = 0; notes.Clear(); Directory.CreateDirectory(directory);
        Parsers();
        string database = Path.Combine(directory, "应用图标.db");
        var core = new NativeCore(database);
        try
        {
            ulong handle = Handle(core);
            ulong revision = core.Snapshot.Revision;
            var fabricated = new InstalledApplication("任意名称", "Opaque.ManagedReject", "C:\\owned\\reject.exe", "desktop", null);
            Expect("icons_edge", () => core.GetApplicationIconAsync(1, fabricated, 0));
            Expect("icons_edge", () => core.GetApplicationIconAsync(1, fabricated, 129));
            Expect("icons_catalog", () => core.GetApplicationIconAsync(0, fabricated, 32));
            Expect("icons_identity", () => core.GetApplicationIconAsync(1, fabricated with { Handler = null }, 32));
            Expect("icons_identity", () => core.GetApplicationIconAsync(1, fabricated with { Unavailable = "missing_target" }, 32));
            Expect("icons_identity", () => core.GetApplicationIconAsync(1, fabricated with { Kind = "shortcut" }, 32));
            Expect("icons_identity", () => core.GetApplicationIconAsync(1, fabricated with { Handler = "C:\\bad\u0001.exe" }, 32));
            Expect("icons_unavailable", () => core.GetApplicationIconAsync(1, fabricated with { Kind = "unavailable", Handler = null, Unavailable = "missing_target" }, 32));
            using (var preCancelled = new CancellationTokenSource())
            {
                preCancelled.Cancel();
                Cancelled(() => core.GetApplicationIconAsync(1, fabricated, 32, preCancelled.Token));
            }
            Check(PollCode(handle, 1) == "icons_job", "A locally rejected request created a native icon job");
            bool ownerEnforced = Task.Run(() =>
            {
                try { core.GetApplicationIconAsync(1, fabricated, 32); return false; }
                catch (InvalidOperationException) { return true; }
            }).GetAwaiter().GetResult();
            Check(ownerEnforced, "An icon request was accepted off the owner thread");

            var first = Read(core.GetApplicationCatalogAsync(refresh: true));
            Check(first.JobId > 0 && first.Snapshot.Scanned > 0, "Real installed-application catalog is unavailable");
            var desktop = first.Snapshot.Applications
                .Where(row => row.Kind == "desktop" && row.Handler is not null && row.Unavailable is null).Take(8).ToList();
            Check(desktop.Count > 0, "Real catalog exposed no routable desktop row");
            InstalledApplication? desktopRow = null; ApplicationIconRaster? desktopIcon = null;
            var desktopFailures = new List<string>();
            foreach (var row in desktop)
            {
                try { desktopIcon = Read(core.GetApplicationIconAsync(first.JobId, row, 32)); desktopRow = row; break; }
                catch (CoreException failure) { desktopFailures.Add(failure.Code); }
            }
            Check(desktopRow is not null && desktopIcon is not null, $"No real desktop icon succeeded ({string.Join(',', desktopFailures)})");
            Check(desktopIcon!.CatalogJobId == first.JobId && desktopIcon.RequestedEdge == 32
                && desktopIcon.Width >= 1 && desktopIcon.Height >= 1 && desktopIcon.Width <= 32 && desktopIcon.Height <= 32
                && desktopIcon.Pixels.Length == desktopIcon.Width * desktopIcon.Height * 4 && desktopIcon.Pixels.Length <= 65536,
                "Real desktop raster violates the requested edge or the byte contract");
            Check(desktopIcon.Identity == new ApplicationIconIdentity(desktopRow!.SourceId, desktopRow.Handler, desktopRow.Kind, null),
                "Real desktop raster is not bound to the requested catalog identity");
            Check(desktopIcon.Pixels.Span.IndexOfAnyExcept((byte)0) >= 0, "Real desktop raster is empty");
            foreach (uint edge in new uint[] { 16, 64, 128 })
            {
                var sized = Read(core.GetApplicationIconAsync(first.JobId, desktopRow!, edge));
                Check(sized.RequestedEdge == edge && sized.Width >= 1 && sized.Height >= 1 && sized.Width <= edge && sized.Height <= edge
                    && sized.Pixels.Length == sized.Width * sized.Height * 4, $"Real raster does not honor edge {edge}");
            }
            var small = core.GetApplicationIconAsync(first.JobId, desktopRow!, 24);
            var large = core.GetApplicationIconAsync(first.JobId, desktopRow!, 48);
            Check(Read(small).RequestedEdge == 24 && Read(large).RequestedEdge == 48, "Concurrent icon requests crossed their edges");
            var packaged = first.Snapshot.Applications
                .Where(row => row.Kind == "packaged" && row.Handler is not null && row.Unavailable is null).Take(6).ToList();
            int packagedIcons = 0; var packagedFailures = new List<string>();
            foreach (var row in packaged)
            {
                try
                {
                    var icon = Read(core.GetApplicationIconAsync(first.JobId, row, 32));
                    Check(icon.Width >= 1 && icon.Height >= 1 && icon.Width <= 32 && icon.Height <= 32
                        && icon.Pixels.Length == icon.Width * icon.Height * 4, "Packaged raster violates the requested edge");
                    packagedIcons++;
                }
                catch (CoreException failure) { packagedFailures.Add(failure.Code); }
            }
            if (packaged.Count == 0) notes.Add("skipped packaged icons: this host catalog has no packaged row");
            else if (packagedIcons == 0) notes.Add($"skipped packaged icons: {string.Join(',', packagedFailures.Distinct())}");
            else notes.Add($"packaged icons: {packagedIcons}/{packaged.Count} real rasters");
            var inMain = first.Snapshot.Applications.Select(Key).ToHashSet(StringComparer.Ordinal);
            var recommendedOnly = first.Snapshot.Recommendations.Values.SelectMany(group => group.Applications)
                .Select(item => item.Application)
                .FirstOrDefault(row => row.Kind is "desktop" or "packaged" && row.Handler is not null
                    && row.Unavailable is null && !inMain.Contains(Key(row)));
            if (recommendedOnly is null) notes.Add("skipped recommendation-only icon: this host catalog has no recommendation-only row");
            else
            {
                try
                {
                    var icon = Read(core.GetApplicationIconAsync(first.JobId, recommendedOnly, 32));
                    Check(icon.Identity.SourceId == recommendedOnly.SourceId, "Recommendation-only identity was not preserved");
                    notes.Add("recommendation-only row accepted");
                }
                catch (CoreException failure) when (failure.Code is "icons_windows" or "icons_unavailable")
                {
                    notes.Add($"skipped recommendation-only icon: {failure.Code}");
                }
            }
            var absent = new InstalledApplication("伪造名称", "Opaque.NotInCatalog", "C:\\owned\\missing\\fake.exe", "desktop", null);
            Expect("icons_identity", () => Read(core.GetApplicationIconAsync(first.JobId, absent, 32)));
            var cached = Read(core.GetApplicationCatalogAsync());
            Check(cached.Cached && cached.JobId > first.JobId, "Cached catalog read did not advance the generation");
            Expect("icons_catalog", () => Read(core.GetApplicationIconAsync(first.JobId, desktopRow!, 32)));

            var requestGate = Gate(core, "requestGate");
            requestGate.Wait();
            try
            {
                using var abandoned = new CancellationTokenSource();
                var admitted = new List<Task<ApplicationIconRaster>>();
                for (int index = 0; index < 10; index++)
                    admitted.Add(core.GetApplicationIconAsync(cached.JobId, desktopRow!, 32, abandoned.Token));
                Check(admitted.All(task => !task.IsCompleted) && Counter(core, "admittedIcons") == 10, "Admission did not bound queued requests");
                Expect("icons_busy", () => core.GetApplicationIconAsync(cached.JobId, desktopRow!, 32));
                abandoned.Cancel();
                foreach (var task in admitted) Cancelled(task);
                Check(Counter(core, "admittedIcons") == 0 && Counter(core, "trackedIcons") == 0, "Cancelled requests retained admission counts");
            }
            finally { requestGate.Release(); }

            var rawOne = RawIconStart(handle, cached.JobId, desktopRow!, 16);
            var rawTwo = RawIconStart(handle, cached.JobId, desktopRow!, 16);
            int[] pulse = new int[1];
            using (var stop = new CancellationTokenSource())
            {
                var pumper = Task.Run(async () => { while (!stop.IsCancellationRequested) { Interlocked.Increment(ref pulse[0]); await Task.Delay(5); } });
                var watch = Stopwatch.StartNew();
                Expect("icons_busy", () => Read(core.GetApplicationIconAsync(cached.JobId, desktopRow!, 32)));
                watch.Stop(); stop.Cancel(); Await(pumper);
                Check(watch.Elapsed >= TimeSpan.FromMilliseconds(100), "Backend busy retry did not yield");
                Check(Volatile.Read(ref pulse[0]) > 0, "Backend busy retry blocked other managed work");
            }
            foreach (ulong raw in new[] { rawOne, rawTwo })
            {
                Raw(new { op = "apps_icon_cancel", handle, job_id = raw });
                Raw(new { op = "apps_icon_poll", handle, job_id = raw });
            }
            Check(core.Refresh().Revision == revision && Read(core.GetHistoryAsync(new())).Items.Count == 0,
                "Icon requests changed settings or history");
            Check(Read(core.GetCaptureStatusAsync()).Starts == 0 && Read(core.GetTargetStatusAsync()).Starts == 0,
                "Icon requests started capture or target services");
            bool webView = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
                .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
                .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
            Check(!webView, "Icon client loaded WebView2");

            // Async drain: an admitted request that never reached the native service is cancelled by
            // shutdown, and the handle is released only after every admitted request settled.
            var workGate = Gate(core, "iconWorkGate");
            workGate.Wait(); workGate.Wait();
            try
            {
                var queued = core.GetApplicationIconAsync(cached.JobId, desktopRow!, 32);
                Check(!queued.IsCompleted, "Queued icon request did not wait for a concurrency slot");
                var closing = core.DisposeAsync().AsTask();
                Check(ReferenceEquals(closing, core.DisposeAsync().AsTask()), "Duplicate async close did not borrow its task");
                try { core.GetApplicationIconAsync(cached.JobId, desktopRow!, 32); throw new Exception("Closing client accepted icon work"); }
                catch (ObjectDisposedException) { checks++; }
                Cancelled(queued);
                Await(closing);
                Check(Handle(core) == 0 && Counter(core, "admittedIcons") == 0 && Counter(core, "trackedIcons") == 0,
                    "Async close released the handle before admitted icon requests settled");
                Check(PollCode(handle, 1) == "invalid_handle", "Async close retained the native handle");
            }
            finally { workGate.Release(); workGate.Release(); }
            Check(workGate.CurrentCount == 2, "Async close lost icon concurrency slots while waiters unwound");
        }
        finally { core.Dispose(); }

        SyncDispose(Path.Combine(directory, "同步释放.db"));
        Console.WriteLine($"PASS application icons real DLL: {checks} checks; real desktop/packaged raster contract, edges, concurrency, stale generation, membership, admission cap, busy yield and sync/async drain");
        if (notes.Count > 0) Console.WriteLine("notes: " + string.Join("; ", notes));
    }

    private static void SyncDispose(string database)
    {
        var core = new NativeCore(database);
        ulong handle = Handle(core);
        var queuedRow = new InstalledApplication("同步", "Opaque.SyncDispose", "C:\\owned\\sync.exe", "desktop", null);
        var workGate = Gate(core, "iconWorkGate");
        workGate.Wait(); workGate.Wait();
        try
        {
            using (var cancel = new CancellationTokenSource())
            {
                var queued = core.GetApplicationIconAsync(1, queuedRow, 32, cancel.Token);
                cancel.Cancel();
                Cancelled(queued);
            }
            Check(PollCode(handle, 1) == "icons_job", "A cancelled queued request started a native icon job");
            var pending = core.GetApplicationIconAsync(1, queuedRow, 32);
            Check(!pending.IsCompleted, "Queued icon request did not wait for a concurrency slot");
            core.Dispose();
            Check(Handle(core) == 0 && Counter(core, "admittedIcons") == 0 && Counter(core, "trackedIcons") == 0,
                "Synchronous close released the handle before admitted icon requests settled");
            Cancelled(pending);
            Check(PollCode(handle, 1) == "invalid_handle", "Synchronous close retained the native handle");
        }
        finally { workGate.Release(); workGate.Release(); }
        Check(workGate.CurrentCount == 2, "Synchronous close lost icon concurrency slots");
        using var reopened = new NativeCore(database);
        Check(reopened.Snapshot.Revision == 0, "Icon teardown saved settings");
    }
}
