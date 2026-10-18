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
    private long entryId, deleteSecond, deleteKept;
    private ulong tagRevision;
    private readonly List<long> sortIds = [];
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
                        && !view.HasDraft && core.Snapshot.Revision == 0 && view.SortOrder == "count"
                        && view.DisplayedTags[0].Name == Existing, "Tag first page/read-only count sort binding failed.");
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
                    Check(!view.HasDraft, "Reload did not discard old color draft.");
                    view.EditName("tag-renamed"); view.Invoke("rename");
                    break;
                case 16:
                    if (!view.HasDialog) return;
                    Check(view.DialogText.Contains("敏感") && view.DialogText.Contains("合并") && ReadTag(Existing).Count == 1,
                        "Rename confirmation omitted merge/privacy consequences or saved before confirmation.");
                    view.InvokeDialog(confirm: false);
                    break;
                case 17:
                    if (view.HasDialog) return;
                    Check(view.HasDraft && view.RenameDraft == "tag-renamed" && view.DisplayedRevision == tagRevision,
                        "Cancel discarded the name draft or renamed a tag.");
                    main.NavigateHistory(); main.NavigateTags();
                    Check(view.RenameDraft == "tag-renamed" && view.HasDraft, "Navigation lost the rename draft.");
                    view.Invoke("rename");
                    break;
                case 18:
                    if (!view.HasDialog) return;
                    tagRevision = (await core.CreateTagAsync("late-rename-conflict", tagRevision)).Revision;
                    view.InvokeDialog(confirm: true);
                    break;
                case 19:
                    if (view.HasDialog) return;
                    Check(view.HasConflict && view.HasError && view.HasDraft && view.RenameDraft == "tag-renamed"
                        && ReadTag(Existing).Count == 1, "Stale rename changed entry or dropped its draft.");
                    view.Invoke("reload");
                    break;
                case 20:
                    Check(!view.HasDraft && !view.HasConflict && view.SelectedTag == Existing, "Reload did not reset rejected rename state.");
                    view.EditName(Created); view.Invoke("rename");
                    break;
                case 21:
                    if (!view.HasDialog) return;
                    view.InvokeDialog(confirm: true);
                    break;
                case 22:
                    Check(!view.HasDraft && !view.HasError && view.SelectedTag == Created && view.DisplayedRevision == tagRevision + 1
                        && ReadTag(Created).Count == 1 && ReadTag(Created).Color is null, "Rename merge failed, retained wrong source color or lost selection.");
                    tagRevision++;
                    Check(core.GetEntryAsync(entryId).GetAwaiter().GetResult().Tags.SequenceEqual(new[] { Created }), "Rename merge did not update entry tags.");
                    view.EditName("PASSWORD"); view.Invoke("rename");
                    break;
                case 23:
                    if (!view.HasDialog) return;
                    view.InvokeDialog(confirm: true);
                    break;
                case 24:
                    Check(!view.HasDraft && !view.HasError && view.SelectedTag == "PASSWORD" && ReadTag("PASSWORD").IsSensitive
                        && view.DisplayedRevision == tagRevision + 1, "Sensitive native rename failed.");
                    tagRevision++; view.Invoke("filter");
                    break;
                case 25:
                    if (main.History.IsBusy) return;
                    Check(main.History.Rows.Count == 1 && main.History.Rows[0].Summary.IsSensitive && main.History.Rows[0].Id == entryId,
                        "History did not follow renamed sensitive tag.");
                    main.NavigateTags(); view.EditName(Existing); view.Invoke("rename");
                    break;
                case 26:
                    if (!view.HasDialog) return;
                    view.InvokeDialog(confirm: true);
                    break;
                case 27:
                    Check(!view.HasDraft && !view.HasError && view.SelectedTag == Existing && !ReadTag(Existing).IsSensitive
                        && view.DisplayedRevision == tagRevision + 1, "Rename from sensitive did not restore ordinary tag state.");
                    tagRevision++;
                    Check(core.GetPreviewEntryAsync(entryId, tagRevision, false).GetAwaiter().GetResult().Content == "  标签窗口\r\n ",
                        "Native rename changed content or left an ordinary entry sensitive.");
                    view.EditName("退出时清除"); view.NewNameDraft = "退出时清除"; view.Invoke("rename");
                    break;
                case 28:
                    if (!view.HasDialog) return;
                    view.InvokeDialog(confirm: false);
                    break;
                case 29:
                    if (view.HasDialog) return;
                    view.Invoke("reload");
                    break;
                case 30:
                    Check(!view.HasDraft && view.SelectedTag == Existing, "Tag delete setup did not discard the rename draft.");
                    view.Invoke("delete");
                    break;
                case 31:
                    if (!view.HasDialog) return;
                    Check(view.DialogText.Contains("全部 1 条") && view.DialogText.Contains("置顶") && view.DialogText.Contains("整条删除")
                        && view.DialogText.Contains("无法撤销"), "Delete confirmation omitted exact counts or destructive scope.");
                    view.InvokeDialog(confirm: false);
                    break;
                case 32:
                    if (view.HasDialog) return;
                    Check(ReadTag(Existing).Count == 1 && view.DisplayedRevision == tagRevision, "Cancel deleted a tag or entry.");
                    view.Invoke("delete");
                    break;
                case 33:
                    if (!view.HasDialog) return;
                    var secondDeleteFixture = await core.AddManualAsync("delete second", tags: new[] { Existing, "delete-other" });
                    tagRevision = secondDeleteFixture.Revision; deleteSecond = secondDeleteFixture.Id;
                    Check(deleteSecond != 0, "Tag delete conflict fixture was missing.");
                    view.InvokeDialog(confirm: true);
                    break;
                case 34:
                    if (view.HasDialog) return;
                    Check(view.HasError && view.HasConflict && ReadTag(Existing).Count == 2, "Stale delete confirmation removed newly added tagged history.");
                    tagRevision = (await core.SetPinnedAsync(entryId, true)).Revision;
                    var survivor = await core.AddManualAsync("delete survivor", tags: new[] { "delete-other" });
                    deleteKept = survivor.Id; tagRevision = survivor.Revision;
                    view.Invoke("reload");
                    break;
                case 35:
                    Check(!view.HasConflict && view.SelectedTag == Existing && ReadTag(Existing).Count == 2, "Refresh did not accept actual tag delete scope.");
                    view.Invoke("delete");
                    break;
                case 36:
                    if (!view.HasDialog) return;
                    Check(view.DialogText.Contains("全部 2 条") && view.DialogText.Contains("已保存 2 条"), "Refreshed delete preview kept stale counts.");
                    view.InvokeDialog(confirm: true);
                    break;
                case 37:
                    Check(!view.HasDialog && !view.HasError && !view.HasDraft && view.SelectedTag is null && view.DisplayedRevision == tagRevision + 1,
                        "Confirmed tag delete did not refresh selection/version.");
                    tagRevision++;
                    Check(!(await core.GetHistoryAsync(new HistoryQuery(Tag: Existing))).Items.Any(), "Confirmed delete left tagged history.");
                    Check((await core.GetEntryAsync(deleteKept)).Content == "delete survivor", "Tag delete removed an unrelated entry.");
                    Check((await core.GetHistoryChangesAsync(tagRevision - 1)).ResyncRequired, "Tag delete failed to invalidate other views.");
                    view.Select(Created); // This previously merged tag no longer exists; no accidental action may target another row.
                    Check(view.SelectedTag is null, "Selecting a deleted tag selected unrelated data.");
                    view.NewNameDraft = "aaa-empty-delete"; view.Invoke("create");
                    break;
                case 38:
                    Check(view.SelectedTag == "aaa-empty-delete" && ReadTag("aaa-empty-delete").Count == 0, "Zero-count delete fixture creation failed.");
                    tagRevision++; view.Invoke("delete");
                    break;
                case 39:
                    if (!view.HasDialog) return;
                    Check(view.DialogText.Contains("全部 0 条"), "Zero-count tag delete preview was incorrect.");
                    view.InvokeDialog(confirm: true);
                    break;
                case 40:
                    Check(!view.HasError && view.SelectedTag is null && view.DisplayedRevision == tagRevision + 1, "Zero-count tag delete failed or removed another tag.");
                    tagRevision++; view.Select("delete-other"); view.Invoke("filter");
                    break;
                case 41:
                    if (main.History.IsBusy) return;
                    Check(main.History.Rows.Count == 1 && main.History.Rows[0].Id == deleteKept, "History did not reflect tag deletion and retained sibling.");
                    main.NavigateTags(); view.Invoke("delete");
                    break;
                case 42:
                    if (!view.HasDialog) return;
                    main.HideToTray();
                    Check(!view.IsActive, "Hide left tag delete confirmation active.");
                    main.ShowMain();
                    break;
                case 43:
                    if (view.HasDialog) return;
                    Check(ReadTag("delete-other").Count == 1, "Hiding confirmation deleted history.");
                    view.Invoke("delete");
                    break;
                case 44:
                    if (!view.HasDialog) return;
                    view.InvokeDialog(confirm: false);
                    break;
                case 45:
                    if (view.HasDialog) return;
                    view.ChooseSort("name");
                    break;
                case 46:
                    Check(view.SortOrder == "name" && view.SelectedTag == "delete-other"
                        && view.DisplayedTags.Select(tag => tag.Name).SequenceEqual(view.DisplayedTags.Select(tag => tag.Name).Order(StringComparer.Ordinal)),
                        "Name sort did not refresh all loaded pages or preserve selection.");
                    view.ChooseSort("count");
                    break;
                case 47:
                    Check(view.SortOrder == "count" && view.DisplayedTags[0].Name == "delete-other" && view.SelectedTag == "delete-other"
                        && view.DisplayedTags.Select(tag => tag.Count).SequenceEqual(view.DisplayedTags.Select(tag => tag.Count).OrderDescending()),
                        "Count sort was limited to the page or lost selection.");
                    view.EditColor("#aabbcc"); view.ChooseSort("name");
                    Check(view.HasDraft && view.SortOrder == "count" && view.ColorDraft == "#aabbcc", "Sort change discarded an unsaved color.");
                    view.Invoke("reload");
                    break;
                case 48:
                    Check(!view.HasDraft && view.SortOrder == "count" && view.DisplayedRevision == tagRevision, "Sort read saved metadata or failed draft reload.");
                    view.ChooseSort("name");
                    break;
                case 49:
                    Check(view.SortOrder == "name", "Second sort selection failed.");
                    main.HideToTray(); main.ShowMain();
                    break;
                case 50:
                    Check(view.SortOrder == "name" && view.SelectedTag == "delete-other" && view.DisplayedRevision == tagRevision
                        && core.Snapshot.Revision == 1, "Hide/reopen changed sorting or persisted a readonly choice.");
                    view.Invoke("delete");
                    break;
                case 51:
                    if (!view.HasDialog) return;
                    view.InvokeDialog(confirm: false);
                    break;
                case 52:
                    if (view.HasDialog) return;
                    view.SearchDraft = "TAG-10"; view.Invoke("search");
                    break;
                case 53:
                    Check(view.Count == 5 && !view.HasMore && view.AcceptedSearch == "TAG-10"
                        && view.DisplayedTags.All(tag => tag.Name.StartsWith("tag-10", StringComparison.Ordinal)), "Full catalog search omitted late tags.");
                    view.ChooseSort("count");
                    break;
                case 54:
                    Check(view.Count == 5 && view.SortOrder == "count" && view.AcceptedSearch == "TAG-10", "Sort discarded the active tag search.");
                    view.SearchDraft = Legacy; view.Invoke("search");
                    break;
                case 55:
                    Check(view.Count == 1 && view.DisplayedTags[0].Name == Legacy && view.AcceptedSearch == Legacy, "Search changed the raw legacy name/whitespace.");
                    view.SearchDraft = "%_不存在"; view.Invoke("search");
                    break;
                case 56:
                    Check(view.Count == 0 && !view.HasMore && view.SelectedTag is null && !view.HasError, "Empty search retained old rows or selection.");
                    view.Invoke("clear-search");
                    break;
                case 57:
                    Check(view.AcceptedSearch == "" && view.SearchDraft == "" && view.Count == 100 && view.HasMore
                        && view.DisplayedTags.Select(tag => tag.Name).SequenceEqual(core.GetTagsAsync(new TagQuery(Sort: "count")).GetAwaiter().GetResult().Items.Select(tag => tag.Name)),
                        "Clear search did not restore the bounded first catalog page.");
                    view.SearchDraft = "delete-"; view.Invoke("search");
                    break;
                case 58:
                    Check(view.Count == 1 && view.DisplayedTags[0].Name == "delete-other", "Tag search returned unrelated names.");
                    view.Select("delete-other"); view.EditColor("#123456"); view.SearchDraft = "tag-"; view.Invoke("search");
                    Check(view.HasDraft && view.AcceptedSearch == "delete-" && view.Count == 1, "Search discarded an unsaved color.");
                    main.HideToTray(); main.ShowMain();
                    break;
                case 59:
                    Check(view.HasDraft && view.ColorDraft == "#123456" && view.SearchDraft == "tag-" && view.AcceptedSearch == "delete-",
                        "Hide/reopen lost the applied filter, pending search or tag draft.");
                    view.Invoke("reload");
                    break;
                case 60:
                    Check(!view.HasDraft && view.AcceptedSearch == "delete-" && view.SearchDraft == "tag-" && view.DisplayedRevision == tagRevision,
                        "Refresh saved a search, discarded pending input or failed to discard the tag draft.");
                    view.Invoke("clear-search");
                    break;
                case 61:
                    Check(view.AcceptedSearch == "" && !view.HasError && view.DisplayedRevision == tagRevision && core.Snapshot.Revision == 1,
                        "Readonly search changed settings/history revision.");
                    view.Select("delete-other"); view.Invoke("delete");
                    break;
                case 62:
                    if (!view.HasDialog) return;
                    view.InvokeDialog(confirm: false);
                    break;
                case 63:
                    if (view.HasDialog) return;
                    for (int index = 0; index < 210; index++)
                    {
                        var fixture = await core.AddManualAsync($"原生排序 {index:000}", tags: new[] { "order-fixture" });
                        sortIds.Add(fixture.Id); tagRevision = fixture.Revision;
                    }
                    tagRevision = (await core.SetPinnedAsync(sortIds[0], true)).Revision;
                    main.NavigateHistory(); main.History.ShowTag("order-fixture");
                    break;
                case 64:
                    if (main.History.IsBusy) return;
                    Check(main.History.SortOrder == "time" && main.History.ActiveTagFilter == "order-fixture"
                        && main.History.Rows.Count == 50 && main.History.Rows[0].Id == sortIds[^1],
                        "Tag record default time sort or bounded first page failed.");
                    main.History.Invoke("more");
                    break;
                case 65:
                    if (main.History.IsBusy) return;
                    Check(main.History.Rows.Count == 100 && main.History.Rows.Select(row => row.Id).SequenceEqual(sortIds.AsEnumerable().Reverse().Take(100)),
                        "Time sorting changed order or repeated records across pages.");
                    main.History.ChooseSort("default");
                    break;
                case 66:
                    if (main.History.IsBusy) return;
                    Check(main.History.SortOrder == "default" && main.History.Rows[0].Id == sortIds[0], "Explicit default sort lost pinned priority.");
                    main.History.ChooseSort("usage");
                    break;
                case 67:
                    if (main.History.IsBusy) return;
                    var expectedOrder = await core.GetHistoryAsync(new(Tag: "order-fixture", Sort: "usage"));
                    Check(main.History.SortOrder == "usage" && main.History.Rows.Select(row => row.Id).SequenceEqual(expectedOrder.Items.Select(row => row.Id))
                        && main.History.Rows[0].Id == sortIds[0] && main.History.Rows[0].Metadata.Contains("使用 0 次"), "Native usage choice lost the core order, legacy tie priority or displayed count.");
                    main.HideToTray(); main.ShowMain();
                    break;
                case 68:
                    if (main.History.IsBusy) return;
                    Check(main.History.SortOrder == "usage" && main.History.ActiveTagFilter == "order-fixture", "Hide/reopen lost record sort/tag state.");
                    main.History.ChooseSort("time");
                    break;
                case 69:
                    if (main.History.IsBusy) return;
                    Check(main.History.SortOrder == "time" && main.History.Rows[0].Id == sortIds[^1], "Time switch failed after reopen.");
                    main.History.Search("不存在的排序记录");
                    break;
                case 70:
                    if (main.History.IsBusy) return;
                    Check(main.History.Rows.Count == 0 && !main.History.CanDelete && main.History.Detail == "", "Empty sorted search left stale rows/actions/content.");
                    main.History.Search("");
                    break;
                case 71:
                    if (main.History.IsBusy) return;
                    Check(main.History.Rows[0].Id == sortIds[^1] && core.Snapshot.Revision == 1, "Clearing sorted search changed settings or lost order.");
                    main.NavigateTags(); view.Invoke("reload");
                    break;
                case 72:
                    view.Select("delete-other"); view.Invoke("delete");
                    break;
                case 73:
                    if (!view.HasDialog) return;
                    Finish(true, null); return;
            }
            phase++;
        }
        catch (Exception error) { Finish(false, $"Tag phase {phase}: {error}"); }
        finally { ticking = false; }
    }
    private async void Finish(bool passed, string? error)
    {
        timer!.Stop(); timer.Tick -= Tick;
        ulong settingsRevision = core.Snapshot.Revision;
        bool webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
            .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
        try
        {
            Check(!webView2ModuleLoaded, "WebView2 loaded during native tags check.");
            if (!await main.Quit()) throw new Exception("Native shutdown did not release the core.");
            Check(!main.Tags.HasRetainedData && !main.Tags.IsActive && !main.Tags.HasDraft, "Quit retained tag drafts, rows or core reference.");
            Directory.Delete(directory, recursive: true);
        }
        catch (Exception failure) { passed = false; error = $"{error}\n{failure}"; }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "tags", phases = phase + 1,
            settingsRevision, tagRevision, realized, webView2ModuleLoaded, elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
