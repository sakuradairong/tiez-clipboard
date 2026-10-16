using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using TieZ.Core;
using Windows.Graphics;
using Microsoft.UI.Windowing;

namespace TieZ.Native;

// Real native controls and DLL, with a disposable isolated database. No production data.
internal sealed class WindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private DispatcherQueueTimer? timer;
    private int phase;
    private bool ticking;
    private HotkeyProbe? hotkeyProbe;
    private long entryId, invalidImageId, pasteGuardId, batchFirst, batchSecond, batchKept;
    private ulong batchRevision;
    private long clearPinned, clearTagged, clearOrdinary, clearLate;
    private ulong clearRevision;
    private long imageFirst, imageSecond, documentId, compactTextId, compactDocumentId;
    private TimeSpan compactRequestedAt;
    private PointInt32 compactMainPosition;
    private bool compactValidated, compactFocusPreserved;
    private string compactInvalidation = "none", compactActivation = "none";
    private AdvancedSettingsWindow? advancedWindow;
    private ulong advancedRevision;
    private const string OriginalAdvancedRules = "first => one\r\nsecond => two";
    private const string CompactText = "  紧凑文本\r\n  ";
    private const string DocumentHtml = "<h2>原生文档</h2><p><b>bold</b> &amp; 中文 <i>italic</i> <a href='https://example.invalid/native'>link</a></p><table><tr><th rowspan=2>H<th colspan=2>top<tr><td>one<td>two</table><img src='https://example.invalid/no-request' alt='logo'><script>never-render-script</script>";
    private const string PreviewPng = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAYAAAC56t6BAAAAEUlEQVR4nGP4z8DQAMIMGAwAfcEI+4thakwAAAAASUVORK5CYII=";
    private const string SecondPreviewPng = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAQAAAACCAYAAAB/qH1jAAAADklEQVR4nGNg+I8G0QUABykP8RHZtOEAAAAASUVORK5CYII=";
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
        main.History.PreviewInvalidated += OnPreviewInvalidated;
        main.Activated += OnSmokeActivation;
        timer = main.DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(500);
        timer.Tick += Tick;
        timer.Start();
    }

    private void OnPreviewInvalidated() => compactInvalidation = $"phase={phase}: {new StackTrace(1, false)}";
    private void OnSmokeActivation(object sender, WindowActivatedEventArgs args) =>
        compactActivation = $"phase={phase}: {args.WindowActivationState}";

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
            Require(elapsed.Elapsed < TimeSpan.FromSeconds(90), $"Timed out in phase {phase}.");
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
                    Require(view.CanCopy && !view.HasImagePreview && view.Detail == "图片预览不可用，原始内容未修改。", "Copy command did not bind to loaded detail.");
                    view.Invoke("copy");
                    break;
                case 40:
                    Require(view.HasError && view.CanCopy && !view.HasImagePreview && view.Detail == "图片预览不可用，原始内容未修改。", "Failed payload preparation lost selection or hid error.");
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
                    batchFirst = (await core.AddManualAsync("多选删除一")).Id;
                    batchSecond = (await core.AddManualAsync("多选删除二")).Id;
                    batchKept = (await core.AddManualAsync("多选保留")).Id;
                    await view.ReloadAsync();
                    batchRevision = (await core.GetHistoryAsync(new())).Revision;
                    view.SelectMany(batchFirst, batchSecond);
                    Require(view.Detail == "" && !view.CanCopy && !view.CanPaste && view.CanDelete,
                        "Multiple selection exposed single-entry content/actions.");
                    view.Invoke("delete");
                    break;
                case 54:
                    Require(view.HasDialog, "Batch delete skipped confirmation.");
                    view.CancelDialog();
                    break;
                case 55:
                    Require(!view.HasDialog && (await core.GetHistoryAsync(new())).Revision == batchRevision,
                        "Cancelling batch changed history.");
                    Require((await core.GetEntryAsync(batchFirst)).Content == "多选删除一", "Cancelled batch removed an entry.");
                    view.Invoke("delete");
                    break;
                case 56:
                    Require(view.HasDialog, "Second batch confirmation missing."); view.ConfirmDialog();
                    break;
                case 57:
                    Require(!view.HasDialog && view.Rows.All(row => row.Id != batchFirst && row.Id != batchSecond)
                        && !view.CanDelete, "Accepted batch left stale rows or selection.");
                    var batchChanges = await core.GetHistoryChangesAsync(batchRevision);
                    Require(batchChanges.Revision == batchRevision + 1 && batchChanges.Changes.Count == 1
                        && batchChanges.Changes[0].RemovedIds.Order().SequenceEqual(new[] { batchFirst, batchSecond }),
                        "Window batch did not produce one complete removal event.");
                    Require((await core.GetEntryAsync(batchKept)).Content == "多选保留", "Batch removed an unselected entry.");
                    batchFirst = (await core.AddManualAsync("过期选择")).Id;
                    await view.ReloadAsync(); view.SelectMany(batchFirst, batchKept); view.Invoke("delete");
                    break;
                case 58:
                    Require(view.HasDialog, "Stale-selection confirmation missing.");
                    await core.EditEntryAsync(batchKept, "外部更新保留");
                    view.ConfirmDialog();
                    break;
                case 59:
                    Require(!view.HasDialog && view.HasError, "Changed history was not rejected during confirmation.");
                    Require((await core.GetEntryAsync(batchFirst)).Content == "过期选择"
                        && (await core.GetEntryAsync(batchKept)).Content == "外部更新保留", "Rejected batch partially deleted history.");
                    await view.ReloadAsync(); view.SelectMany(batchFirst, batchKept);
                    view.Search("多选保留");
                    Require(!view.CanDelete && !view.CanCopy && view.Detail == "", "Filter change kept multi-selection actions/content.");
                    break;
                case 60:
                    clearPinned = (await core.AddManualAsync("清空保留置顶")).Id;
                    await core.SetPinnedAsync(clearPinned, true);
                    clearTagged = (await core.AddManualAsync("清空保留标签", tags: ["保留"])).Id;
                    clearOrdinary = (await core.AddManualAsync("筛选范围外的普通历史")).Id;
                    await view.ReloadAsync();
                    Require(view.Rows.Count == 0, "Clear fixture should have an empty filtered list.");
                    var clearPreview = await core.GetClearHistoryPreviewAsync();
                    clearRevision = clearPreview.Revision;
                    Require(clearPreview.EligibleCount > 0, "Clear preview incorrectly followed the UI filter.");
                    view.Invoke("clear");
                    break;
                case 61:
                    Require(view.HasDialog, "Clear skipped confirmation."); view.CancelDialog();
                    break;
                case 62:
                    Require(!view.HasDialog && (await core.GetHistoryAsync(new())).Revision == clearRevision,
                        "Cancelling clear changed history.");
                    Require((await core.GetEntryAsync(clearOrdinary)).Content == "筛选范围外的普通历史", "Cancel removed history.");
                    view.Invoke("clear");
                    break;
                case 63:
                    Require(view.HasDialog, "Clear preview confirmation missing.");
                    clearLate = (await core.AddManualAsync("清空预览期间新增")).Id;
                    view.ConfirmDialog();
                    break;
                case 64:
                    Require(!view.HasDialog && view.HasError, "Changed preview was not rejected.");
                    Require((await core.GetEntryAsync(clearOrdinary)).Content == "筛选范围外的普通历史"
                        && (await core.GetEntryAsync(clearLate)).Content == "清空预览期间新增", "Rejected clear removed history.");
                    await view.ReloadAsync(); view.Invoke("clear");
                    break;
                case 65:
                    Require(view.HasDialog, "Corrected clear confirmation missing.");
                    clearRevision = (await core.GetClearHistoryPreviewAsync()).Revision;
                    view.ConfirmDialog();
                    break;
                case 66:
                    Require(!view.HasDialog && (await core.GetClearHistoryPreviewAsync()).EligibleCount == 0,
                        "Accepted clear left unprotected history.");
                    Require((await core.GetEntryAsync(clearPinned)).IsPinned
                        && (await core.GetEntryAsync(clearTagged)).Tags.Contains("保留"), "Clear removed protected records.");
                    var clearedChanges = await core.GetHistoryChangesAsync(clearRevision);
                    Require(clearedChanges.Revision == clearRevision + 1 && clearedChanges.ResyncRequired
                        && clearedChanges.Changes.Count == 0, "Clear did not create a bounded resync boundary.");
                    clearRevision = clearedChanges.Revision;
                    view.Invoke("clear");
                    break;
                case 67:
                    Require(!view.HasDialog && !view.HasError && (await core.GetHistoryAsync(new())).Revision == clearRevision,
                        "Empty clear showed a confirmation/error or advanced history revision.");
                    imageFirst = (await core.AddManualAsync(PreviewPng, "image")).Id;
                    imageSecond = (await core.AddManualAsync(SecondPreviewPng, "image")).Id;
                    view.Search(""); await view.ReloadAsync(imageFirst);
                    break;
                case 68:
                    if (!view.HasImagePreview) return;
                    Require(view.PreviewedImageId == imageFirst && view.ImagePixelWidth == 2 && view.CanCopy,
                        "WinUI did not decode/render the selected native PNG.");
                    view.SelectMany(imageFirst, imageSecond);
                    Require(!view.HasImagePreview && !view.CanCopy, "Multiple selection retained image preview.");
                    view.Select(imageSecond);
                    break;
                case 69:
                    if (!view.HasImagePreview) return;
                    Require(view.PreviewedImageId == imageSecond && view.ImagePixelWidth == 4, "Previous image decode replaced the new selection.");
                    await core.SetTagsAsync(imageSecond, ["sensitive"]);
                    break;
                case 70:
                    if (!view.ExternalPending) return;
                    Require(!view.HasImagePreview && !view.CanCopy, "External sensitive transition kept decoded image visible.");
                    await view.ReloadAsync(imageSecond);
                    break;
                case 71:
                    Require(view.CanReveal && !view.HasImagePreview, "Sensitive image was rendered before reveal.");
                    view.Invoke("reveal");
                    break;
                case 72:
                    if (!view.HasImagePreview) return;
                    Require(view.PreviewedImageId == imageSecond && view.ImagePixelWidth == 4, "Sensitive reveal did not decode native image.");
                    view.Search("不存在的图片");
                    Require(!view.HasImagePreview && !view.CanCopy, "Search retained revealed image.");
                    break;
                case 73:
                    view.Search(""); await view.ReloadAsync();
                    // Start two native detail loads back-to-back; only the final selection may publish.
                    view.Select(imageFirst); view.Select(imageSecond);
                    break;
                case 74:
                    Require(view.CanReveal && !view.HasImagePreview && !view.CanCopy, "Stale decode bypassed final sensitive selection.");
                    documentId = (await core.AddManualAsync("原始文档\r\n文本", "rich_text", DocumentHtml)).Id;
                    await view.ReloadAsync(documentId);
                    break;
                case 75:
                    if (!view.HasDocumentPreview) return;
                    Require(view.PreviewedDocumentId == documentId && view.DocumentCells == 4 && view.DocumentMergedCells == 2
                        && view.DocumentEmphasis >= 3 && view.DocumentLinks == 1 && view.DocumentOmittedMedia
                        && view.DocumentText.Contains("& 中文") && !view.DocumentText.Contains("never-render-script"),
                        "Native rich document/table was not rendered correctly.");
                    Require((await core.GetEntryAsync(documentId)).HtmlContent == DocumentHtml, "Preview changed original rich formats.");
                    view.SelectMany(documentId, imageFirst);
                    Require(!view.HasDocumentPreview && !view.HasImagePreview && !view.CanCopy, "Multi-selection kept native document visible.");
                    view.Select(documentId);
                    break;
                case 76:
                    if (!view.HasDocumentPreview) return;
                    await core.SetTagsAsync(documentId, ["sensitive"]);
                    break;
                case 77:
                    if (!view.ExternalPending) return;
                    Require(!view.HasDocumentPreview && !view.CanCopy, "Sensitive transition retained native document.");
                    await view.ReloadAsync(documentId);
                    break;
                case 78:
                    Require(view.CanReveal && !view.HasDocumentPreview, "Sensitive document rendered before reveal.");
                    view.Invoke("reveal");
                    break;
                case 79:
                    if (!view.HasDocumentPreview) return;
                    Require(view.PreviewedDocumentId == documentId && view.DocumentCells == 4, "Sensitive document reveal failed.");
                    view.Search("不存在的文档");
                    Require(!view.HasDocumentPreview && !view.CanCopy, "Search retained native document.");
                    break;
                case 80:
                    view.Search(""); await view.ReloadAsync(documentId);
                    view.Invoke("reveal"); view.Select(imageFirst);
                    break;
                case 81:
                    if (!view.HasImagePreview) return;
                    Require(!view.HasDocumentPreview && view.PreviewedImageId == imageFirst, "Stale document replaced final image selection.");
                    view.Select(documentId);
                    break;
                case 82:
                    Require(view.CanReveal && !view.HasDocumentPreview && !view.HasImagePreview && !view.CanCopy,
                        "Stale native preview bypassed sensitive final selection.");
                    CheckCompactPlacement();
                    compactTextId = (await core.AddManualAsync(CompactText)).Id;
                    compactDocumentId = (await core.AddManualAsync("紧凑文档", "rich_text", DocumentHtml)).Id;
                    await view.ReloadAsync(compactTextId);
                    Require(!view.CompactMode && main.CompactPreview.WindowsCreated == 0, "Compact mode must default off without a hidden window.");
                    main.OpenSettings();
                    break;
                case 83:
                    Require(!main.Settings!.CompactMode && !main.Settings.HasDraft, "Opening compact settings wrote a draft.");
                    main.Settings.SetCompactMode(true);
                    Require(main.Settings.HasDraft && !view.CompactMode, "Compact draft applied before saving.");
                    main.Settings.InvokeSettingsSave();
                    Require(view.CompactMode && core.Snapshot.Values["app.compact_mode"] == "true", "Compact save did not synchronize main.");
                    main.CloseSettings(); main.ShowMain();
                    break;
                case 84:
                    compactRequestedAt = elapsed.Elapsed;
                    view.Invoke("preview");
                    Require(main.CompactPreview.HasPending && !main.CompactPreview.Visible, "Preview bypassed the hover delay.");
                    view.CancelPreview();
                    Require(!main.CompactPreview.HasPending && main.CompactPreview.DisplayedId is null, "Cancellation retained a delayed request.");
                    view.Invoke("preview");
                    break;
                case 85:
                    Require(main.CompactPreview.HasPending || main.CompactPreview.Visible, "Native text preview stopped before display.");
                    if (!main.CompactPreview.Visible) return;
                    Require(elapsed.Elapsed - compactRequestedAt >= TimeSpan.FromMilliseconds(900), "Compact preview did not wait one second.");
                    Require(main.CompactPreview.DisplayedId == compactTextId,
                        $"Compact text ID mismatch: expected={compactTextId}, actual={main.CompactPreview.DisplayedId}.");
                    Require(Lines(main.CompactPreview.Text) == Lines(CompactText), "Compact text content mismatch.");
                    Require(main.CompactPreview.IsNonActivating, "Compact preview lost non-activation styles.");
                    Require(main.CompactPreview.ShowPreservedForeground,
                        $"Native show changed foreground or no foreground was available: {main.CompactPreview.FocusDiagnostic}.");
                    compactFocusPreserved = true;
                    Require(main.CompactPreview.WindowsCreated == 1, "First hover created duplicate windows.");
                    var previewBounds = main.CompactPreview.Bounds!.Value;
                    var previewWork = DisplayArea.GetFromRect(previewBounds, DisplayAreaFallback.Nearest).WorkArea;
                    Require(Within(previewBounds, previewWork), "Native preview escaped monitor work area.");
                    view.PreviewRow(imageFirst);
                    Require(!main.CompactPreview.Visible && main.CompactPreview.DisplayedId is null && main.CompactPreview.Text == "",
                        "Changing hover retained old text.");
                    break;
                case 86:
                    Require(main.CompactPreview.HasPending || main.CompactPreview.Visible, "Image hover failed.");
                    if (!main.CompactPreview.Visible) return;
                    Require(main.CompactPreview.HasImage && main.CompactPreview.DisplayedId == imageFirst
                        && main.CompactPreview.WindowsCreated == 1, "Native image preview was not reused.");
                    view.Search("取消紧凑预览");
                    Require(!main.CompactPreview.Visible && !main.CompactPreview.HasImage && main.CompactPreview.DisplayedId is null,
                        "Search retained native compact image.");
                    break;
                case 87:
                    view.Search(""); await view.ReloadAsync(compactDocumentId);
                    view.Invoke("preview");
                    break;
                case 88:
                    Require(main.CompactPreview.HasPending || main.CompactPreview.Visible, "Document hover failed.");
                    if (!main.CompactPreview.Visible) return;
                    Require(main.CompactPreview.HasDocument && main.CompactPreview.WindowsCreated == 1, "Compact table preview did not reuse the native window.");
                    await core.SetTagsAsync(compactDocumentId, ["sensitive"]);
                    break;
                case 89:
                    if (!view.ExternalPending) return;
                    Require(!main.CompactPreview.Visible && !main.CompactPreview.HasDocument && main.CompactPreview.DisplayedId is null,
                        "External sensitive transition retained compact document.");
                    await view.ReloadAsync(compactDocumentId);
                    break;
                case 90:
                    view.PreviewRow(compactDocumentId);
                    Require(view.CanReveal && !main.CompactPreview.HasPending && !main.CompactPreview.Visible,
                        "Unrevealed sensitive row requested a hover preview.");
                    view.Invoke("reveal");
                    break;
                case 91:
                    if (!view.HasDocumentPreview) return;
                    view.Invoke("preview");
                    break;
                case 92:
                    Require(main.CompactPreview.HasPending || main.CompactPreview.Visible, "Revealed hover failed.");
                    if (!main.CompactPreview.Visible) return;
                    Require(main.CompactPreview.DisplayedId == compactDocumentId && main.CompactPreview.HasDocument,
                        "Revealed sensitive document did not display.");
                    core.Update(new Dictionary<string, string> { ["app.color_mode"] = "dark" }, core.Snapshot.Revision);
                    break;
                case 93:
                    Require(main.CompactPreview.Visible && main.CompactPreview.CurrentTheme == ElementTheme.Dark,
                        "Compact preview did not apply shared theme.");
                    main.HideToTray();
                    Require(!main.CompactPreview.Visible && !main.CompactPreview.HasDocument && main.CompactPreview.Text == "",
                        "Hidden main retained sensitive compact data.");
                    main.ShowMain(); await view.ReloadAsync(compactTextId);
                    view.Invoke("preview");
                    break;
                case 94:
                    view.CancelPreview();
                    Require(!main.CompactPreview.HasPending && !main.CompactPreview.Visible, "Pending image survived cancellation.");
                    main.OpenSettings();
                    Require(main.Settings!.CompactMode && !main.Settings.HasDraft, "Reopened settings lost compact mode or wrote defaults.");
                    main.Settings.SetCompactMode(false); main.Settings.InvokeSettingsSave();
                    main.CloseSettings(); main.ShowMain();
                    break;
                case 95:
                    Require(!view.CompactMode && !main.CompactPreview.HasPending && !main.CompactPreview.Visible,
                        "Disabling compact mode retained hover.");
                    view.PreviewRow(compactTextId);
                    Require(!main.CompactPreview.HasPending, "Disabled compact mode accepted preview requests.");
                    core.Update(new Dictionary<string, string> { ["app.compact_mode"] = "true" }, core.Snapshot.Revision);
                    await view.ReloadAsync(compactTextId);
                    break;
                case 96:
                    view.Invoke("preview");
                    break;
                case 97:
                    Require(main.CompactPreview.HasPending || main.CompactPreview.Visible, "Reenabled preview failed.");
                    if (!main.CompactPreview.Visible) return;
                    compactMainPosition = main.AppWindow.Position;
                    main.AppWindow.Move(new PointInt32(compactMainPosition.X + 10, compactMainPosition.Y));
                    break;
                case 98:
                    Require(!main.CompactPreview.Visible && main.CompactPreview.Text == "", "Owner movement retained detached preview.");
                    main.AppWindow.Move(compactMainPosition);
                    break;
                case 99:
                    view.PreviewRow(imageFirst); view.PreviewRow(compactTextId);
                    break;
                case 100:
                    Require(main.CompactPreview.HasPending || main.CompactPreview.Visible, "Last hover request was lost.");
                    if (!main.CompactPreview.Visible) return;
                    Require(main.CompactPreview.DisplayedId == compactTextId && !main.CompactPreview.HasImage
                        && !main.CompactPreview.HasDocument && main.CompactPreview.WindowsCreated == 1,
                        "Old hover replaced final text or leaked another window.");
                    Require(PostMessageW(main.CompactPreview.WindowHandle, 0x0010, 0, 0), "Native preview close message failed.");
                    break;
                case 101:
                    Require(main.CompactPreview.WindowHandle == 0 && !main.CompactPreview.Visible, "Closed preview was retained.");
                    view.PreviewRow(compactTextId);
                    Require(main.CompactPreview.HasPending, $"Reopen request was rejected immediately; {compactInvalidation}; {compactActivation}");
                    break;
                case 102:
                    Require(main.CompactPreview.HasPending || main.CompactPreview.Visible, $"Closed preview could not reopen; {compactInvalidation}; {compactActivation}");
                    if (!main.CompactPreview.Visible) return;
                    Require(main.CompactPreview.WindowsCreated == 2 && main.CompactPreview.DisplayedId == compactTextId
                        && (await core.GetCaptureStatusAsync()).Starts == 2 && (await core.GetTargetStatusAsync()).Starts == 2,
                        "Reopened preview duplicated windows or core services.");
                    compactValidated = true;
                    core.Update(new Dictionary<string, string> { ["app.cleanup_rules"] = OriginalAdvancedRules }, core.Snapshot.Revision);
                    advancedRevision = core.Snapshot.Revision;
                    main.OpenSettings(); main.Settings!.OpenAdvanced();
                    break;
                case 103:
                    if (main.Settings?.IsContentReady != true || main.AdvancedSettings?.IsContentReady != true) return;
                    advancedWindow = main.AdvancedSettings;
                    Require(!advancedWindow.HasDraft && advancedWindow.DisplayedRevision == advancedRevision
                        && core.Snapshot.Revision == advancedRevision, "Advanced window initialization wrote settings.");
                    main.Settings.OpenAdvanced();
                    Require(ReferenceEquals(advancedWindow, main.AdvancedSettings), "Repeated advanced open duplicated the window.");
                    advancedWindow.SetRules("email", advancedWindow.PrivacyDraft, advancedWindow.CleanupDraft, advancedWindow.PolicyDraft);
                    Require(advancedWindow.HasDraft, "Advanced rules did not create an independent draft.");
                    advancedWindow.SetRules(core.Snapshot.Values["app.privacy_protection_kinds"], advancedWindow.PrivacyDraft,
                        advancedWindow.CleanupDraft, advancedWindow.PolicyDraft);
                    Require(!advancedWindow.HasDraft && core.Snapshot.Values["app.cleanup_rules"] == OriginalAdvancedRules,
                        "Unchanged advanced TextBox line endings created a draft or changed storage.");
                    main.Settings.SetSmokeHotkey("Ctrl++");
                    advancedWindow.SetRules("email", "", "[( => broken", "[]");
                    advancedWindow.InvokeSave();
                    Require(advancedWindow.HasError && advancedWindow.HasDraft && advancedWindow.CleanupDraft == "[( => broken"
                        && core.Snapshot.Revision == advancedRevision && core.Snapshot.Values["app.cleanup_rules"] == OriginalAdvancedRules,
                        "Invalid advanced cleanup was saved or its draft was lost.");
                    break;
                case 104:
                    advancedWindow!.SetRules("email", "secret\\d+", "foo => bar", "not JSON");
                    advancedWindow.InvokeSave();
                    Require(advancedWindow.HasError && advancedWindow.HasDraft && advancedWindow.PolicyDraft == "not JSON"
                        && core.Snapshot.Revision == advancedRevision && core.Snapshot.Values["app.cleanup_rules"] == OriginalAdvancedRules,
                        "Invalid policy caused a partial settings save.");
                    break;
                case 105:
                    advancedWindow!.SetRules("email", "secret\\d+", "foo => bar", "[]");
                    advancedWindow.InvokeSave();
                    Require(!advancedWindow.HasError && !advancedWindow.HasDraft && core.Snapshot.Revision == advancedRevision + 1
                        && core.Snapshot.Values["app.cleanup_rules"] == "foo => bar"
                        && core.Snapshot.Values["app.privacy_protection_custom_rules"] == "secret\\d+",
                        "Accepted advanced correction failed to save.");
                    advancedRevision = core.Snapshot.Revision;
                    Require(main.Settings!.HasDraft && main.Settings.HasError && main.Settings.DraftHotkey == "Ctrl++",
                        "Advanced save overwrote the ordinary settings draft.");
                    main.Settings.InvokeSettingsSave();
                    Require(main.Settings.HasError && main.Settings.HasDraft && core.Snapshot.Revision == advancedRevision,
                        "Conflicted ordinary draft overwrote advanced rules.");
                    break;
                case 106:
                    advancedWindow!.SetRules("phone", advancedWindow.PrivacyDraft, advancedWindow.CleanupDraft, advancedWindow.PolicyDraft);
                    core.Update(new Dictionary<string, string> { ["app.color_mode"] = "light" }, core.Snapshot.Revision);
                    advancedRevision = core.Snapshot.Revision;
                    Require(advancedWindow.HasDraft && advancedWindow.HasError && advancedWindow.KindsDraft == "phone"
                        && advancedWindow.CurrentTheme == ElementTheme.Light, "External snapshot lost advanced draft or theme.");
                    Require(main.Settings!.CurrentTheme == ElementTheme.Light && main.Settings.HasDraft
                        && main.Settings.DraftHotkey == "Ctrl++", "Ordinary settings did not follow accepted theme while preserving draft.");
                    main.CloseSettings();
                    Require(main.Settings is null && ReferenceEquals(main.AdvancedSettings, advancedWindow)
                        && (await core.GetCaptureStatusAsync()) is { Running: true, Starts: 2 },
                        "Closing normal settings closed advanced settings or restarted capture.");
                    advancedWindow.InvokeSave();
                    Require(advancedWindow.HasDraft && advancedWindow.HasError && core.Snapshot.Revision == advancedRevision
                        && core.Snapshot.Values["app.privacy_protection_kinds"] == "email", "Stale advanced draft was saved.");
                    break;
                case 107:
                    advancedWindow!.InvokeReload();
                    Require(!advancedWindow.HasDraft && !advancedWindow.HasError && advancedWindow.KindsDraft == "email"
                        && advancedWindow.DisplayedRevision == advancedRevision, "Advanced reload did not discard conflicted draft explicitly.");
                    main.CloseAdvancedSettings();
                    Require(main.AdvancedSettings is null && (await core.GetTargetStatusAsync()) is { Running: true, Starts: 2 },
                        "Closing advanced settings stopped the shared target service.");
                    main.OpenAdvancedSettings();
                    break;
                case 108:
                    if (main.AdvancedSettings?.IsContentReady != true) return;
                    Require(!ReferenceEquals(advancedWindow, main.AdvancedSettings) && !main.AdvancedSettings.HasDraft
                        && main.AdvancedSettings.DisplayedRevision == advancedRevision && main.AdvancedSettings.CleanupDraft == "foo => bar"
                        && core.Snapshot.Revision == advancedRevision, "Reopened advanced window wrote or lost accepted settings.");
                    advancedWindow = main.AdvancedSettings;
                    main.OpenAdvancedSettings();
                    Require(ReferenceEquals(advancedWindow, main.AdvancedSettings), "Advanced reopen created duplicate subscribers/windows.");
                    main.OpenSettings();
                    break;
                case 109:
                    if (main.Settings?.IsContentReady != true) return;
                    Require(!main.Settings.HasDraft && main.Settings.DisplayedRevision == advancedRevision
                        && !advancedWindow!.HasDraft && core.Snapshot.Revision == advancedRevision,
                        "Reopening ordinary settings wrote defaults or affected advanced drafts.");
                    main.CloseSettings(); main.ShowMain();
                    await view.ReloadAsync(compactTextId);
                    break;
                case 110:
                    Require(main.AdvancedSettings is not null && (await core.GetCaptureStatusAsync()) is { Running: true, Starts: 2 },
                        "Independent advanced lifecycle changed core ownership.");
                    view.Invoke("preview");
                    break;
                case 111:
                    Require(main.CompactPreview.HasPending || main.CompactPreview.Visible, "Quit fixture lost the pending preview.");
                    // Close with advanced settings, capture and a pending preview still active.
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

    private static bool Within(RectInt32 bounds, RectInt32 work) => bounds.Width > 0 && bounds.Height > 0
        && bounds.X >= work.X && bounds.Y >= work.Y && bounds.X + bounds.Width <= work.X + work.Width
        && bounds.Y + bounds.Height <= work.Y + work.Height;
    private static void CheckCompactPlacement()
    {
        foreach (double scale in new[] { 1.0, 1.5, 2.0 })
            foreach (var work in new[] { new RectInt32(0, 0, 1920, 1040), new RectInt32(-1920, -200, 1920, 1080), new RectInt32(0, 0, 180, 160) })
            {
                var owner = new RectInt32(work.X + 30, work.Y + 20, Math.Min(900, work.Width), Math.Min(700, work.Height));
                var anchor = new RectInt32(owner.X + 20, work.Y + work.Height - 20, 200, 40);
                Require(Within(CompactPreviewPlacement.Calculate(owner, anchor, work, scale), work), "Placement failed negative/small/DPI work-area bounds.");
            }
        var normalWork = new RectInt32(0, 0, 1920, 1040);
        Require(CompactPreviewPlacement.Calculate(new(20, 20, 800, 600), new(20, 20, 100, 40), normalWork, 1).X > 820, "Preview did not prefer free right side.");
        Require(CompactPreviewPlacement.Calculate(new(1000, 20, 900, 600), new(1020, 20, 100, 40), normalWork, 1).X < 1000, "Preview did not use free left side.");
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
        main.History.PreviewInvalidated -= OnPreviewInvalidated;
        main.Activated -= OnSmokeActivation;
        ulong revision = core.Snapshot.Revision;
        try { hotkeyProbe?.Dispose(); hotkeyProbe = null; }
        catch (Exception failure) { passed = false; error = failure.ToString(); }
        try
        {
            main.Quit();
            if (passed && compactValidated) Require(!main.CompactPreview.Visible && !main.CompactPreview.HasPending
                && main.CompactPreview.WindowHandle == 0 && main.CompactPreview.Text == ""
                && main.Settings is null && main.AdvancedSettings is null, "Quit retained native windows/resources.");
            Directory.Delete(directory, recursive: true);
        }
        catch (Exception failure) { passed = false; error = failure.ToString(); }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, phases = phase + 1,
            framework = "WinUI 3", revision, webView2ModuleLoaded, captureStarts, loaded, realized,
            compactWindowsCreated = main.CompactPreview.WindowsCreated, compactFocusPreserved,
            elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
