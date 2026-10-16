using System.Diagnostics;
using System.Runtime.InteropServices;
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
    private HotkeyProbe? hotkeyProbe;
    private long entryId, invalidImageId, pasteGuardId;
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
                    Require((await core.PollShellAsync()).Status is { Running: true, Starts: 1, TrayVisible: true }
                        && main.ResidentMode, "Native tray did not register or resident close is unavailable.");
                    Require(!view.CaptureRunning && (await core.GetCaptureStatusAsync()).Starts == 0, "Smoke capture must be explicitly controlled.");
                    Require((await core.GetTargetStatusAsync()) is { Running: true, Starts: 1 }, "Main startup did not own one target hook.");
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
                    Require((await core.GetTargetStatusAsync()).Starts == 1, "Settings lifetime restarted target tracking.");
                    Require((await core.PollShellAsync()).Status.Starts == 1, "Settings lifetime restarted native shell.");
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
                    invalidImageId = (await core.AddManualAsync("data:image/png;base64,bad!", contentType: "image")).Id;
                    await view.ReloadAsync(invalidImageId);
                    break;
                case 39:
                    Require(view.CanCopy && view.Detail == "data:image/png;base64,bad!", "Copy command did not bind to loaded detail.");
                    view.Invoke("copy");
                    break;
                case 40:
                    Require(view.HasError && view.CanCopy && view.Detail == "data:image/png;base64,bad!", "Failed payload preparation lost selection or hid error.");
                    await core.DeleteEntryAsync(invalidImageId);
                    await view.ReloadAsync();
                    break;
                case 41:
                    Require(!view.CanCopy && !view.CanPaste, "Clipboard commands were enabled without loaded detail.");
                    await core.StopTargetTrackingAsync();
                    pasteGuardId = (await core.AddManualAsync("paste guard fixture")).Id;
                    await view.ReloadAsync(pasteGuardId);
                    break;
                case 42:
                    if (view.TargetTrackingRunning) return; // Allow the real polling path to observe stop.
                    Require(view.CanCopy && !view.CanPaste, "Stopped target tracking left paste enabled for a loaded entry.");
                    await core.DeleteEntryAsync(pasteGuardId);
                    await view.ReloadAsync();
                    view.Invoke("target-start");
                    break;
                case 43:
                    if (!view.TargetTrackingRunning) return;
                    Require((await core.GetTargetStatusAsync()).Starts == 2 && !view.CanPaste, "Target restart or empty-selection paste guard failed.");
                    // SC_CLOSE follows the same native close path as the title-bar button.
                    if (!PostMessageW(WinRT.Interop.WindowNative.GetWindowHandle(main), 0x112, 0xF060, 0))
                        throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                    break;
                case 44:
                    Require(!main.AppWindow.IsVisible && main.ResidentMode && view.CaptureRunning, "Native close did not preserve background capture.");
                    Require((await core.PollShellAsync()).Status is { Running: true, Starts: 1, TrayVisible: true }, "Close-to-tray stopped native shell.");
                    main.OpenSettings();
                    break;
                case 45:
                    Require(main.Settings is not null && main.Settings.DisplayedRevision == core.Snapshot.Revision, "Settings did not open independently while main was hidden.");
                    main.CloseSettings(); main.ShowMain();
                    break;
                case 46:
                    Require(main.AppWindow.IsVisible && main.ResidentMode && core.Refresh().Revision == 3, "Show from resident mode failed or wrote settings.");
                    main.OpenSettings();
                    break;
                case 47:
                    if (!main.Settings!.IsContentReady) return;
                    main.Settings.SetSmokeHotkey("Ctrl+");
                    Require(main.Settings.HasDraft, "Text edit did not immediately enable Save.");
                    main.Settings.SetSmokeHotkey(core.Snapshot.Values["app.hotkey"]);
                    Require(!main.Settings.HasDraft, "Returning to accepted text left Save enabled.");
                    main.Settings.SetSmokeHotkey("Ctrl+");
                    Require(main.Settings.HasDraft, "Second text edit did not immediately enable Save.");
                    main.Settings.InvokeSettingsSave();
                    break;
                case 48:
                    Require(main.Settings!.HasError && main.Settings.DraftHotkey == "Ctrl+" && core.Snapshot.Revision == 3, "Invalid shortcut did not preserve draft/revision.");
                    hotkeyProbe = new HotkeyProbe();
                    main.Settings.SetSmokeHotkey("Ctrl+Alt+Shift+F24"); Require(main.Settings.HasDraft, "Text edit did not immediately enable Save."); main.Settings.InvokeSettingsSave();
                    break;
                case 49:
                    Require(main.Settings!.HasError && main.Settings.DraftHotkey == "Ctrl+Alt+Shift+F24" && core.Snapshot.Revision == 3, "Conflicting shortcut changed state or lost draft.");
                    hotkeyProbe!.Dispose(); hotkeyProbe = null;
                    main.Settings.InvokeSettingsSave();
                    break;
                case 50:
                    Require(core.Snapshot.Revision == 4 && core.Snapshot.Values["app.hotkey"] == "Ctrl+Alt+Shift+F24"
                        && (await core.PollShellAsync()).Status.MainHotkeyActive, "Accepted shortcut did not register and persist.");
                    main.CloseSettings(); main.HideToTray(); PostOwnedHotkey();
                    break;
                case 51:
                    if (!main.AppWindow.IsVisible && elapsed.Elapsed.TotalSeconds < 55) return;
                    Require(main.AppWindow.IsVisible && view.CaptureRunning, "Native hotkey event did not show resident main window.");
                    main.OpenSettings();
                    break;
                case 52:
                    if (!main.Settings!.IsContentReady) return;
                    main.Settings.SetSmokeHotkey(""); Require(main.Settings.HasDraft, "Text edit did not immediately enable Save."); main.Settings.InvokeSettingsSave();
                    break;
                case 53:
                    Require(core.Snapshot.Revision == 5 && core.Snapshot.Values["app.hotkey"] == ""
                        && !(await core.PollShellAsync()).Status.MainHotkeyActive, "Disabled shortcut remained registered.");
                    using (var released = new HotkeyProbe()) { }
                    main.CloseSettings();
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

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool PostMessageW(nint window, uint message, nuint wparam, nint lparam);

    private static void PostOwnedHotkey()
    {
        nint found = 0;
        EnumWindows((window, _) =>
        {
            GetWindowThreadProcessId(window, out uint process);
            if (process != (uint)Environment.ProcessId) return true;
            var name = new System.Text.StringBuilder(256);
            GetClassNameW(window, name, name.Capacity);
            if (name.ToString().StartsWith($"TieZNativeShell-{Environment.ProcessId}-", StringComparison.Ordinal)) found = window;
            return true;
        }, 0);
        Require(found != 0, "Owned shell HWND missing.");
        // Smoke startup skips automatic registration; the first accepted binding uses 0x4100.
        // This posts only a notification to our HWND. It injects no physical keyboard input.
        if (!PostMessageW(found, 0x312, 0x4100, (0x87 << 16) | 7))
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
    }
    private sealed class HotkeyProbe : IDisposable
    {
        private bool disposed;
        internal HotkeyProbe()
        {
            if (!RegisterHotKey(0, 0x6401, 7 | 0x4000, 0x87))
                throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), "Owned shortcut conflict fixture unavailable");
        }
        public void Dispose()
        {
            if (disposed) return;
            if (!UnregisterHotKey(0, 0x6401)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            disposed = true;
        }
    }
    private delegate bool EnumWindowProc(nint window, nint data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowProc callback, nint data);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(nint window, out uint process);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassNameW(nint window, System.Text.StringBuilder name, int maximum);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool RegisterHotKey(nint window, int id, uint modifiers, uint key);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool UnregisterHotKey(nint window, int id);

    private void Finish(bool passed, string? error)
    {
        timer!.Stop();
        ulong revision = core.Snapshot.Revision;
        try { hotkeyProbe?.Dispose(); hotkeyProbe = null; }
        catch (Exception failure) { passed = false; error = failure.ToString(); }
        try { main.Quit(); Directory.Delete(directory, recursive: true); }
        catch (Exception failure) { passed = false; error = failure.ToString(); }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, phases = phase + 1,
            framework = "WinUI 3", revision, webView2ModuleLoaded, captureStarts, loaded, realized,
            elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
