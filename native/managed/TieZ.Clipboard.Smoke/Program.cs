using System.Text;
using TieZ.Core;

static T Read<T>(Task<T> task) => task.GetAwaiter().GetResult();
static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
static void Expect(string code, Action action)
{
    try { action(); }
    catch (CoreException error) when (error.Code == code) { return; }
    throw new InvalidOperationException($"Expected {code}");
}
static CaptureStatus Wait(NativeCore core, Func<CaptureStatus, bool> predicate)
{
    var until = DateTime.UtcNow.AddSeconds(8);
    while (DateTime.UtcNow < until)
    {
        var status = Read(core.GetCaptureStatusAsync());
        if (predicate(status)) return status;
        if (status.LastError is not null) throw new InvalidOperationException($"Capture service: {status.LastError}");
        Thread.Sleep(20);
    }
    throw new InvalidOperationException("Isolated capture did not settle.");
}
string directory = Path.Combine(Path.GetTempPath(), $"TieZ-clipboard-roundtrip-{Guid.NewGuid():N}");
Directory.CreateDirectory(directory);
try
{
    using var isolation = new IsolatedDesktop(); // Must precede all clipboard/native-core activity.
    using var fixture = new ClipboardFixture();
    using var core = new NativeCore(Path.Combine(directory, "fixture.db"));
    core.Update(new Dictionary<string, string> { ["app.capture_rich_text"] = "true", ["app.capture_files"] = "true" }, core.Snapshot.Revision);
    fixture.Text("must not import startup clipboard");
    Check(Read(core.StartCaptureAsync()).Running, "Start isolated native listener");
    Thread.Sleep(250);
    Check(Read(core.GetHistoryAsync(new())).Items.Count == 0, "Startup clipboard was imported");
    string body = "  隔离原生正文\r\n\t ";
    fixture.Text(body);
    Wait(core, status => status.Accepted == 1);
    var first = Read(core.GetHistoryAsync(new())).Items.Single();
    Check(first.Id < 0 && Read(core.GetEntryAsync(first.Id)).Content == body.Replace("\r\n", "\n"), "Real text capture/session/whitespace");
    ulong copyRevision = Read(core.GetHistoryAsync(new())).Revision;
    Read(core.CopyEntryAsync(first.Id, expectedRevision: copyRevision));
    Expect("history_conflict", () => Read(core.CopyEntryAsync(first.Id, expectedRevision: copyRevision - 1)));
    Check(fixture.ReadText() == body.Replace("\r\n", "\n"), "Rejected stale copy changed the OS clipboard");
    Check(fixture.ReadText() == body.Replace("\r\n", "\n"), "Real Unicode copy");
    Thread.Sleep(250);
    Check(Read(core.GetCaptureStatusAsync()).Accepted == 1, "Own copy was captured");
    Thread.Sleep(2100);
    fixture.Text(body);
    Wait(core, status => status.Accepted == 2);
    Check(Read(core.GetHistoryAsync(new())).Items.Single().Id == first.Id, "Intentional identical copy lost stable session ID");

    byte[] rtf = Encoding.ASCII.GetBytes("{\\rtf1 isolated fixture}\0");
    byte[] biff = [9, 0, 255, 7, 0];
    uint htmlFormat = ClipboardFixture.Format("HTML Format"), rtfFormat = ClipboardFixture.Format("Rich Text Format"), biffFormat = ClipboardFixture.Format("Biff8");
    fixture.Write((13, Encoding.Unicode.GetBytes("rich 中文\0")), (htmlFormat, Encoding.UTF8.GetBytes("<html><body><!--StartFragment--><b>rich 中文</b><!--EndFragment--></body></html>\0")), (rtfFormat, rtf), (biffFormat, biff));
    Wait(core, status => status.Accepted == 3);
    var rich = Read(core.GetHistoryAsync(new(ContentType: "rich_text"))).Items.Single();
    Read(core.CopyEntryAsync(rich.Id, expectedRevision: Read(core.GetHistoryAsync(new())).Revision));
    Check(fixture.ReadText() == "rich 中文", "Rich Unicode fallback");
    Check(Encoding.UTF8.GetString(fixture.Read(htmlFormat) ?? []).Contains("<b>rich 中文</b>"), "CF_HTML copy");
    Check((fixture.Read(rtfFormat) ?? []).SequenceEqual(rtf), "RTF payload preservation");
    Check((fixture.Read(biffFormat) ?? []).SequenceEqual(biff), "Office binary payload preservation");
    Read(core.CopyEntryAsync(rich.Id, plainText: true));
    Check(fixture.ReadText() == "rich 中文" && fixture.Read(htmlFormat) is null && fixture.Read(rtfFormat) is null, "Plain-only rich copy");

    var paths = new[] { Path.Combine(directory, "文件%20 one.txt"), Path.Combine(directory, "第二项.txt") };
    foreach (string path in paths) File.WriteAllText(path, "fixture");
    ulong beforeFiles = Read(core.GetCaptureStatusAsync()).Accepted;
    fixture.Write((15, ClipboardFixture.DropFiles(paths)));
    Wait(core, status => status.Accepted == beforeFiles + 1);
    var files = Read(core.GetHistoryAsync(new(ContentType: "file"))).Items.Single();
    Read(core.CopyEntryAsync(files.Id));
    Check(fixture.ReadFiles().SequenceEqual(paths), "File path copy/Unicode/percent preservation");

    ulong beforeImage = Read(core.GetCaptureStatusAsync()).Accepted;
    fixture.Write((8, ClipboardFixture.Dib()));
    Wait(core, status => status.Accepted == beforeImage + 1);
    var image = Read(core.GetHistoryAsync(new(ContentType: "image"))).Items.Single();
    Read(core.CopyEntryAsync(image.Id));
    Check(fixture.Read(8) is { Length: > 40 }, "Image DIB copy");
    Check((fixture.Read(ClipboardFixture.Format("PNG")) ?? []).AsSpan().StartsWith(new byte[] { 137, 80, 78, 71 }), "PNG compatibility format");

    ulong beforePrivate = Read(core.GetCaptureStatusAsync()).Accepted;
    fixture.Text("fixture@example.com");
    Wait(core, status => status.Accepted == beforePrivate + 1);
    var sensitive = Read(core.GetHistoryAsync(new(Search: "fixture@example.com"))).Items.Single();
    Check(sensitive.IsSensitive, "Native privacy stage");
    Expect("sensitive_confirmation", () => Read(core.CopyEntryAsync(sensitive.Id)));
    Expect("history_conflict", () => Read(core.CopyEntryAsync(sensitive.Id, allowSensitive: true, expectedRevision: copyRevision)));
    Read(core.CopyEntryAsync(sensitive.Id, allowSensitive: true, expectedRevision: Read(core.GetHistoryAsync(new())).Revision));
    Check(fixture.ReadText() == "fixture@example.com", "Explicit sensitive copy");

    core.Update(new Dictionary<string, string> { ["app.persistent"] = "true" }, core.Snapshot.Revision);
    fixture.Text(body);
    Wait(core, status => status.Accepted == beforePrivate + 2);
    var promoted = Read(core.GetHistoryAsync(new(Search: "隔离原生正文"))).Items.Single();
    Check(promoted.Id > 0, "Real capture persistence promotion");
    Expect("not_found", () => Read(core.GetEntryAsync(first.Id)));
    Check(!Read(core.StopCaptureAsync()).Running, "Stop isolated native service");
    Console.WriteLine("PASS: private Windows window station; real text/rich/RTF/Office/file/image capture and copy; whitespace, privacy, echo, session promotion and teardown");
}
catch (ClipboardIsolationUnavailable error)
{
    Console.WriteLine($"UNAVAILABLE: private Windows clipboard station (Win32 {error.NativeError}); no clipboard formats were read or written.");
    Environment.ExitCode = 77;
}
finally { Directory.Delete(directory, recursive: true); }
