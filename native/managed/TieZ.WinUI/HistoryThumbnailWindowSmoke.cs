using System.Diagnostics;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

internal sealed class HistoryThumbnailWindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private sealed record Held(ImagePreviewData Preview, CancellationToken Token, TaskCompletionSource<ImagePreviewData> Completion);
    private readonly List<Held> held = [];
    private readonly HashSet<long> ordinaryIds = [];
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private DispatcherQueueTimer? timer;
    private int phase, visibleImages, peak, initialRealized, pagedRealized;
    private bool ticking;
    private long sensitiveId, brokenId, animationId, changedId;
    private long[] initial = [];
    private long originalId;
    private string originalImage = "", originalAnimation = "";
    private ulong settingsRevision, historyRevision;
    private const string Png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAYAAAC56t6BAAAAEUlEQVR4nGP4z8DQAMIMGAwAfcEI+4thakwAAAAASUVORK5CYII=";
    private const string Gif = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    internal void Start()
    {
        timer = main.DispatcherQueue.CreateTimer(); timer.Interval = TimeSpan.FromMilliseconds(350); timer.Tick += Tick; timer.Start();
    }
    private void HoldReads()
    {
        held.Clear();
        main.History.ThumbnailReader = async (id, revision, token) =>
        {
            var preview = await core.GetImagePreviewAsync(id, revision, maxEdge: 256, cancellationToken: token);
            var completion = new TaskCompletionSource<ImagePreviewData>(TaskCreationOptions.RunContinuationsAsynchronously);
            held.Add(new(preview, token, completion));
            return await completion.Task; // Deliberately cannot abort a response already in native work.
        };
    }
    private void ReleaseReads()
    {
        main.History.ThumbnailReader = null;
        foreach (var item in held) item.Completion.TrySetResult(item.Preview);
    }
    private bool Ready()
    {
        var view = main.History;
        return view.ThumbnailRequests == 0 && view.ThumbnailIds.Length >= 2;
    }
    private async void Tick(DispatcherQueueTimer sender, object args)
    {
        if (ticking) return; ticking = true;
        try
        {
            Check(elapsed.Elapsed < TimeSpan.FromSeconds(65), $"Timed out in thumbnail phase {phase}.");
            var view = main.History;
            if (phase < 36 && (!view.IsLoaded || view.IsBusy)) return;
            Check(view.ThumbnailRequests is >= 0 and <= 2, "Thumbnail native/decode work exceeded two slots.");
            switch (phase)
            {
                case 0:
                    for (int i = 0; i < 72; i++) ordinaryIds.Add((await core.AddManualAsync(Png, "image", tags: ["gallery", $"item-{i}"])).Id);
                    await core.AddManualAsync("native text fallback", tags: ["gallery"]);
                    brokenId = (await core.AddManualAsync("data:image/png;base64,bad!", "image", tags: ["gallery"])).Id;
                    animationId = (await core.AddManualAsync(Gif, "image", tags: ["gallery"])).Id;
                    sensitiveId = (await core.AddManualAsync(Png, "image", tags: ["gallery", "password"])).Id;
                    await core.SetPinnedAsync(brokenId, true); await core.SetPinnedAsync(animationId, true);
                    historyRevision = (await core.SetPinnedAsync(sensitiveId, true)).Revision;
                    originalId = ordinaryIds.Max();
                    originalImage = (await core.GetEntryAsync(originalId)).Content;
                    originalAnimation = (await core.GetEntryAsync(animationId)).Content;
                    main.AppWindow.Resize(new SizeInt32(1560, 1000)); await view.ReloadAsync(); break;
                case 1:
                    Check(view.Rows.Count == 50 && view.ThumbnailResources == 0 && core.Snapshot.Revision == 0, "List default read/wrote thumbnails or settings.");
                    view.ChooseLayout("grid"); break;
                case 2:
                    if (!Ready() || !view.UnavailableThumbnailIds.Contains(brokenId)) return;
                    initial = view.ThumbnailIds; visibleImages = initial.Length;
                    Check(initial.All(view.VisibleThumbnailIds.Contains), $"Offscreen image IDs: {string.Join(',', initial.Except(view.VisibleThumbnailIds))}; visible: {string.Join(',', view.VisibleThumbnailIds)}.");
                    Check(!initial.Contains(sensitiveId), "Sensitive image appeared in the card.");
                    Check(initial.Contains(animationId), $"Animation {animationId} missing; images: {string.Join(',', initial)}; unavailable: {string.Join(',', view.UnavailableThumbnailIds)}.");
                    Check(view.ThumbnailResources < 25, $"Unbounded resources: {view.ThumbnailResources}.");
                    initialRealized = view.RealizedCount; // WinUI overscan may cache the whole first small page in a wide window.
                    Check(initial.Contains(originalId) && (await core.GetEntryAsync(animationId)).Content == originalAnimation
                        && (await core.GetEntryAsync(originalId)).Content == originalImage
                        && (await core.GetHistoryAsync(new())).Revision == historyRevision, "Thumbnail reads changed original formats/history.");
                    view.Select(sensitiveId); view.Invoke("reveal"); break;
                case 3:
                    Check(view.HasImagePreview && view.PreviewedImageId == sensitiveId && !view.ThumbnailIds.Contains(sensitiveId),
                        "Explicit sensitive detail reveal leaked into a card.");
                    core.Update(new Dictionary<string,string> { ["app.compact_mode"] = "true" }, core.Snapshot.Revision); break;
                case 4:
                    if (!Ready()) return;
                    Check(view.CompactMode && !view.ThumbnailIds.Contains(sensitiveId), "Compact card template exposed sensitive pixels.");
                    core.Update(new Dictionary<string,string> { ["app.color_mode"] = "dark", ["app.compact_mode"] = "false" }, core.Snapshot.Revision); break;
                case 5:
                    if (!Ready()) return;
                    Check(main.CurrentTheme == Microsoft.UI.Xaml.ElementTheme.Dark, "Native thumbnail theme update failed.");
                    settingsRevision = core.Snapshot.Revision; view.ScrollIntoHistory(view.Rows[40].Id); break;
                case 6:
                    if (!Ready()) return;
                    Check(view.ThumbnailIds.All(view.VisibleThumbnailIds.Contains) && !view.ThumbnailIds.Intersect(initial).Any(),
                        "Offscreen/recycled cards retained earlier image identities.");
                    view.ScrollToEnd(); break;
                case 7:
                    if (view.Rows.Count <= 50 || !Ready()) return;
                    pagedRealized = view.RealizedCount;
                    Check(pagedRealized < view.Rows.Count && view.ThumbnailResources < view.Rows.Count / 2
                        && view.ThumbnailIds.All(view.VisibleThumbnailIds.Contains), "Continuation retained unbounded containers/offscreen bitmaps.");
                    view.ScrollIntoHistory(sensitiveId); break;
                case 8:
                    if (!Ready()) return;
                    changedId = view.ThumbnailIds.First(ordinaryIds.Contains); view.Select(changedId);
                    await core.SetTagsAsync(changedId, ["gallery", "password"]); break;
                case 9:
                    if (!view.ExternalPending) return;
                    Check(view.ThumbnailResources == 0 && view.ThumbnailIds.Length == 0 && view.Rows.Single(row => row.Id == changedId).IsStale,
                        "External privacy change retained thumbnail resources.");
                    await view.ReloadAsync(changedId); break;
                case 10:
                    if (!Ready()) return;
                    Check(!view.ThumbnailIds.Contains(changedId) && view.CanReveal && !view.HasImagePreview, "Refreshed sensitive image bypassed consent.");
                    main.NavigateTags(); Check(view.ThumbnailResources == 0, "Navigation retained thumbnails."); break;
                case 11:
                    Check(view.ThumbnailResources == 0, "Hidden page reloaded thumbnails."); main.NavigateHistory(); break;
                case 12:
                    if (!Ready()) return;
                    main.HideToTray(); Check(view.ThumbnailResources == 0, "Tray hide retained images."); break;
                case 13:
                    Check(!main.AppWindow.IsVisible && view.ThumbnailResources == 0, "Hidden native window read thumbnails."); main.ShowMain(); break;
                case 14:
                    if (!Ready()) return;
                    ((Microsoft.UI.Windowing.OverlappedPresenter)main.AppWindow.Presenter).Minimize(); break;
                case 15:
                    Check(view.ThumbnailResources == 0, "Minimize retained images."); main.ShowMain(); break;
                case 16:
                    if (!Ready()) return;
                    view.ChooseLayout("list"); Check(view.ThumbnailResources == 0, "List switch retained cards."); HoldReads(); view.ChooseLayout("grid"); break;
                case 17:
                    if (held.Count < 2) return;
                    Check(view.ThumbnailRequests == 2 && held.All(item => !item.Token.IsCancellationRequested), "Delayed readers were not bounded/current.");
                    view.ScrollIntoHistory(view.Rows[40].Id); break;
                case 18:
                    Check(held.Count == 2 && held.All(item => item.Token.IsCancellationRequested) && view.ThumbnailResources == 0
                        && view.ThumbnailRequests == 2, "Recycled readers were not cancelled or freed slots before native completion.");
                    ReleaseReads(); break;
                case 19:
                    if (!Ready()) return;
                    Check(view.ThumbnailIds.All(view.VisibleThumbnailIds.Contains) && !view.ThumbnailIds.Intersect(held.Select(item => item.Preview.Id)).Any(),
                        "Late recycled responses painted a new card.");
                    view.ChooseLayout("list"); HoldReads(); view.ChooseLayout("grid"); break;
                case 20:
                    if (held.Count < 2) return;
                    view.ChooseLayout("list"); view.ChooseLayout("grid"); break;
                case 21:
                    Check(held.Count == 2 && held.All(item => item.Token.IsCancellationRequested) && view.ThumbnailIds.Length == 0
                        && view.ThumbnailRequests == 2, "Layout change reused delayed readers or exceeded slots.");
                    ReleaseReads(); break;
                case 22:
                    if (!Ready()) return;
                    view.ChooseLayout("list"); HoldReads(); view.ChooseLayout("grid"); break;
                case 23:
                    if (held.Count < 2) return;
                    view.Search("no matching thumbnail fixture"); await view.ReloadAsync();
                    Check(held.All(item => item.Token.IsCancellationRequested), "New query did not cancel thumbnail reads."); ReleaseReads(); break;
                case 24:
                    if (view.ThumbnailRequests != 0) return;
                    Check(view.Rows.Count == 0 && view.ThumbnailResources == 0, "Old query response repopulated empty results.");
                    view.Search(""); await view.ReloadAsync(); break;
                case 25:
                    if (!Ready()) return;
                    view.ChooseLayout("list"); HoldReads(); view.ChooseLayout("grid"); break;
                case 26:
                    if (held.Count < 2) return;
                    settingsRevision = core.Snapshot.Revision;
                    changedId = held[0].Preview.Id; view.Select(changedId);
                    await core.SetTagsAsync(changedId, ["gallery", "password"]); break;
                case 27:
                    if (!view.ExternalPending) return;
                    Check(held.All(item => item.Token.IsCancellationRequested) && view.ThumbnailResources == 0, "Privacy transition kept delayed readers.");
                    ReleaseReads(); break;
                case 28:
                    if (view.ThumbnailRequests != 0) return;
                    Check(view.ThumbnailIds.Length == 0, "Late public raster bypassed sensitive transition."); await view.ReloadAsync(changedId); break;
                case 29:
                    if (!Ready()) return;
                    Check(!view.ThumbnailIds.Contains(changedId) && !view.ThumbnailIds.Contains(sensitiveId) && core.Snapshot.Revision == settingsRevision,
                        "Privacy reload exposed images or reads saved settings.");
                    view.ChooseLayout("list");
                    view.ThumbnailReader = async (id, revision, token) => (await core.GetImagePreviewAsync(id, revision, maxEdge: 256, cancellationToken: token)) with { Id = long.MaxValue };
                    view.ChooseLayout("grid"); break;
                case 30:
                    if (view.ThumbnailRequests != 0 || view.UnavailableThumbnailIds.Length < 2) return;
                    Check(view.ThumbnailIds.Length == 0, "Mismatched raster identity was accepted.");
                    view.ChooseLayout("list");
                    view.ThumbnailReader = async (id, revision, token) => (await core.GetImagePreviewAsync(id, revision, maxEdge: 256, cancellationToken: token)) with { Width = 257 };
                    view.ChooseLayout("grid"); break;
                case 31:
                    if (view.ThumbnailRequests != 0 || view.UnavailableThumbnailIds.Length < 2) return;
                    Check(view.ThumbnailIds.Length == 0, "Oversize raster reached an image control.");
                    view.ChooseLayout("list"); view.ThumbnailReader = null; view.ChooseLayout("grid"); break;
                case 32:
                    if (!Ready()) return;
                    Check(view.ThumbnailPeakRequests == 2 && (await core.GetCaptureStatusAsync()).Starts == 0,
                        "Thumbnail work exceeded concurrency or started capture.");
                    view.ChooseLayout("list"); HoldReads(); view.ChooseLayout("grid"); break;
                case 33:
                    if (held.Count < 2) return;
                    main.HideToTray(); Check(held.All(item => item.Token.IsCancellationRequested), "Hide did not cancel delayed reads."); ReleaseReads(); break;
                case 34:
                    if (view.ThumbnailRequests != 0) return;
                    Check(view.ThumbnailResources == 0 && view.ThumbnailIds.Length == 0, "Hidden late response restored a raster."); main.ShowMain(); break;
                case 35:
                    if (!Ready()) return;
                    view.ChooseLayout("list"); HoldReads(); view.ChooseLayout("grid"); break;
                case 36:
                    if (held.Count < 2) return;
                    peak = view.ThumbnailPeakRequests;
                    view.Dispose(); Check(view.ThumbnailResources == 0 && held.All(item => item.Token.IsCancellationRequested), "Dispose retained readers/images.");
                    ReleaseReads(); break;
                case 37:
                    if (view.ThumbnailRequests != 0) return;
                    Check(view.ThumbnailResources == 0 && !view.HasLayoutResources, "Disposed late readers restored a template."); Finish(true, null); return;
            }
            phase++;
        }
        catch (Exception error) { Finish(false, $"Thumbnail phase {phase}: {error}"); }
        finally { ticking = false; }
    }
    private async void Finish(bool passed, string? error)
    {
        timer!.Stop(); timer.Tick -= Tick; ReleaseReads();
        bool webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
            .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
        try
        {
            Check(!webView2ModuleLoaded, "WebView2 loaded in native thumbnails.");
            if (!await main.Quit()) throw new Exception("Native shutdown did not release the core."); Check(main.History.ThumbnailResources == 0 && !main.History.HasLayoutResources, "Quit retained thumbnail resources.");
            Directory.Delete(directory, recursive: true);
        }
        catch (Exception failure) { passed = false; error = $"{error}\n{failure}"; }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "history-thumbnails", phases = phase + 1,
            visibleImages, initialRealized, pagedRealized, peakRequests = peak, webView2ModuleLoaded, elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
