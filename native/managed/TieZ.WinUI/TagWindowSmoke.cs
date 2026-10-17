using System.Diagnostics;
using System.Text.Json;
using Microsoft.UI;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using TieZ.Core;
using Colors = Microsoft.UI.Colors;

namespace TieZ.Native;

// Real navigation, ColorPicker, fields/buttons and DLL. One shared core, isolated data.
internal sealed class TagWindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private DispatcherQueueTimer? timer;
    private int phase, realized;
    private bool ticking;
    private long entryId;
    private ulong tagRevision;
    private const string Existing = "tag-000", Legacy = "  tag-000  ", Created = "aaa-原生标签";
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    internal void Start()
    {
        timer = main.DispatcherQueue.CreateTimer(); timer.Interval = TimeSpan.FromMilliseconds(250);
        timer.Tick += Tick; timer.Start();
    }
    private TagInfo ReadTag(string name) => core.GetTagsAsync(new TagQuery(Limit: 200)).GetAwaiter().GetResult().Items.Single(tag => tag.Name == name);
    private async void Tick(DispatcherQueueTimer sender, object args)
    {
        if (ticking) return;
        ticking = true;
        try
        {
            Check(elapsed.Elapsed < TimeSpan.FromSeconds(40), $"Timed out in tag phase {phase}.");
            var view = main.Tags;
            if (view.IsBusy) return;
            switch (phase)
            {
                case 0:
                    if (!main.History.IsLoaded || main.History.IsBusy) return;
                    Check(view.Count == 0 && !view.IsActive && core.Snapshot.Revision == 0, "Inactive tag page queried or saved data on startup.");
                    Check(NativeTagColor.TryParse("#1238", out var shorthand) && NativeTagColor.Css(shorthand) == "#11223388", "CSS shorthand alpha order.");
                    Check(NativeTagColor.TryParse("rgba(17,34,51,.5)", out var rgba) && rgba.A == 128 && rgba.R == 17 && rgba.B == 51, "Legacy RGBA parsing.");
                    Check(NativeTagColor.TryParse("navy", out var named) && named == Colors.Navy, "Legacy named color parsing.");
                    Check(!NativeTagColor.TryParse("var(--legacy-accent)", out _), "Unsupported legacy color was reinterpreted.");
                    for (int index = 0; index < 105; index++) tagRevision = (await core.CreateTagAsync($"tag-{index:000}", tagRevision)).Revision;
                    tagRevision = (await core.SetTagColorAsync(Legacy, "var(--legacy-accent)", tagRevision)).Revision;
                    var added = await core.AddManualAsync("  标签窗口\r\n ", tags: new[] { Existing }); entryId = added.Id; tagRevision = added.Revision;
                    main.NavigateTags();
                    break;
                case 1:
                    if (!view.IsContentReady || view.DisplayedRevision is null) return;
                    Check(view.IsActive && view.Count == 100 && view.HasMore && view.DisplayedRevision == tagRevision
                        && !view.HasDraft && core.Snapshot.Revision == 0, "Tag first page/read-only binding failed.");
                    realized = view.RealizedCount;
                    Check(realized > 0 && realized < view.Count, "Tag list did not virtualize its native rows.");
                    view.Invoke("more");
                    break;
                case 2:
                    Check(view.Count == 108 && !view.HasMore, "Tag continuation duplicated or omitted rows.");
                    view.Select(Existing);
                    Check(ReadTag(Existing).Count == 1 && view.SelectedTag == Existing, "Tag count or native selection failed.");
                    view.ChooseColor(ColorHelper.FromArgb(128, 17, 34, 51));
                    break;
                case 3:
                    Check(view.HasDraft && view.CanSave && view.ColorDraft == "#11223380" && ReadTag(Existing).Color is null,
                        "Native ColorPicker auto-saved or lost CSS alpha order.");
                    view.Invoke("save");
                    break;
                case 4:
                    Check(!view.HasDraft && !view.HasError && view.SelectedTag == Existing && view.Count == 108
                        && view.DisplayedRevision == tagRevision + 1 && ReadTag(Existing).Color == "#11223380", "Tag color save/reload failed.");
                    tagRevision++;
                    view.EditColor("rgba(17,34,51,0.5019607843137255)");
                    Check(!view.HasDraft && !view.CanSave, "Equivalent CSS color created a format-only draft.");
                    view.EditColor("not-a-color");
                    Check(view.HasInvalidColor && !view.CanSave, "Invalid color was silently accepted.");
                    view.EditColor("#ff8800"); view.NewNameDraft = "保留的新标签草稿";
                    main.NavigateHistory(); main.NavigateTags();
                    Check(view.HasDraft && view.ColorDraft == "#ff8800" && view.NewNameDraft == "保留的新标签草稿", "Navigation discarded tag drafts.");
                    core.Update(new Dictionary<string, string> { ["app.color_mode"] = "dark" }, core.Snapshot.Revision);
                    tagRevision = (await core.SetTagColorAsync(Existing, "navy", tagRevision)).Revision;
                    view.Invoke("save");
                    break;
                case 5:
                    if (!view.HasConflict) return;
                    Check(view.HasDraft && view.ColorDraft == "#ff8800" && view.NewNameDraft == "保留的新标签草稿"
                        && !view.CanSave && ReadTag(Existing).Color == "navy" && view.CurrentTheme == ElementTheme.Dark,
                        "External tag conflict lost drafts, overwrote accepted color or failed to follow theme.");
                    view.Invoke("reload");
                    break;
                case 6:
                    Check(!view.HasDraft && !view.HasConflict && !view.HasError && view.ColorDraft == "navy"
                        && view.NewNameDraft == "保留的新标签草稿", "Explicit reload lost new-name draft or failed to accept actual color.");
                    view.NewNameDraft = Created; view.Invoke("create");
                    break;
                case 7:
                    Check(view.Count == 109 && view.SelectedTag == Created && view.NewNameDraft == ""
                        && view.DisplayedRevision == tagRevision + 1 && ReadTag(Created).Count == 0, "Create tag failed or wrote wrong count/selection.");
                    tagRevision++; view.NewNameDraft = Created; view.Invoke("create");
                    break;
                case 8:
                    Check(view.Count == 109 && view.DisplayedRevision == tagRevision && !view.HasError, "Duplicate creation advanced revision or duplicated UI.");
                    view.EditColor("#00000000"); view.Invoke("save");
                    break;
                case 9:
                    Check(!view.HasDraft && ReadTag(Created).Color == "#00000000" && view.DisplayedRevision == tagRevision + 1,
                        "Explicit transparent color was confused with no color.");
                    tagRevision++; view.Invoke("clear");
                    break;
                case 10:
                    Check(view.HasDraft && view.ColorDraft == "" && ReadTag(Created).Color == "#00000000", "Default-color draft was auto-saved.");
                    view.Invoke("save");
                    break;
                case 11:
                    Check(!view.HasDraft && ReadTag(Created).Color is null && view.DisplayedRevision == tagRevision + 1, "Clear tag color failed.");
                    tagRevision++; view.Select(Legacy);
                    Check(view.ColorDraft == "var(--legacy-accent)" && !view.HasDraft && !view.HasInvalidColor && !view.CanSave,
                        "Binding an unsupported legacy color rewrote it or blocked no-op preservation.");
                    view.Invoke("filter");
                    break;
                case 12:
                    if (main.History.IsBusy) return;
                    Check(!view.IsActive && main.History.ActiveTagFilter == Legacy && main.History.Rows.Count == 0
                        && ReadTag(Legacy).Color == "var(--legacy-accent)", "Catalog tag filter trimmed the name or altered its legacy color.");
                    main.NavigateTags(); view.Select(Existing); view.Invoke("filter");
                    break;
                case 13:
                    if (main.History.IsBusy) return;
                    Check(main.History.ActiveTagFilter == Existing && main.History.Rows.Count == 1 && main.History.Rows[0].Id == entryId,
                        "Native tag filter did not select the matching history.");
                    main.NavigateTags(); view.EditColor("#abcdef"); view.NewNameDraft = "关闭时清除";
                    main.HideToTray();
                    Check(!view.IsActive && view.HasDraft && view.ColorDraft == "#abcdef", "Hide discarded drafts or kept tag reads active.");
                    main.ShowMain();
                    Check(view.IsActive && view.HasDraft && view.ColorDraft == "#abcdef", "Show failed to restore the tag page/draft.");
                    break;
                case 14:
                    Check(ReadTag(Existing).Color == "navy" && core.Snapshot.Revision == 1
                        && (await core.GetCaptureStatusAsync()) is { Running: false, Starts: 0 }, "Tag page changed clipboard service ownership or saved a hidden draft.");
                    Check(core.GetEntryAsync(entryId).GetAwaiter().GetResult().Content == "  标签窗口\r\n ", "Tag navigation altered entry whitespace.");
                    view.Invoke("reload");
                    break;
                case 15:
                    view.EditColor("#010203"); view.NewNameDraft = "退出时清除";
                    Finish(true, null); return;
            }
            phase++;
        }
        catch (Exception error) { Finish(false, $"Tag phase {phase}: {error}"); }
        finally { ticking = false; }
    }
    private void Finish(bool passed, string? error)
    {
        timer!.Stop(); timer.Tick -= Tick;
        ulong settingsRevision = core.Snapshot.Revision;
        bool webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
            .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
        try
        {
            Check(!webView2ModuleLoaded, "WebView2 loaded during native tags check.");
            main.Quit();
            Check(!main.Tags.HasRetainedData && !main.Tags.IsActive && !main.Tags.HasDraft, "Quit retained tag drafts, rows or core reference.");
            Directory.Delete(directory, recursive: true);
        }
        catch (Exception failure) { passed = false; error = $"{error}\n{failure}"; }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "tags", phases = phase + 1,
            settingsRevision, tagRevision, realized, webView2ModuleLoaded, elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
