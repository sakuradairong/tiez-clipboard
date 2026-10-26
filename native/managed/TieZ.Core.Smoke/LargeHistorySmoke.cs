using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using TieZ.Core;

internal static class LargeHistorySmoke
{
    private static int checks;
    private static readonly TimeSpan Bound = TimeSpan.FromSeconds(30);
    private const BindingFlags Hidden = BindingFlags.Instance | BindingFlags.Static | BindingFlags.NonPublic;
    private static void Check(bool value, string message) { Interlocked.Increment(ref checks); if (!value) throw new InvalidOperationException(message); }
    private static T Read<T>(Task<T> task) => task.WaitAsync(Bound).GetAwaiter().GetResult();
    private static void Wait(Task task) => task.WaitAsync(Bound).GetAwaiter().GetResult();
    private static void Expect(string code, Action action)
    {
        try { action(); } catch (CoreException failure) when (failure.Code == code) { checks++; return; }
        throw new InvalidOperationException($"Expected {code}");
    }
    private static void Cancelled(Task task)
    {
        try { Wait(task); } catch (OperationCanceledException) { checks++; return; }
        throw new InvalidOperationException("Queued/staged history work was not cancelled");
    }
    private static SemaphoreSlim Gate(NativeCore core, string name) => (SemaphoreSlim)typeof(NativeCore).GetField(name, Hidden)!.GetValue(core)!;
    private static ulong Handle(NativeCore core) => (ulong)typeof(NativeCore).GetField("handle", Hidden)!.GetValue(core)!;
    private static int Tracked(NativeCore core) => (int)typeof(NativeCore).GetField("trackedLargeHistory", Hidden)!.GetValue(core)!;
    private static void Observe(NativeCore core, Func<string, JsonElement, Task>? observer) => typeof(NativeCore).GetField("largeHistoryObserved", Hidden)!.SetValue(core, observer);
    private static string Body => string.Concat(Enumerable.Repeat(string.Concat("  ", (char)13, (char)10, "中文😀", (char)9, " "), 12000));
    [DllImport("tiez_native_core", EntryPoint = "tiez_core_request", CallingConvention = CallingConvention.Cdecl)]
    private static extern nint Request([In] byte[] bytes, nuint length);
    [DllImport("tiez_native_core", EntryPoint = "tiez_core_free", CallingConvention = CallingConvention.Cdecl)]
    private static extern void Free(nint response);
    private static JsonElement Raw(object request)
    {
        byte[] bytes = JsonSerializer.SerializeToUtf8Bytes(request); nint response = Request(bytes, (nuint)bytes.Length);
        try { using var document = JsonDocument.Parse(Marshal.PtrToStringUTF8(response)!); return document.RootElement.Clone(); }
        finally { Free(response); }
    }
    private static JsonElement Json(string json) { using var document = JsonDocument.Parse(json); return document.RootElement.Clone(); }
    private static object? Invoke(string name, params object?[] arguments)
    {
        try { return typeof(NativeCore).GetMethod(name, Hidden)!.Invoke(null, arguments); }
        catch (TargetInvocationException error) { throw error.InnerException ?? error; }
    }
    private static void Invalid(string method, params object?[] arguments) => Expect("invalid_response", () => Invoke(method, arguments));

    private static void RoundTrip(string database)
    {
        using var core = new NativeCore(database);
        int settingsEvents = 0; core.Changed += _ => settingsEvents++;
        string content = Body, html = "<table><tr><td>原样😀</td></tr></table>" + new string(' ', 70000);
        string[] tags = ["large", "保留"];
        var saving = core.AddManualAsync(content, "rich_text", html, tags); tags[0] = "mutated caller";
        var added = Read(saving);
        var entry = Read(core.GetEntryAsync(added.Id));
        Check(added.Changed && added.Id > 0 && entry.Content == content && entry.HtmlContent == html, "Large rich add changed source bytes");
        Check(entry.Tags.SequenceEqual(new[] { "large", "保留" }), "Large request did not snapshot caller tags before yielding");
        string edited = content + "  changed😀  ";
        var changed = Read(core.EditEntryAsync(added.Id, edited));
        var plain = Read(core.GetEntryAsync(added.Id));
        Check(changed.Changed && plain.Content == edited && plain.ContentType == "text" && plain.HtmlContent is null, "Large edit did not retain legacy rich-to-plain behavior");
        var unchanged = Read(core.EditEntryAsync(added.Id, edited));
        Check(!unchanged.Changed && unchanged.Revision == changed.Revision, "Large no-op edit advanced history");
        var privateEntry = Read(core.AddManualAsync(content, tags: ["password"]));
        Read(core.EditEntryAsync(privateEntry.Id, edited));
        Check(Read(core.GetEntryAsync(privateEntry.Id)).Content == edited, "Explicit large private edit did not round-trip");
        Expect("transfer_bounds", () => Read(core.AddManualAsync(new string((char)1, 3_000_000))));
        Check(Read(core.GetHistoryAsync(new())).StoredCount == 2, "Oversized escaped JSON partially saved history");
        Check(core.Refresh().Revision == 0 && settingsEvents == 0 && Tracked(core) == 0, "Large history changed settings or retained requests");
    }
    private delegate int ChunkBoundary(ReadOnlySpan<byte> bytes, int offset, int maximum);
    private static void Parsers()
    {
        const string token = "0123456789abcdef0123456789abcdef";
        foreach (var shape in new[] { default(JsonElement), Json("[]"), Json("null"), Json("true"), Json("17"), JsonSerializer.SerializeToElement("scalar") })
        {
            Invalid("ParseHistoryUploadLimits", shape);
            Invalid("ValidateHistoryUploadStatus", shape, token, 32, 0, 7UL);
            Invalid("ReadLargeHistoryReceipt", shape, null, 7UL);
            Check(Invoke("RecoverHistoryUploadToken", shape) is null, "Non-object response fabricated a resource token");
        }
        var fields = new Dictionary<string, object?> { ["version"] = 1, ["history_revision"] = 7UL,
            ["max_payload_bytes"] = 16777216, ["max_reserved_bytes"] = 33554432, ["max_chunk_bytes"] = 4096,
            ["max_uploads"] = 2, ["idle_seconds"] = 120 };
        Check(Invoke("ParseHistoryUploadLimits", JsonSerializer.SerializeToElement(fields)) is not null, "Valid native limits rejected");
        foreach (string key in fields.Keys)
        {
            var malformed = new Dictionary<string, object?>(fields); malformed.Remove(key);
            Invalid("ParseHistoryUploadLimits", JsonSerializer.SerializeToElement(malformed));
            malformed[key] = "wrong type";
            Invalid("ParseHistoryUploadLimits", JsonSerializer.SerializeToElement(malformed));
        }
        foreach (var (key, value) in new (string, object)[] { ("max_payload_bytes", 16777217), ("max_reserved_bytes", 0),
            ("max_chunk_bytes", 3), ("max_chunk_bytes", 4097), ("max_uploads", 3), ("idle_seconds", 0), ("history_revision", -1) })
        {
            var malformed = new Dictionary<string, object?>(fields) { [key] = value };
            Invalid("ParseHistoryUploadLimits", JsonSerializer.SerializeToElement(malformed));
        }
        var future = new Dictionary<string, object?>(fields) { ["version"] = 2 };
        Expect("transfer_version", () => Invoke("ParseHistoryUploadLimits", JsonSerializer.SerializeToElement(future)));
        var status = new Dictionary<string, object?> { ["token"] = token, ["total_bytes"] = 32, ["received_bytes"] = 16, ["expected_revision"] = 7UL };
        Invoke("ValidateHistoryUploadStatus", JsonSerializer.SerializeToElement(status), token, 32, 16, 7UL);
        foreach (var (key, value) in new (string, object?)[] { ("token", "wrong"), ("token", new string('f', 32)),
            ("total_bytes", 31), ("received_bytes", 15), ("expected_revision", 8UL), ("received_bytes", null) })
        {
            var malformed = new Dictionary<string, object?>(status) { [key] = value };
            Invalid("ValidateHistoryUploadStatus", JsonSerializer.SerializeToElement(malformed), token, 32, 16, 7UL);
        }
        var receipt = new Dictionary<string, object?> { ["revision"] = 8UL, ["changed"] = true, ["id"] = 5L,
            ["replaced_id"] = null, ["removed_ids"] = Array.Empty<long>(), ["persistent_sync_pending"] = true };
        Check(((HistoryMutation)Invoke("ReadLargeHistoryReceipt", JsonSerializer.SerializeToElement(receipt), null, 7UL)!).Id == 5, "Valid native receipt rejected");
        foreach (var (key, value) in new (string, object?)[] { ("revision", 7UL), ("revision", 9UL), ("changed", "true"),
            ("id", 0L), ("replaced_id", 1L), ("removed_ids", new long[] { 1 }), ("persistent_sync_pending", null) })
        {
            var malformed = new Dictionary<string, object?>(receipt) { [key] = value };
            Invalid("ReadLargeHistoryReceipt", JsonSerializer.SerializeToElement(malformed), null, 7UL);
        }
        Invalid("ReadLargeHistoryReceipt", JsonSerializer.SerializeToElement(receipt), 6L, 7UL);
        var chunk = typeof(NativeCore).GetMethod("HistoryChunkEnd", Hidden)!.CreateDelegate<ChunkBoundary>();
        byte[] utf8 = Encoding.UTF8.GetBytes(new string('x', 4095) + "😀中文");
        int end = chunk(utf8, 0, 4096);
        Check(end == 4095 && chunk(utf8, end, 4096) == utf8.Length, "UTF-8 chunking split a multibyte code point");
        var payloadType = typeof(NativeCore).GetNestedType("LargeHistoryPayload", BindingFlags.NonPublic)!;
        object payload = Activator.CreateInstance(payloadType, Hidden | BindingFlags.Public, null,
            [new string('x', 511) + "😀" + (char)13 + (char)10 + "中", "text", null, Array.Empty<string>()], null)!;
        var buffer = (MemoryStream)Invoke("SerializeHistoryPayload", payload, 4096, CancellationToken.None)!;
        using (var parsed = JsonDocument.Parse(buffer.ToArray())) Check(parsed.RootElement.GetProperty("content").GetString()!.Contains("😀"), "Segmented JSON lost a surrogate pair");
        byte[] allocated = buffer.GetBuffer(); buffer.Dispose();
        Check(allocated.All(value => value == 0), "Disposed serialized staging buffer was not zeroized");
        Expect("transfer_bounds", () => Invoke("SerializeHistoryPayload", payload, 16, CancellationToken.None));
    }
    private static void QueueAndCancellation(string database)
    {
        using var core = new NativeCore(database);
        string body = Body; int began = 0, owner = Environment.CurrentManagedThreadId;
        Observe(core, (op, _) => { Check(Environment.CurrentManagedThreadId != owner, "Transfer work captured its owner thread"); if (op == "history_upload_begin") Interlocked.Increment(ref began); return Task.CompletedTask; });
        var work = Gate(core, "largeHistoryGate"); work.Wait();
        using var cancelled = new CancellationTokenSource();
        var queued = new List<Task<HistoryMutation>>();
        try
        {
            for (int i = 0; i < 4; i++) queued.Add(core.AddManualAsync(body, cancellationToken: cancelled.Token));
            Check(Tracked(core) == 4, "Large transfer admission was not counted before yielding");
            Expect("transfer_capacity", () => core.AddManualAsync(body));
            cancelled.Cancel(); foreach (var task in queued) Cancelled(task);
            Check(Tracked(core) == 0 && began == 0, "Queued cancellation created a native resource or lost its admission");
        }
        finally { work.Release(); }
        using var pre = new CancellationTokenSource(); pre.Cancel();
        try { core.AddManualAsync(body, cancellationToken: pre.Token); throw new Exception("Pre-cancelled large request admitted"); }
        catch (OperationCanceledException) { checks++; }
        bool ownerRejected = Task.Run(() =>
        {
            try { core.AddManualAsync(body); return false; }
            catch (InvalidOperationException) { return true; }
        }).GetAwaiter().GetResult();
        Check(ownerRejected && Tracked(core) == 0, "Foreign thread admitted large work");
        Observe(core, null);
    }
    private static void CancelAcceptedBeginAndPreserveAcceptedCommit(string database)
    {
        using var core = new NativeCore(database); string body = Body;
        var entered = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        using var cancellation = new CancellationTokenSource();
        Observe(core, (op, result) =>
        {
            if (op != "history_upload_begin") return Task.CompletedTask;
            entered.TrySetResult(result); return release.Task;
        });
        var request = core.AddManualAsync(body, cancellationToken: cancellation.Token);
        string? token = null;
        try
        {
            var resource = Read(entered.Task); token = resource.GetProperty("token").GetString();
            Check(Raw(new { op = "history_upload_status", handle = Handle(core), token }).GetProperty("result").GetProperty("received_bytes").GetInt32() == 0,
                "Begin did not retain a real empty native resource");
            var watch = System.Diagnostics.Stopwatch.StartNew();
            core.Update(new Dictionary<string, string> { ["app.color_mode"] = "dark" }, core.Snapshot.Revision);
            Check(watch.Elapsed < TimeSpan.FromSeconds(2), "Staged upload held the shared request gate while paused");
            cancellation.Cancel();
        }
        finally { release.TrySetResult(); }
        Cancelled(request);
        Check(Raw(new { op = "history_upload_status", handle = Handle(core), token }).GetProperty("error").GetProperty("code").GetString() == "transfer_missing",
            "Cancellation after native begin leaked its accepted token");
        Check(Read(core.GetHistoryAsync(new())).StoredCount == 0 && Tracked(core) == 0, "Cancelled draft partially saved history");
        using var race = new CancellationTokenSource();
        Observe(core, (op, _) => { if (op == "history_upload_commit") race.Cancel(); return Task.CompletedTask; });
        var accepted = Read(core.AddManualAsync(body, cancellationToken: race.Token));
        Check(race.IsCancellationRequested && accepted.Changed && Read(core.GetEntryAsync(accepted.Id)).Content == body,
            "Cancellation after native commit hid its actual accepted receipt");
        Check(Tracked(core) == 0, "Accepted commit retained managed work");
        Observe(core, null);
    }
    private static void Drain(string database, bool asynchronous)
    {
        var core = new NativeCore(database); string body = Body;
        var entered = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        Observe(core, (op, result) => { if (op != "history_upload_begin") return Task.CompletedTask; entered.TrySetResult(result); return release.Task; });
        var pending = core.AddManualAsync(body);
        var shared = Gate(core, "requestGate"); bool held = false;
        try
        {
            string token = Read(entered.Task).GetProperty("token").GetString()!;
            if (asynchronous)
            {
                shared.Wait(); held = true;
                var watch = System.Diagnostics.Stopwatch.StartNew();
                Task closing = core.DisposeAsync().AsTask();
                Check(watch.Elapsed < TimeSpan.FromSeconds(1) && !closing.IsCompleted, "Async close blocked its owner or skipped accepted work");
                Check(ReferenceEquals(closing, core.DisposeAsync().AsTask()), "Duplicate close did not borrow its pending task");
                Check(core.IsClosing && Tracked(core) == 1, "Shutdown lost an admitted native resource before cleanup");
                try { core.AddManualAsync(body); throw new Exception("Closing client admitted work"); }
                catch (ObjectDisposedException) { checks++; }
                release.TrySetResult();
                Check(!closing.IsCompleted, "Close freed its handle while resource abort was behind the held request gate");
                shared.Release(); held = false;
                Wait(closing);
            }
            else
            {
                var shutdown = (CancellationTokenSource)typeof(NativeCore).GetField("shutdown", Hidden)!.GetValue(core)!;
                using var registration = shutdown.Token.Register(() => release.TrySetResult());
                core.Dispose(); // The whole process has an external watchdog, including this wait.
            }
            Cancelled(pending);
            Check(Tracked(core) == 0 && Handle(core) == 0 && typeof(NativeCore).GetField("largeHistoryObserved", Hidden)!.GetValue(core) is null,
                "Released client retained staged requests, handles or observer delegates");
            core.Dispose(); Wait(core.DisposeAsync().AsTask());
        }
        finally
        {
            release.TrySetResult(); if (held) shared.Release();
            if (Handle(core) != 0) core.Dispose();
        }
        using var reopened = new NativeCore(database);
        Check(Read(reopened.GetHistoryAsync(new())).StoredCount == 0, "Shutdown committed a partially staged draft");
    }

    private static void RevisionConflict(string database)
    {
        using var core = new NativeCore(database);
        var original = Read(core.AddManualAsync("preserved original", "rich_text", "<b>preserved</b>"));
        var entered = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        Observe(core, (op, result) => { if (op != "history_upload_begin") return Task.CompletedTask; entered.TrySetResult(result); return release.Task; });
        var saving = core.EditEntryAsync(original.Id, Body);
        string? token = null;
        try
        {
            token = Read(entered.Task).GetProperty("token").GetString();
            Read(core.SetPinnedAsync(original.Id, true));
        }
        finally { release.TrySetResult(); }
        Expect("history_conflict", () => Read(saving));
        var entry = Read(core.GetEntryAsync(original.Id));
        Check(entry.Content == "preserved original" && entry.HtmlContent == "<b>preserved</b>" && entry.IsPinned,
            "Conflicted large edit overwrote accepted source or pin state");
        Check(Raw(new { op = "history_upload_status", handle = Handle(core), token }).GetProperty("error").GetProperty("code").GetString() == "transfer_missing",
            "Rejected commit retained its native resource");
        Check(Tracked(core) == 0 && core.Refresh().Revision == 0, "Rejected large edit retained work or changed settings");
        Observe(core, null);
    }
    internal static void Run(string directory)
    {
        Directory.CreateDirectory(directory);
        Parsers(); RoundTrip(Path.Combine(directory, "roundtrip.db"));
        RevisionConflict(Path.Combine(directory, "conflict.db"));
        QueueAndCancellation(Path.Combine(directory, "queue.db"));
        CancelAcceptedBeginAndPreserveAcceptedCommit(Path.Combine(directory, "cancel.db"));
        Drain(Path.Combine(directory, "sync-close.db"), false);
        Drain(Path.Combine(directory, "async-close.db"), true);
        Console.WriteLine($"PASS large history real DLL: {checks} checks; bounded UTF-8/JSON add/edit, original formats, snapshot/no-op/privacy, limits and response validation.");
    }
}
