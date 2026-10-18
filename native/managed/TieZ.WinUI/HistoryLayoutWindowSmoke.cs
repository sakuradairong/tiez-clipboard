using System.Diagnostics;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

internal sealed class HistoryLayoutWindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private DispatcherQueueTimer? timer;
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private bool ticking;
    private int phase, realized, wideColumns;
    private long sensitiveId, changedId, imageId, richId;
    private long? anchor;
    private long[] selected = [];
    private ulong historyRevision, settingsRevision;
    private const string Sensitive = "private layout body", Html = "<b>native layout</b>";
    private const string Png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAYAAAC56t6BAAAAEUlEQVR4nGP4z8DQAMIMGAwAfcEI+4thakwAAAAASUVORK5CYII=";
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    internal void Start()
    {
        timer = main.DispatcherQueue.CreateTimer(); timer.Interval = TimeSpan.FromMilliseconds(350); timer.Tick += Tick; timer.Start();
    }
    private async void Tick(DispatcherQueueTimer sender, object args)
    {
        if (ticking) return; ticking = true;
        try
        {
            Check(elapsed.Elapsed < TimeSpan.FromSeconds(50), $"Timed out in layout phase {phase}.");
            var view = main.History;
            if (!view.IsLoaded || view.IsBusy || (view.HasDialog && !view.IsDialogReady)) return;
            switch (phase)
            {
                case 0:
                    for (int i = 0; i < 240; i++) await core.AddManualAsync($"layout fixture {i:000}", tags: ["work"]);
                    imageId = (await core.AddManualAsync(Png, "image", tags: ["work"])).Id;
                    richId = (await core.AddManualAsync("  rich layout\r\n ", "rich_text", Html, ["work"])).Id;
                    sensitiveId = (await core.AddManualAsync(Sensitive, tags: ["password", "work"])).Id;
                    historyRevision = (await core.SetPinnedAsync(sensitiveId, true)).Revision;
                    await view.ReloadAsync(); break;
                case 1:
                    Check(view.ViewLayout == "list" && view.Rows.Count == 50 && view.InactiveLayoutReleased && core.Snapshot.Revision == 0,
                        "Ordinary default loaded/wrote a second layout.");
                    selected = [sensitiveId, view.Rows.First(row => !row.Summary.IsSensitive).Id];
                    view.SelectMany(selected); view.ChooseLayout("grid"); break;
                case 2:
                    Check(view.ViewLayout == "grid" && view.SelectedHistoryIds.Order().SequenceEqual(selected.Order()) && view.CanDelete
                        && view.Detail == "" && view.InactiveLayoutReleased, "Grid switch lost multiselection or disclosed content.");
                    realized = view.RealizedCount;
                    Check(realized > 0 && realized < 50 && view.CardWidth > 0, "Grid did not realize a virtualized native page.");
                    Check((await core.GetHistoryAsync(new())).Revision == historyRevision && core.Snapshot.Values["app.history_view_mode"] == "grid"
                        && core.Snapshot.Values["app.tag_history_view_mode"] == "grid", "Layout wrote history or wrong preference scope.");
                    view.ChooseLayout("list"); break;
                case 3:
                    Check(view.SelectedHistoryIds.Order().SequenceEqual(selected.Order()) && view.InactiveLayoutReleased, "List switch lost selection/retained inactive rows.");
                    view.ScrollIntoHistory(view.Rows[20].Id); break;
                case 4:
                    anchor = view.ViewportAnchorId; Check(anchor is not null, "Scrolled list lost viewport anchor."); view.ChooseLayout("grid"); break;
                case 5:
                    Check(view.IsHistoryItemVisible(anchor!.Value), "Layout switch lost the visible record anchor.");
                    view.Select(sensitiveId); view.Invoke("reveal"); break;
                case 6:
                    Check(view.Detail == Sensitive, "Sensitive reveal did not load in cards."); view.ChooseLayout("list"); break;
                case 7:
                    Check(view.SelectedHistoryIds.SequenceEqual(new[] { sensitiveId }) && view.Detail == "" && view.CanReveal,
                        "Layout reused revealed sensitive content.");
                    view.Invoke("tags"); break;
                case 8:
                    Check(!view.CanChangeLayout && view.TagPicker is { CanSave: true }, "Dialog did not block layout changes.");
                    view.CancelDialog(); break;
                case 9:
                    view.ChooseLayout("grid");
                    core.Update(new Dictionary<string,string> { ["app.compact_mode"] = "true" }, core.Snapshot.Revision); break;
                case 10:
                    Check(view.ViewLayout == "grid" && view.CompactMode && view.SelectedHistoryIds.SequenceEqual(new[] { sensitiveId })
                        && view.Detail == "" && view.InactiveLayoutReleased, "Compact setting changed layout/selection or exposed content.");
                    core.Update(new Dictionary<string,string> { ["app.color_mode"] = "dark", ["app.compact_mode"] = "false" }, core.Snapshot.Revision);
                    view.ChooseLayout("list"); break;
                case 11:
                    Check(main.CurrentTheme == Microsoft.UI.Xaml.ElementTheme.Dark && view.ViewLayout == "list", "Theme/layout update not accepted.");
                    settingsRevision = core.Snapshot.Revision; view.ShowTag("work"); break;
                case 12:
                    Check(view.ViewLayout == "grid" && view.ActiveTagFilter == "work" && view.SortOrder == "time" && view.Rows.Count == 50
                        && core.Snapshot.Revision == settingsRevision, "Tag scope failed default card preference/read-only binding.");
                    view.ChooseLayout("list"); break;
                case 13:
                    Check(core.Snapshot.Values["app.history_view_mode"] == "list" && core.Snapshot.Values["app.tag_history_view_mode"] == "list",
                        "Tag layout modified the ordinary preference.");
                    view.Filter(0, "", false); break;
                case 14:
                    Check(view.ViewLayout == "list", "Clearing tag scope did not restore ordinary layout."); view.ChooseLayout("grid"); break;
                case 15: view.ShowTag("work"); break;
                case 16:
                    Check(view.ViewLayout == "list", "Tag preference overwritten by ordinary card choice.");
                    view.Search("no-layout-record-matches"); break;
                case 17:
                    Check(view.Rows.Count == 0 && !view.CanDelete && view.Detail == "", "Empty query retained actions/body."); view.ChooseLayout("grid"); break;
                case 18:
                    Check(view.ViewLayout == "grid" && view.Rows.Count == 0 && view.InactiveLayoutReleased, "Empty layout switch retained old items.");
                    view.Search(""); break;
                case 19:
                    Check(view.Rows.Count == 50, "Clearing card search lost bounded first page."); view.Invoke("more"); break;
                case 20:
                    Check(view.Rows.Count == 100 && view.Rows.Select(row => row.Id).Distinct().Count() == 100, "Card continuation duplicated/skipped page.");
                    main.AppWindow.Resize(new SizeInt32(1560, 900)); break;
                case 21:
                    wideColumns = view.RealizedColumns;
                    Check(wideColumns > 1 && view.RealizedCount < view.Rows.Count / 2, "Wide card view did not wrap/virtualize.");
                    main.AppWindow.Resize(new SizeInt32(900, 760)); break;
                case 22:
                    Check(view.RealizedColumns < wideColumns && view.CardWidth > 0 && view.InactiveLayoutReleased, "Resize did not reflow cards.");
                    selected = [view.Rows[0].Id, view.Rows[1].Id]; view.SelectMany(selected);
                    settingsRevision = core.Snapshot.Revision; main.HideToTray(); main.ShowMain(); break;
                case 23:
                    Check(view.ViewLayout == "grid" && view.SelectedHistoryIds.Order().SequenceEqual(selected.Order()) && core.Snapshot.Revision == settingsRevision,
                        "Hide/reopen changed layout/selection or wrote settings.");
                    main.OpenSettings(); main.Settings!.SetSmokeHotkey("Ctrl+Alt+F9"); view.ChooseLayout("list"); break;
                case 24:
                    Check(main.Settings!.HasError && main.Settings.DraftHotkey == "Ctrl+Alt+F9", "Layout save overwrote another window's dirty draft.");
                    main.CloseSettings();
                    changedId = view.Rows.First(row => !row.Summary.IsSensitive && row.Summary.ContentType == "text").Id;
                    view.Select(changedId); break;
                case 25:
                    Check(view.Detail.StartsWith("layout fixture"), "Selected card/list detail not reconciled.");
                    var meta = await core.GetTagSelectionAsync(changedId);
                    historyRevision = (await core.SetExactTagsAsync(changedId, ["work", "password"], meta.Revision)).Revision; break;
                case 26:
                    if (!view.ExternalPending) return;
                    Check(view.Detail == "" && view.Rows.Single(row => row.Id == changedId).IsStale, "External privacy change did not invalidate detail/row.");
                    view.ChooseLayout("grid"); break;
                case 27:
                    Check(view.Detail == "" && !view.CanCopy && view.Rows.Single(row => row.Id == changedId).Preview == "记录已更新 · 请刷新",
                        "Switching stale layout restored private content/actions.");
                    await view.ReloadAsync(changedId); break;
                case 28:
                    Check(view.CanReveal && view.Detail == "", "Refreshed private card bypassed consent."); view.Invoke("tags"); break;
                case 29:
                    var picker = view.TagPicker ?? throw new InvalidOperationException("No picker in cards.");
                    Check(!view.CanChangeLayout && picker.Selection.Contains("password"), "Card selection not routed into tag picker.");
                    picker.NameDraft = "layout-selection"; picker.Invoke("add"); view.ConfirmDialog(); break;
                case 30:
                    Check(!view.HasDialog && (await core.GetTagSelectionAsync(changedId)).Tags.Contains("layout-selection") && view.Detail == "",
                        "Card tag save lost identity or revealed body.");
                    view.Select(richId); break;
                case 31:
                    Check(view.HasDocumentPreview && view.PreviewedDocumentId == richId, "Card rich record did not load native document detail.");
                    view.ChooseLayout("list"); break;
                case 32:
                    Check(view.HasDocumentPreview && view.PreviewedDocumentId == richId && (await core.GetEntryAsync(richId)).HtmlContent == Html,
                        "List/card switch changed rich format or preview identity.");
                    view.Select(imageId); break;
                case 33:
                    Check(view.HasImagePreview && view.PreviewedImageId == imageId && view.ImagePixelWidth == 2, "List image detail failed.");
                    view.ChooseLayout("grid"); break;
                case 34:
                    Check(view.HasImagePreview && view.PreviewedImageId == imageId && view.InactiveLayoutReleased, "Card image detail lost original identity.");
                    Check((await core.GetCaptureStatusAsync()).Starts == 0, "Layout changes started duplicate/unrequested capture.");
                    Finish(true, null); return;
            }
            phase++;
        }
        catch (Exception error) { Finish(false, $"Layout phase {phase}: {error}"); }
        finally { ticking = false; }
    }
    private void Finish(bool passed, string? error)
    {
        timer!.Stop(); timer.Tick -= Tick;
        ulong revision = core.Snapshot.Revision;
        bool webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
            .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
        try
        {
            Check(!webView2ModuleLoaded, "WebView2 loaded in history layouts.");
            main.Quit(); Check(!main.History.HasLayoutResources, "Quit retained layout sources/rows."); Directory.Delete(directory, recursive: true);
        }
        catch (Exception failure) { passed = false; error = $"{error}\n{failure}"; }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "history-layout", phases = phase + 1,
            revision, realized, wideColumns, webView2ModuleLoaded, elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
