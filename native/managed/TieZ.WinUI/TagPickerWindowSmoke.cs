using System.Diagnostics;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using TieZ.Core;

namespace TieZ.Native;

internal sealed class TagPickerWindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private DispatcherQueueTimer? timer;
    private bool ticking;
    private int phase, realized;
    private long id;
    private ulong revision;
    private TagPickerView? retained;
    private const string Raw = "  旧目录 ' %_  ", Body = "  picker original\r\n ", Html = "<b>original</b>";
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    internal void Start()
    {
        timer = main.DispatcherQueue.CreateTimer(); timer.Interval = TimeSpan.FromMilliseconds(300); timer.Tick += Tick; timer.Start();
    }
    private TagPickerView Picker => main.History.TagPicker ?? throw new InvalidOperationException("Picker not open.");
    private async void Tick(DispatcherQueueTimer sender, object args)
    {
        if (ticking) return; ticking = true;
        try
        {
            Check(elapsed.Elapsed < TimeSpan.FromSeconds(45), $"Timed out in picker phase {phase}.");
            var history = main.History;
            if (!history.IsLoaded || history.IsBusy || (history.HasDialog && !history.IsDialogReady)) return;
            switch (phase)
            {
                case 0:
                    for (int i = 0; i < 212; i++) revision = (await core.CreateTagAsync($"picker-{i:000}", revision)).Revision;
                    revision = (await core.SetTagColorAsync(Raw, "#123456", revision)).Revision;
                    revision = (await core.CreateTagAsync("password", revision)).Revision;
                    var added = await core.AddManualAsync(Body, "rich_text", Html); id = added.Id;
                    revision = (await core.SetExactTagsAsync(id, [Raw], added.Revision)).Revision;
                    await history.ReloadAsync(id); break;
                case 1: history.Select(id); history.Invoke("tags"); break;
                case 2:
                    Check(Picker.CanSave && Picker.Selection.SequenceEqual(new[] { Raw }) && Picker.Catalog.Count == 100
                        && Picker.HasMore && Picker.Revision == revision && !Picker.HasError, "Metadata/first catalog read failed.");
                    Picker.ObserveRevision(revision - 1);
                    Check(Picker.CanSave && !Picker.IsStale, "Late older history poll invalidated a newer accepted snapshot.");
                    realized = Picker.RealizedCount;
                    Check(realized > 0 && realized < 100, "Catalog rows did not virtualize.");
                    Check(new PickedTagName(" ").Label == "\" \"" && new PickedTagName("\n").Label.Contains("\\n"), "Legacy names were not visibly escaped.");
                    Picker.Choose("picker-000", true);
                    Picker.NameDraft = "  新草稿  "; Picker.Invoke("add");
                    Picker.SearchDraft = "PICKER-211"; Picker.Invoke("search"); break;
                case 3:
                    Check(Picker.Catalog.Count == 1 && Picker.Catalog[0].Name == "picker-211" && Picker.Selection.Contains("新草稿")
                        && Picker.Selection.Contains(Raw) && Picker.Selection.Contains("picker-000"), "Whole-catalog Unicode search lost selection.");
                    Picker.Choose("picker-211", true); Picker.SearchDraft = ""; Picker.Invoke("search"); break;
                case 4:
                    Check(Picker.Catalog.Count == 100 && Picker.Selection.Contains("picker-211"), "Clearing search lost off-page selection.");
                    Picker.Invoke("more"); break;
                case 5:
                    Check(Picker.Catalog.Count == 200 && Picker.HasMore, "Second picker page not bounded.");
                    Picker.Invoke("more"); break;
                case 6:
                    Check(Picker.Catalog.Count == 216 && !Picker.HasMore && Picker.Catalog.Select(tag => tag.Name).Distinct().Count() == 216,
                        "Picker skipped/duplicated catalog pages.");
                    Check(Picker.Catalog.Single(tag => tag.Name == Raw).Color == "#123456", "Exact catalog name/color lost.");
                    Picker.Choose("picker-210", true); retained = Picker; history.CancelDialog(); break;
                case 7:
                    Check(!history.HasDialog && retained is { HasRetainedData: false }, "Cancel retained picker resources.");
                    Check((await core.GetTagSelectionAsync(id)).Tags.SequenceEqual(new[] { Raw })
                        && (await core.GetTagSelectionAsync(id)).Revision == revision && core.Snapshot.Revision == 0, "Opening/search/select/cancel wrote state.");
                    history.Invoke("tags"); break;
                case 8:
                    Picker.NameDraft = "invalid\nname";
                    Check(Picker.NameDraft == "" && Picker.HasError, "New tag accepted a line break.");
                    Picker.NameDraft = " keep draft "; Picker.Invoke("add"); Picker.NameDraft = "未加入的输入";
                    revision = (await core.CreateTagAsync("external-picker", revision)).Revision;
                    Picker.Invoke("search"); break;
                case 9:
                    Check(Picker.IsStale && !Picker.CanSave && Picker.Selection.Contains("keep draft") && Picker.NameDraft == "未加入的输入",
                        "Conflict discarded selection/input or left save enabled.");
                    Check((await core.GetTagSelectionAsync(id)).Tags.SequenceEqual(new[] { Raw }), "Conflict changed stored tags.");
                    Picker.Invoke("reload"); break;
                case 10:
                    Check(Picker.CanSave && !Picker.IsStale && Picker.Revision == revision && Picker.Selection.SequenceEqual(new[] { Raw })
                        && Picker.NameDraft == "", "Explicit reread did not adopt accepted metadata.");
                    Picker.SearchDraft = "PASSWORD"; Picker.Invoke("search"); break;
                case 11:
                    Check(Picker.Catalog.Single().IsSensitive, "Sensitive catalog classification lost.");
                    Picker.Choose("password", true); history.ConfirmDialog(); break;
                case 12:
                    Check(!history.HasDialog && history.Rows.Single().Summary.IsSensitive && history.Detail == "", "Sensitive selection leaked detail.");
                    var sensitive = await core.GetTagSelectionAsync(id); revision = sensitive.Revision;
                    Check(sensitive.Tags.SequenceEqual(new[] { Raw, "password" }), "Exact save did not preserve selection order/name.");
                    history.Select(id); history.Invoke("tags"); break;
                case 13:
                    Check(Picker.Selection.Contains("password") && Picker.Selection.Contains(Raw) && history.Detail == "", "Opening sensitive picker exposed content/lost metadata.");
                    Picker.SelectForRemoval("password"); Picker.Invoke("remove"); history.CancelDialog(); break;
                case 14:
                    Check((await core.GetTagSelectionAsync(id)).Tags.Contains("password"), "Canceled removal removed protection.");
                    history.Invoke("tags"); break;
                case 15:
                    Picker.SelectForRemoval("password"); Picker.Invoke("remove"); history.ConfirmDialog(); break;
                case 16:
                    var ordinary = await core.GetTagSelectionAsync(id); revision = ordinary.Revision;
                    var entry = await core.GetPreviewEntryAsync(id, revision, false);
                    Check(!history.HasDialog && entry.Content == Body && entry.HtmlContent == Html && entry.Tags.SequenceEqual(new[] { Raw }),
                        "Removing sensitivity changed original formats/name.");
                    history.Select(id); history.Invoke("tags"); break;
                case 17:
                    Picker.Choose("picker-001", true); retained = Picker; main.HideToTray(); main.ShowMain(); break;
                case 18:
                    Check(!history.HasDialog && retained is { HasRetainedData: false }
                        && (await core.GetTagSelectionAsync(id)).Revision == revision, "Hide kept picker draft/core or wrote tags.");
                    history.Select(id); history.Invoke("tags"); break;
                case 19:
                    Picker.Choose("picker-001", true); revision = (await core.DeleteEntryAsync(id)).Revision;
                    Picker.Invoke("reload"); break;
                case 20:
                    Check(Picker.IsStale && !Picker.CanSave && Picker.Selection.Contains("picker-001") && Picker.Selection.Contains(Raw),
                        "Failed reread discarded the draft of a deleted entry.");
                    history.CancelDialog(); await history.ReloadAsync();
                    var replacement = await core.AddManualAsync(Body); id = replacement.Id; revision = replacement.Revision;
                    await history.ReloadAsync(id); break;
                case 21: history.Select(id); history.Invoke("tags"); break;
                case 22:
                    Check(Picker.Selection.Count == 0, "New record inherited old picker selection.");
                    Picker.SearchDraft = "不存在目录"; Picker.Invoke("search"); break;
                case 23:
                    Check(Picker.Catalog.Count == 0 && Picker.CanSave && !Picker.HasMore, "Empty search left stale choices/save state.");
                    history.ConfirmDialog(); break;
                case 24:
                    Check(!history.HasDialog && (await core.GetTagSelectionAsync(id)).Revision == revision, "Unchanged selection advanced revision.");
                    history.Select(id); history.Invoke("tags"); break;
                case 25:
                    Picker.NameDraft = "新增标签"; Picker.Invoke("add"); history.ConfirmDialog(); break;
                case 26:
                    var created = await core.GetTagSelectionAsync(id); revision = created.Revision;
                    Check(created.Tags.SequenceEqual(new[] { "新增标签" }) && (await core.GetTagsAsync(new(Search: "新增标签"))).Items.Single().Count == 1,
                        "New free-input tag did not enter global catalog through record save.");
                    history.Select(id); history.Invoke("tags"); break;
                case 27:
                    Picker.NameDraft = "尚未加入的输入"; history.ConfirmDialog(); break;
                case 28:
                    Check(history.HasDialog && Picker.NameDraft == "尚未加入的输入" && Picker.HasError
                        && (await core.GetTagSelectionAsync(id)).Revision == revision, "Save silently discarded unadded input or wrote tags.");
                    Picker.NameDraft = "退出草稿"; Picker.Invoke("add"); retained = Picker; Finish(true, null); return;
            }
            phase++;
        }
        catch (Exception error) { Finish(false, $"Picker phase {phase}: {error}"); }
        finally { ticking = false; }
    }
    private async void Finish(bool passed, string? error)
    {
        timer!.Stop(); timer.Tick -= Tick;
        bool webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
            .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
        try
        {
            Check(!webView2ModuleLoaded, "WebView2 loaded in tag picker.");
            if (!await main.Quit()) throw new Exception("Native shutdown did not release the core."); Check(retained is null || !retained.HasRetainedData, "Quit retained picker drafts/core.");
            Directory.Delete(directory, recursive: true);
        }
        catch (Exception failure) { passed = false; error = $"{error}\n{failure}"; }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "tag-picker", phases = phase + 1,
            revision, realized, webView2ModuleLoaded, elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
