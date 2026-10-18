using System.Diagnostics;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

internal sealed class HistoryCardCopyWindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private readonly List<CardCopyRequest> requests = [];
    private DispatcherQueueTimer? timer;
    private TaskCompletionSource<ClipboardWriteResult>? held;
    private TaskCompletionSource<bool>? release;
    private CancellationToken heldToken;
    private CardCopyRequest? heldRequest;
    private int phase;
    private bool ticking, actualConflict;
    private long publicId, otherId, privateId, richId, badId, scrolledId;
    private long[] selection = [];
    private ulong historyRevision, settingsRevision;
    private const string Html = "<b>native card rich</b>";
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    internal void Start()
    {
        timer = main.DispatcherQueue.CreateTimer(); timer.Interval = TimeSpan.FromMilliseconds(350); timer.Tick += Tick; timer.Start();
    }
    private Task<ClipboardWriteResult> FakeWrite(CardCopyRequest request, CancellationToken token)
    {
        requests.Add(request); token.ThrowIfCancellationRequested();
        return Task.FromResult(new ClipboardWriteResult(request.Id, 1));
    }
    private void HoldWrite(bool honourCancellation = true)
    {
        held = new(TaskCreationOptions.RunContinuationsAsynchronously);
        main.History.CardCopyWriter = (request, token) =>
        {
            requests.Add(request); heldRequest = request; heldToken = token;
            return honourCancellation ? held.Task.WaitAsync(token) : held.Task;
        };
    }
    private void HoldActualConflict()
    {
        release = new(TaskCreationOptions.RunContinuationsAsynchronously); actualConflict = false;
        main.History.CardCopyWriter = async (request, token) =>
        {
            heldRequest = request; heldToken = token;
            await release.Task.WaitAsync(token);
            try { return await core.CopyEntryAsync(request.Id, request.PlainText, request.AllowSensitive, token, request.Revision); }
            catch (CoreException error) when (error.Code == "history_conflict") { actualConflict = true; throw; }
        };
    }
    private async void Tick(DispatcherQueueTimer sender, object args)
    {
        if (ticking) return; ticking = true;
        try
        {
            Check(elapsed.Elapsed < TimeSpan.FromSeconds(65), $"Timed out in card-copy phase {phase}.");
            var view = main.History;
            if (phase < 38 && (!view.IsLoaded || (view.IsBusy && !view.CardCopyPending)
                || (view.HasDialog && !view.IsDialogReady))) return;
            switch (phase)
            {
                case 0:
                    for (int i = 0; i < 72; i++) await core.AddManualAsync($"  card copy {i:000}\r\n ", tags: ["cards"]);
                    richId = (await core.AddManualAsync("  rich card\r\n ", "rich_text", Html, ["cards"])).Id;
                    badId = (await core.AddManualAsync("data:image/png;base64,bad!", "image", tags: ["cards"])).Id;
                    privateId = (await core.AddManualAsync("private card", tags: ["cards", "password"])).Id;
                    await core.SetPinnedAsync(richId, true); await core.SetPinnedAsync(badId, true);
                    historyRevision = (await core.SetPinnedAsync(privateId, true)).Revision;
                    main.AppWindow.Resize(new SizeInt32(1560, 1000)); await view.ReloadAsync();
                    view.CardCopyWriter = FakeWrite; view.ChooseLayout("grid"); break;
                case 1:
                    publicId = view.Rows.First(row => row.Summary.ContentType == "text" && !row.Summary.IsSensitive).Id;
                    otherId = view.Rows.First(row => row.Summary.ContentType == "text" && !row.Summary.IsSensitive && row.Id != publicId).Id;
                    if (!view.CardCanCopy(publicId) || !view.CardCanCopy(richId)) return;
                    Check(!view.CardCanCopy(privateId) && view.CardCanCopy(publicId, true), "Sensitive card lacks consent or plain-copy control is unavailable.");
                    selection = [publicId, otherId]; view.SelectMany(selection); settingsRevision = core.Snapshot.Revision;
                    view.InvokeCardCopy(richId); break;
                case 2:
                    Check(requests is [{ Id: var id, PlainText: false, AllowSensitive: false, Revision: var revision }]
                        && id == richId && revision == historyRevision && view.SelectedHistoryIds.Order().SequenceEqual(selection.Order())
                        && !view.HasError && view.CopyFeedback.Contains("原始格式"), "Direct rich copy lost identity/version/selection or feedback.");
                    Check((await core.GetEntryAsync(richId)).HtmlContent == Html && (await core.GetHistoryAsync(new())).Revision == historyRevision
                        && core.Snapshot.Revision == settingsRevision, "Copy routing mutated source/settings/history.");
                    view.InvokeCardCopy(publicId, true); break;
                case 3:
                    Check(requests.Count == 2 && requests[^1] is { PlainText: true, AllowSensitive: false } && requests[^1].Id == publicId
                        && view.CopyFeedback.Contains("纯文本") && view.SelectedHistoryIds.Order().SequenceEqual(selection.Order()), "Plain card copy changed selection/format.");
                    view.Select(privateId); break;
                case 4:
                    Check(view.CanReveal && !view.CardCanCopy(privateId) && view.Detail == "", "Selecting a private card granted copy consent.");
                    view.Invoke("reveal"); break;
                case 5:
                    if (!view.CardCanCopy(privateId)) return;
                    Check(view.Detail == "private card", "Sensitive detail reveal did not complete."); view.InvokeCardCopy(privateId); break;
                case 6:
                    Check(requests.Count == 3 && requests[^1] is { AllowSensitive: true, PlainText: false } && requests[^1].Id == privateId,
                        "Explicit sensitive copy lost its consent/ID.");
                    view.SelectMany(privateId, publicId); break;
                case 7:
                    Check(!view.CardCanCopy(privateId) && view.CardCanCopy(publicId), "Multi-selection retained private copy consent.");
                    view.Select(privateId); view.Invoke("tags"); break;
                case 8:
                    Check(!view.CardCanCopy(publicId) && !view.CardCanCopy(privateId), "Dialog left direct copy enabled."); view.CancelDialog(); break;
                case 9:
                    if (!view.CardCanCopy(publicId)) return;
                    core.Update(new Dictionary<string,string> { ["app.compact_mode"] = "true", ["app.color_mode"] = "dark" }, core.Snapshot.Revision); break;
                case 10:
                    if (!view.CardCanCopy(publicId)) return;
                    Check(view.CompactMode && !view.CardCanCopy(privateId) && main.CurrentTheme == Microsoft.UI.Xaml.ElementTheme.Dark,
                        "Compact/theme update lost controls or reused sensitive consent.");
                    view.InvokeCardCopy(publicId); break;
                case 11:
                    Check(requests.Count == 4 && requests[^1].Id == publicId && !requests[^1].PlainText, "Compact card routed the wrong record.");
                    core.Update(new Dictionary<string,string> { ["app.compact_mode"] = "false" }, core.Snapshot.Revision);
                    view.ScrollIntoHistory(view.Rows[40].Id); break;
                case 12:
                    if (!view.Rows.Any(row => row.Id != publicId && view.CardCanCopy(row.Id))) return;
                    scrolledId = view.Rows.First(row => row.Id != publicId && view.CardCanCopy(row.Id)).Id;
                    Check(!view.CardCanCopy(publicId), "Offscreen cached card remained actionable.");
                    view.InvokeCardCopy(scrolledId, true); break;
                case 13:
                    Check(requests.Count == 5 && requests[^1].Id == scrolledId && requests[^1].PlainText,
                        "Recycled card copied its old data context.");
                    view.ScrollIntoHistory(publicId); break;
                case 14:
                    if (!view.CardCanCopy(publicId)) return;
                    view.CardCopyWriter = null; view.InvokeCardCopy(badId); break;
                case 15:
                    Check(!view.CardCopyPending && view.HasError && !view.CopyFeedback.Contains("已复制"), "Invalid raster preparation was reported as copied.");
                    HoldWrite(); view.InvokeCardCopy(publicId); break;
                case 16:
                    if (!view.CardCopyPending) return;
                    Check(!view.CardCanCopy(publicId) && !view.CanChangeLayout, "Pending copy left commands enabled."); main.HideToTray(); break;
                case 17:
                    if (view.CardCopyPending) return;
                    Check(heldToken.IsCancellationRequested && !main.AppWindow.IsVisible && view.CopyFeedback.Contains("取消"), "Tray hide did not cancel queued copy.");
                    held!.TrySetCanceled(); view.CardCopyWriter = FakeWrite; main.ShowMain(); break;
                case 18:
                    if (!view.CardCanCopy(publicId)) return;
                    HoldWrite(); view.InvokeCardCopy(publicId); break;
                case 19:
                    if (!view.CardCopyPending) return;
                    view.Search("no matching card copy"); break;
                case 20:
                    if (view.CardCopyPending || view.Rows.Count != 0) return;
                    Check(heldToken.IsCancellationRequested, "New query did not cancel queued copy."); held!.TrySetCanceled();
                    view.CardCopyWriter = FakeWrite; view.Search(""); await view.ReloadAsync(); break;
                case 21:
                    if (!view.CardCanCopy(publicId)) return;
                    HoldWrite(); view.InvokeCardCopy(publicId); break;
                case 22:
                    if (!view.CardCopyPending) return;
                    main.NavigateTags(); break;
                case 23:
                    if (view.CardCopyPending) return;
                    Check(heldToken.IsCancellationRequested && !view.CardCanCopy(publicId), "Page navigation retained a queued card action.");
                    held!.TrySetCanceled(); view.CardCopyWriter = FakeWrite; main.NavigateHistory(); break;
                case 24:
                    if (!view.CardCanCopy(publicId)) return;
                    HoldWrite(); view.InvokeCardCopy(publicId); break;
                case 25:
                    if (!view.CardCopyPending) return;
                    core.Update(new Dictionary<string,string> { ["app.history_view_mode"] = "list" }, core.Snapshot.Revision); break;
                case 26:
                    if (view.CardCopyPending) return;
                    Check(heldToken.IsCancellationRequested && view.ViewLayout == "list", "External layout change retained queued copy.");
                    held!.TrySetCanceled(); view.CardCopyWriter = FakeWrite; view.ChooseLayout("grid"); break;
                case 27:
                    if (!view.CardCanCopy(publicId)) return;
                    HoldActualConflict(); view.InvokeCardCopy(publicId); break;
                case 28:
                    if (!view.CardCopyPending) return;
                    await core.EditEntryAsync(publicId, "changed during queued copy"); release!.TrySetResult(true); break;
                case 29:
                    if (view.CardCopyPending) return;
                    Check(actualConflict && view.HasError && view.CopyFeedback.Contains("记录已变化"), "Actual DLL accepted a stale card request or lost refresh feedback.");
                    view.CardCopyWriter = FakeWrite; await view.ReloadAsync(); break;
                case 30:
                    if (!view.CardCanCopy(publicId)) return;
                    HoldActualConflict(); view.InvokeCardCopy(publicId); break;
                case 31:
                    if (!view.CardCopyPending) return;
                    await core.SetTagsAsync(publicId, ["cards", "password"]); release!.TrySetResult(true); break;
                case 32:
                    if (view.CardCopyPending) return;
                    Check(actualConflict && view.HasError, "Privacy transition reached a card clipboard write.");
                    view.CardCopyWriter = FakeWrite; await view.ReloadAsync(publicId); break;
                case 33:
                    Check(!view.CardCanCopy(publicId) && view.Detail == "" && view.CanReveal, "Privacy reload silently retained copy consent.");
                    if (!view.CardCanCopy(otherId)) return;
                    HoldWrite(honourCancellation: false); view.InvokeCardCopy(otherId); break;
                case 34:
                    if (!view.CardCopyPending) return;
                    main.HideToTray(); Check(heldToken.IsCancellationRequested, "Hidden accepted copy did not receive cancellation.");
                    held!.TrySetResult(new(heldRequest!.Id, 1)); break;
                case 35:
                    if (view.CardCopyPending) return;
                    Check(!view.HasError && view.CopyFeedback.Contains("已按原始格式"), "Late accepted result was misreported as cancelled.");
                    view.CardCopyWriter = FakeWrite; main.ShowMain(); break;
                case 36:
                    if (!view.CardCanCopy(otherId)) return;
                    HoldWrite(); view.InvokeCardCopy(otherId); break;
                case 37:
                    if (!view.CardCopyPending) return;
                    view.Dispose(); Check(heldToken.IsCancellationRequested, "Dispose did not cancel queued card action."); break;
                case 38:
                    if (view.CardCopyPending) return;
                    Check(!view.HasLayoutResources && !view.CardCanCopy(otherId) && (await core.GetCaptureStatusAsync()).Starts == 0,
                        "Card copies leaked layout resources or started capture."); Finish(true, null); return;
            }
            phase++;
        }
        catch (Exception error) { Finish(false, $"Card-copy phase {phase}: {error}"); }
        finally { ticking = false; }
    }
    private async void Finish(bool passed, string? error)
    {
        timer!.Stop(); timer.Tick -= Tick; held?.TrySetCanceled(); release?.TrySetCanceled();
        main.History.CardCopyWriter = null;
        bool webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
            .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
        try { Check(!webView2ModuleLoaded, "WebView2 loaded in card actions."); if (!await main.Quit()) throw new Exception("Native shutdown did not release the core."); Directory.Delete(directory, recursive: true); }
        catch (Exception failure) { passed = false; error = $"{error}\n{failure}"; }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "history-card-copy", phases = phase + 1,
            controlledRequests = requests.Count, actualConflict, successfulOsClipboardWritesTested = false, webView2ModuleLoaded,
            elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
