using System.Diagnostics;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using TieZ.Core;

namespace TieZ.Native;

// Real native controls and DLL, with a disposable isolated database. No production data.
internal sealed class WindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private DispatcherQueueTimer? timer;
    private int phase;
    private bool ticking;
    private long entryId;
    private ulong captureStarts, captureSettingsRevision;
    private int realized, loaded;
    private string savedText = "";
    private static string Lines(string value) => value.Replace("\r\n", "\n").Replace("\r", "\n");
    private bool? webView2ModuleLoaded;
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private const string Original = "  原生测试 %_ \r\n  ";
    private const string Edited = "  修改后中文 %_ \r\n  ";
    internal void Start()
    {
        timer = main.DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(500);
        timer.Tick += Tick;
        timer.Start();
    }

    private static void Require(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }

    private async void Tick(DispatcherQueueTimer sender, object args)
    {
        if (ticking) return;
        ticking = true;
        try
        {
            Require(elapsed.Elapsed < TimeSpan.FromSeconds(55), $"Timed out in phase {phase}.");
            var view = main.History;
            if (view.IsBusy) return;
            switch (phase)
            {
                case 0:
                    Require(core.Snapshot.Revision == 0, "Initial window binding wrote configuration.");
                    Require(!view.CaptureRunning && (await core.GetCaptureStatusAsync()).Starts == 0, "Smoke capture must be explicitly controlled.");
                    main.OpenSettings();
                    var first = main.Settings;
                    main.OpenSettings();
                    Require(ReferenceEquals(first, main.Settings), "Repeated open created duplicate settings.");
                    break;
                case 1: main.Settings!.InvokeSmokeSave(); break;
                case 2:
                    Require(core.Snapshot.Revision == 1 && core.Snapshot.Values["app.language"] == "tw", "Save did not reach core.");
                    Require(!main.SearchVisible && main.CurrentTheme == ElementTheme.Dark, "Main did not synchronize.");
                    main.CloseSettings();
                    break;
                case 3:
                    Require(main.Settings is null, "Settings window was not released.");
                    main.OpenSettings();
                    Require(main.Settings!.DisplayedRevision == 1 && core.Refresh().Revision == 1, "Reopen wrote configuration.");
                    Require(main.Settings.CaptureOptionsLoaded, "Capture settings did not reload accepted values.");
                    main.CloseSettings();
                    core.Update(new Dictionary<string, string> { ["app.show_search_box"] = "true" }, 1);
                    break;
                case 4: view.Invoke("add"); break;
                case 5:
                    Require(view.HasDialog, "Add did not open native dialog.");
                    view.SetDialogText(Original); savedText = view.DialogText; view.ConfirmDialog();
                    break;
                case 6:
                    Require(!view.HasDialog && view.Rows.Count == 1, "Add did not update native list.");
                    entryId = view.Rows[0].Id;
                    Require((await core.GetEntryAsync(entryId)).Content == savedText, "Add changed editor payload.");
                    view.Select(entryId);
                    break;
                case 7:
                    Require(Lines(view.Detail) == Lines(Original), "Detail changed whitespace or CRLF.");
                    view.Invoke("pin");
                    break;
                case 8:
                    Require(view.Rows.Single().Summary.IsPinned, "Pin did not update native list.");
                    view.Invoke("edit");
                    break;
                case 9:
                    view.SetDialogText(new string('x', 70_000)); view.ConfirmDialog();
                    break;
                case 10:
                    Require(view.HasDialog && view.HasError && view.DialogText.Length == 70_000, "Failed save lost draft or closed dialog.");
                    view.SetDialogText(Edited); view.ConfirmDialog();
                    break;
                case 11:
                    Require(!view.HasDialog && Lines(view.Detail) == Lines(Edited), "Edit did not preserve original text.");
                    view.Invoke("tags");
                    break;
                case 12: view.SetDialogText("password\n工作"); view.ConfirmDialog(); break;
                case 13:
                    Require(!view.HasDialog && view.Rows.Single().Summary.IsSensitive && view.Detail == "" && view.CanReveal,
                        "Sensitive content leaked after tagging.");
                    Require(!view.Rows.Single().Preview.Contains("修改后"), "Sensitive row leaked text.");
                    view.Invoke("reveal");
                    break;
                case 14:
                    Require(Lines(view.Detail) == Lines(Edited), "Explicit reveal failed.");
                    view.Search("不存在"); view.Search("%_");
                    break;
                case 15:
                    Require(view.Rows.Count == 1 && view.Rows[0].Id == entryId, "Debounced literal search returned stale data.");
                    Require(view.Detail == "", "Search kept sensitive text revealed.");
                    for (int index = 0; index < 240; index++) await core.AddManualAsync($"虚拟化记录 {index:D4}");
                    view.Search("");
                    break;
                case 16:
                    Require(view.Rows.Count >= 50 && view.Rows.Count < 241, "Initial page did not stay bounded.");
                    view.ScrollToEnd();
                    break;
                case 17:
                    if (view.Rows.Count < 200) { view.Invoke("more"); return; }
                    loaded = view.Rows.Count; realized = view.RealizedCount;
                    Require(realized > 0 && realized < loaded / 2, "List virtualization did not constrain realized containers.");
                    view.Select(entryId); view.Invoke("delete");
                    break;
                case 18:
                    Require(view.HasDialog, "Delete did not request confirmation."); view.ConfirmDialog();
                    break;
                case 19:
                    Require(!view.HasDialog && view.Rows.All(row => row.Id != entryId), "Delete left stale selection/list.");
                    Require((await core.GetHistoryAsync(new(Search: "%_"))).Items.Count == 0, "Delete did not persist.");
                    entryId = (await core.AddManualAsync(Original, tags: ["work"])).Id;
                    await view.ReloadAsync(entryId);
                    break;
                case 20:
                    Require(Lines(view.Detail) == Lines(Original), "Original CRLF did not display correctly.");
                    view.Invoke("edit"); break;
                case 21: view.ConfirmDialog(); break;
                case 22:
                    Require((await core.GetEntryAsync(entryId)).Content == Original, "Unchanged editor rewrote original CRLF.");
                    view.Filter(1, "work", false); break;
                case 23:
                    Require(view.Rows.Count == 1 && view.Rows[0].Id == entryId, "Native type/tag filter failed.");
                    view.Filter(4, "work", false); break;
                case 24:
                    Require(view.Rows.Count == 0, "Empty native filtered list retained stale items.");
                    view.Filter(0, "", false); break;
                case 25: view.Select(entryId); break;
                case 26:
                    Require(Lines(view.Detail) == Lines(Original), "Selection lost original text.");
                    await core.AddManualAsync("变更流新增"); break;
                case 27:
                    if (!view.ExternalPending) return;
                    Require(Lines(view.Detail) == Lines(Original), "Unrelated change reset active detail.");
                    await core.SetTagsAsync(entryId, ["password"]); break;
                case 28:
                    if (!view.Rows.First(row => row.Id == entryId).IsStale) return;
                    Require(view.Detail == "",
                        "Change feed left a stale plaintext detail/preview visible.");
                    await view.ReloadAsync(entryId); break;
                case 29:
                    Require(view.Detail == "" && view.CanReveal, "Refresh did not enforce new sensitive tag.");
                    view.Invoke("capture");
                    break;
                case 30:
                    Require(view.CaptureRunning && view.CaptureStarts == 1, "Capture control did not start native service.");
                    main.OpenSettings();
                    captureSettingsRevision = core.Snapshot.Revision;
                    break;
                case 31:
                    Require(main.Settings!.CaptureOptionsLoaded, "Capture options lost after reopening.");
                    main.Settings.SetSmokeLimit("-1");
                    break;
                case 32: main.Settings!.InvokeSettingsSave(); break;
                case 33:
                    Require(main.Settings!.HasError && main.Settings.DraftLimit == "-1", "Rejected capture policy lost its draft.");
                    Require(core.Snapshot.Revision == captureSettingsRevision && core.Snapshot.Values["app.persistent_limit"] == "500", "Invalid capture policy changed accepted state.");
                    main.Settings.SetSmokeLimit("501");
                    break;
                case 34: main.Settings!.InvokeSettingsSave(); break;
                case 35:
                    Require(!main.Settings!.HasError && main.Settings.DraftLimit == "501" && core.Snapshot.Values["app.persistent_limit"] == "501"
                        && core.Snapshot.Revision == captureSettingsRevision + 1, "Capture policy correction did not save.");
                    main.CloseSettings();
                    break;
                case 36:
                    Require(view.CaptureRunning && view.CaptureStarts == 1, "Settings lifetime restarted or stopped capture.");
                    Require((await core.GetCaptureStatusAsync()).PolicyRevision == core.Snapshot.Revision, "Worker policy revision not acknowledged.");
                    view.Invoke("capture");
                    break;
                case 37:
                    Require(!view.CaptureRunning, "Pause did not stop and join native capture.");
                    view.Invoke("capture");
                    break;
                case 38:
                    Require(view.CaptureRunning && view.CaptureStarts == 2, "Capture restart failed.");
                    captureStarts = view.CaptureStarts;
                    // Close with capture running and a pending read to exercise disposal guards.
                    view.Search("关闭时取消");
                    var modules = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(m => m.ModuleName);
                    var assemblies = AppDomain.CurrentDomain.GetAssemblies().Select(a => a.GetName().Name ?? "");
                    webView2ModuleLoaded = modules.Concat(assemblies).Any(m => m.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
                    Require(webView2ModuleLoaded == false, "WebView2 module loaded.");
                    Finish(true, null); return;
            }
            phase++;
        }
        catch (Exception error) { Finish(false, $"Phase {phase}: {error}"); }
        finally { ticking = false; }
    }

    private void Finish(bool passed, string? error)
    {
        timer!.Stop();
        ulong revision = core.Snapshot.Revision;
        try { main.Close(); Directory.Delete(directory, recursive: true); }
        catch (Exception failure) { passed = false; error = failure.ToString(); }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, phases = phase + 1,
            framework = "WinUI 3", revision, webView2ModuleLoaded, captureStarts, loaded, realized,
            elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
