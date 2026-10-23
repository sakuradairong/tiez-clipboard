using System.Diagnostics;
using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TieZ.Core;

namespace TieZ.Native;

// Acceptance coverage for the real opener settings editor and the real application picker ContentDialog:
// only actual rendered dialog template buttons, the actual refresh button and the actual settings Save
// button are used. Controlled catalog fixtures never replace those button flows.
internal sealed class ApplicationPickerWindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private const string Opaque = "opaque7f3a9c2ezeta";
    private const string Notepad = @"C:\Windows\notepad.exe";
    private const string Padded = "填充应用 042";
    private const string Zeta = "Zeta 工具";
    private const string Legacy = "Legacy_Browser_123!App";
    private const string Manual = @"C:\manual draft.exe";
    private static readonly string[] Keys = ["app.text", "app.code", "app.rich_text", "app.image", "app.file", "app.video", "app.url", "app.link"];
    private static readonly string[] Types = ["text", "code", "rich_text", "image", "file", "video", "url", "link"];
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private readonly List<string> passes = [];
    private readonly List<string> shutdownProblems = [];
    private SettingsWindow? settings;
    private OpenerSettingsEditor? openers;
    private ApplicationPickerView? lastView;
    private int checks, dialogsShown, confirmations, cancellations, catalogCalls, refreshCalls, job;
    private bool finished, quitVerified, lateIgnored, webView2ModuleLoaded, coreClosed, fixtureDeleted;
    private int realRows, realRealized;
    private bool realVirtualized;

    private void Check(bool value, string message) { checks++; if (!value) throw new InvalidOperationException(message); }
    private void Pass(string message) => passes.Add($"{checks}:{message}");

    private async Task WaitUntil(Func<bool> ready, string message, double seconds = 10)
    {
        var watch = Stopwatch.StartNew();
        while (!ready() && watch.Elapsed < TimeSpan.FromSeconds(seconds)) await Task.Delay(15);
        Check(ready(), message);
    }

    private async Task WaitUnloadedAsync(ContentDialog dialog)
    {
        var watch = Stopwatch.StartNew();
        while (dialog.IsLoaded && watch.Elapsed < TimeSpan.FromSeconds(3)) await Task.Delay(15);
    }

    private static InstalledApplication App(string name, string source, string? handler, string kind, string? unavailable = null)
        => new(name, source, handler, kind, unavailable);

    private static IReadOnlyList<InstalledApplication> AllApps()
    {
        var apps = new List<InstalledApplication>
        {
            App("记事本", "src-notepad", Notepad, "desktop"),
            App(Zeta, Opaque, @"C:\Tools\zeta.exe", "desktop"),
        };
        for (int i = 0; i < 60; i++) apps.Add(App($"填充应用 {i:000}", $"src-pad-{i:000}", $@"C:\Pad\pad{i:000}.exe", "desktop"));
        apps.Add(App("记事本 副本", "src-dup-notepad", Notepad, "desktop"));
        apps.Add(App("损坏程序", "src-broken", @"C:\Missing\broken.exe", "desktop", "missing_target"));
        return apps;
    }

    private static IReadOnlyList<RecommendedApplication> Recommended(string type) =>
    [
        new(App($"推荐A-{type}", $"src-rec-a-{type}", $@"C:\Rec\{type}\a.exe", "desktop"), "association"),
        new(App($"推荐B-{type}", $"src-rec-b-{type}", $"RecB_{type}!App", "packaged"), "keyword")
    ];

    private ApplicationCatalogResult Controlled(bool refresh)
    {
        var apps = AllApps();
        var recommendations = new Dictionary<string, ApplicationRecommendations>();
        foreach (string type in Types) recommendations[type] = new(Recommended(type), null, 0, false, $"默认-{type}");
        return new ApplicationCatalogResult((ulong)++job, refresh, new ApplicationCatalog(apps, apps.Count, 0, 1, false, recommendations));
    }

    private Func<bool, CancellationToken, Task<ApplicationCatalogResult>> ControlledReader() => (refresh, _) =>
    {
        catalogCalls++;
        if (refresh) refreshCalls++;
        return Task.FromResult(Controlled(refresh));
    };

    private static bool Sorted(IEnumerable<string> names)
    {
        string[] values = names.ToArray();
        for (int i = 1; i < values.Length; i++)
            if (StringComparer.CurrentCultureIgnoreCase.Compare(values[i - 1], values[i]) > 0) return false;
        return true;
    }

    private static IEnumerable<DependencyObject> Descendants(DependencyObject node)
    {
        yield return node;
        int count = VisualTreeHelper.GetChildrenCount(node);
        for (int i = 0; i < count; i++)
            foreach (var child in Descendants(VisualTreeHelper.GetChild(node, i))) yield return child;
    }

    // A button only counts when it is loaded, enabled, rendered with real dimensions, every ancestor is
    // visible and laid out, and the element really belongs to the current dialog template.
    private static bool RenderedInDialog(DependencyObject node, ContentDialog dialog)
    {
        if (node is not Button button || !button.IsLoaded || !button.IsEnabled || button.Visibility != Visibility.Visible) return false;
        if (button.ActualWidth <= 0 || button.ActualHeight <= 0) return false;
        for (DependencyObject? current = node; current is not null; current = VisualTreeHelper.GetParent(current))
        {
            if (current is UIElement { Visibility: not Visibility.Visible }) return false;
            if (current is FrameworkElement element && !ReferenceEquals(element, dialog)
                && (element.ActualWidth <= 0 || element.ActualHeight <= 0)) return false;
            if (ReferenceEquals(current, dialog)) return true;
        }
        return false;
    }

    private static List<Button> DialogButtons(ContentDialog dialog, string name) =>
        Descendants(dialog).OfType<Button>().Where(button => button.Name == name && RenderedInDialog(button, dialog)).ToList();

    private sealed class Observation(ContentDialog target)
    {
        internal int Primary, Close;
        internal bool PrimaryFromDialog, CloseFromDialog, Pressed;
        internal Observation Attach()
        {
            target.PrimaryButtonClick += (sender, _) => { Primary++; PrimaryFromDialog |= ReferenceEquals(sender, target); };
            target.CloseButtonClick += (sender, _) => { Close++; CloseFromDialog |= ReferenceEquals(sender, target); };
            return this;
        }
    }

    private async Task PressAsync(ContentDialog dialog, string name, Observation events)
    {
        Check(!events.Pressed, "The same dialog button would have been pressed twice.");
        Button? button = null;
        await WaitUntil(() =>
        {
            var found = DialogButtons(dialog, name);
            if (found.Count > 1) throw new InvalidOperationException($"The current dialog rendered {found.Count} ready {name} buttons.");
            button = found.Count == 1 ? found[0] : null;
            return button is not null;
        }, $"The current dialog never rendered exactly one ready {name} button.", 8);
        Check(button!.IsEnabled, $"The current dialog {name} button was disabled.");
        events.Pressed = true;
        ((IInvokeProvider)new ButtonAutomationPeer(button).GetPattern(PatternInterface.Invoke)).Invoke();
    }

    private async Task WaitClosedAsync(ContentDialog dialog, ApplicationPickerView view, Observation events, bool primary)
    {
        await WaitUntil(() => openers!.ApplicationDialog is null && openers.ActiveApplications is null,
            "The dialog button did not close the application picker.", 8);
        Check(primary
                ? events.Primary == 1 && events.Close == 0 && events.PrimaryFromDialog
                : events.Close == 1 && events.Primary == 0 && events.CloseFromDialog,
            "The dialog did not observe exactly one self-originated button click.");
        Check(view.IsDisposed && !view.HasRetainedData, "The closed picker view retained data.");
        if (primary) confirmations++; else cancellations++;
        await WaitUnloadedAsync(dialog);
    }

    private async Task<SettingsWindow> ShowSettingsAsync()
    {
        main.OpenSettings();
        var shown = main.Settings!;
        shown.ShowOpenerSettings();
        await WaitUntil(() => shown.IsContentReady, "Settings window did not load.");
        return shown;
    }

    private async Task<(ContentDialog Dialog, ApplicationPickerView View)> OpenAsync(string key)
    {
        Check(openers is not null, "The opener editor was not available.");
        openers!.InvokeApplications(key);
        dialogsShown++;
        ContentDialog? dialog = null;
        ApplicationPickerView? view = null;
        bool seen = false;
        await WaitUntil(() =>
        {
            var current = openers.ApplicationDialog;
            var active = openers.ActiveApplications;
            if (current is not null && active is not null) { dialog = current; view = active; seen = true; return true; }
            if (seen) throw new InvalidOperationException($"The picker dialog for {key} closed before it rendered.");
            return false;
        }, $"The actual application picker dialog for {key} did not open.", 10);
        Check(ReferenceEquals(dialog!.Content, view), "The opened dialog no longer hosts the application picker view.");
        await WaitUntil(() => dialog.IsLoaded && dialog.ActualWidth > 0 && dialog.ActualHeight > 0,
            $"The picker dialog for {key} never reached rendered layout.", 10);
        Check(view!.XamlRoot is not null, "The rendered picker had no XamlRoot.");
        lastView = view;
        return (dialog, view);
    }

    private async Task LoadedAsync(ApplicationPickerView view, string message, double seconds = 15)
        => await WaitUntil(() => !view.IsLoading, message, seconds);

    private async Task RowsAsync(ApplicationPickerView view)
        => await WaitUntil(() => view.RealizedCount > 0, "The picker list never realized a visible row.", 5);

    internal async void Start()
    {
        try
        {
            await WaitUntil(() => main.ResidentMode && !main.History.IsBusy, "Main window was not ready.");
            var captureBefore = await core.GetCaptureStatusAsync();
            var targetBefore = await core.GetTargetStatusAsync();
            var lifetime = main.OpenLifetime;
            ulong revision = core.Snapshot.Revision;

            settings = await ShowSettingsAsync();
            openers = settings.Openers;
            Check(Keys.All(key => openers.Draft(key) == core.Snapshot.Values[key]), "Initial eight opener drafts did not bind the accepted values.");
            Check(openers.Draft("app.text").Length == 0, "Fresh isolated opener draft was not empty.");
            Check(!settings.HasDraft && core.Refresh().Revision == revision, "Initial binding saved settings.");

            // 1. The real core catalog through the real settings dialog, canceled without saving.
            openers.CatalogReader = core.GetApplicationCatalogAsync;
            var (realDialog, realView) = await OpenAsync("app.text");
            await LoadedAsync(realView, "The real application catalog did not finish loading.", 45);
            await RowsAsync(realView);
            Check(!realView.IsFailed && !realView.HasError, $"The real catalog load failed: {realView.FeedbackMessage}");
            var real = realView.VisibleChoices;
            realRows = real.Count;
            realRealized = realView.RealizedCount;
            Check(real.Count >= 2, "The real catalog returned no application rows.");
            Check(real[0].GroupLabel == "默认规则" && real[0].CanSelect && real[0].Value == "", "The real catalog lost the default rule row.");
            Check(realView.CanChoose && realView.SelectedValue == "" && realDialog.IsPrimaryButtonEnabled,
                "The empty real draft did not preselect the confirmable default rule row.");
            Check(real.Take(2).All(choice => choice.Details.Length > 0), "A real catalog row had no launch details.");
            if (real.Count > 10)
            {
                Check(realRealized > 0 && realRealized < real.Count, "The real picker list did not virtualize its rows.");
                realVirtualized = true;
            }
            string probe = real[^1].Name;
            realView.SearchDraft = probe;
            var probed = realView.VisibleChoices;
            Check(probed.Count >= 1 && probed.All(choice => choice.SearchText.Contains(probe, StringComparison.OrdinalIgnoreCase)),
                "Searching a real application name did not filter to matching rows.");
            Check(!realView.CanChoose && !realDialog.IsPrimaryButtonEnabled,
                "Filtering away the real default selection kept confirm enabled.");
            realView.SearchDraft = "";
            var realEvents = new Observation(realDialog).Attach();
            await PressAsync(realDialog, "CloseButton", realEvents);
            await WaitClosedAsync(realDialog, realView, realEvents, primary: false);
            Check(openers.Draft("app.text").Length == 0 && core.Refresh().Revision == revision && !settings.HasDraft,
                "Canceling the real picker changed the draft or saved configuration.");
            Pass("real core catalog through the actual dialog: rows, default rule, search gating and actual close-button cancel");

            // 2. Controlled fixture: ordering, duplicate suppression, opaque identity and search semantics.
            openers.CatalogReader = ControlledReader();
            var opaqueOwners = AllApps().Where(app => app.SourceId.Contains(Opaque, StringComparison.OrdinalIgnoreCase)).ToArray();
            Check(opaqueOwners.Length == 1 && opaqueOwners[0].Name == Zeta, "The opaque fixture identity was not unique to its row.");
            Check(AllApps().All(app => !app.Name.Contains(Opaque, StringComparison.OrdinalIgnoreCase)
                && !(app.Handler ?? "").Contains(Opaque, StringComparison.OrdinalIgnoreCase)),
                "The opaque fixture identity leaked into a name or handler.");
            var (dialog, view) = await OpenAsync("app.text");
            await LoadedAsync(view, "The controlled catalog did not finish loading.");
            await RowsAsync(view);
            Check(!view.IsFailed && !view.HasError, "The controlled catalog load reported an error.");
            var rows = view.VisibleChoices;
            Check(rows.Count == 66, $"Controlled catalog built {rows.Count} rows instead of 66.");
            Check(rows[0].GroupLabel == "默认规则" && rows[0].Name == "使用系统默认应用"
                && rows[0].Details == "打开时使用 Windows 当前关联" && rows[0].CanSelect && rows[0].Value == "",
                "Default rule row missing, relabeled or not selectable.");
            Check(rows[1].Name == "推荐A-text" && rows[1].GroupLabel.StartsWith("系统关联推荐", StringComparison.Ordinal), "Association recommendation order lost.");
            Check(rows[2].Name == "推荐B-text" && rows[2].GroupLabel.StartsWith("名称推荐", StringComparison.Ordinal), "Keyword recommendation order lost.");
            Check(rows[^1].Name == "损坏程序" && !rows[^1].CanSelect && rows[^1].GroupLabel == "无法用作打开程序", "Unavailable row was not last or selectable.");
            Check(rows.All(row => row.Name != "记事本 副本"), "Duplicate handler row was not suppressed.");
            Check(rows.Count(row => row.Details == Notepad) == 1, "Duplicate handler produced more than one confirmable row.");
            var others = rows.Where(row => row.Group == "other").Select(row => row.Name).ToArray();
            Check(others.Length == 62, $"Other-application group held {others.Length} rows instead of 62.");
            Check(Sorted(others), "Other-application rows lost their name ordering.");
            Check(view.RealizedCount > 0 && view.RealizedCount < rows.Count, "The picker list did not virtualize 66 rows.");
            Check(view.CanChoose && view.SelectedValue == "" && dialog.IsPrimaryButtonEnabled, "Empty draft did not preselect the default rule row with confirm enabled.");
            view.SearchDraft = Opaque;
            Check(view.VisibleChoices.Count == 1 && view.VisibleChoices[0].Name == Zeta, "Opaque SourceId search missed its row.");
            Check(view.VisibleChoices[0].Details == @"C:\Tools\zeta.exe", "Desktop details no longer show the handler.");
            view.SearchDraft = Opaque.ToUpperInvariant();
            Check(view.VisibleChoices.Count == 1 && view.VisibleChoices[0].Name == Zeta, "SourceId search was not case-insensitive.");
            view.SearchDraft = @"pad042";
            Check(view.VisibleChoices.Count == 1 && view.VisibleChoices[0].Name == Padded, "Handler path search missed its row.");
            view.SearchDraft = @"PAD042";
            Check(view.VisibleChoices.Count == 1 && view.VisibleChoices[0].Name == Padded, "Handler path search was not case-insensitive.");
            view.SearchDraft = @"C:\Tools\zeta.exe";
            Check(view.VisibleChoices.Count == 1 && view.VisibleChoices[0].Name == Zeta, "Full handler path search missed its row.");
            view.SearchDraft = "记事本";
            Check(view.VisibleChoices.Count == 1 && view.VisibleChoices[0].Name == "记事本", "Name search missed its row.");
            view.SearchDraft = "无此应用-9x";
            Check(view.VisibleChoices.Count == 0 && !view.CanChoose && !dialog.IsPrimaryButtonEnabled, "No-match search left rows or confirm enabled.");
            view.SearchDraft = "";
            Check(view.VisibleChoices.Count == 66, "Clearing the search did not restore all rows.");
            view.Filter = 1;
            Check(view.VisibleChoices.Count == 2 && !view.CanChoose && !dialog.IsPrimaryButtonEnabled, "Recommended filter did not isolate rows or left confirm enabled.");
            view.Filter = 2;
            Check(view.VisibleChoices.Count == 62, "Other-application filter count changed.");
            view.Filter = 3;
            Check(view.VisibleChoices.Count == 1, "Unavailable filter lost its row.");
            view.SelectUnavailable();
            Check(!view.CanChoose && view.SelectedValue is null && !dialog.IsPrimaryButtonEnabled, "Unavailable row could be confirmed.");
            view.Filter = 0;
            Check(view.VisibleChoices.Count == 66, "Returning to all rows lost filter results.");
            view.Select(Notepad);
            Check(view.CanChoose && view.SelectedValue == Notepad && dialog.IsPrimaryButtonEnabled, "Selectable row did not enable confirm.");
            Pass("controlled fixture: eight-type recommendations, ordering, duplicate path suppression, opaque identity, path/name search and filters");

            // 3. The actual primary button applies the draft only; an explicit actual Save persists it.
            ulong beforeConfirm = core.Snapshot.Revision;
            var confirmEvents = new Observation(dialog).Attach();
            await PressAsync(dialog, "PrimaryButton", confirmEvents);
            await WaitClosedAsync(dialog, view, confirmEvents, primary: true);
            Check(openers.Draft("app.text") == Notepad, "The actual primary button did not apply the selection to the opener draft.");
            Check(core.Refresh().Revision == beforeConfirm && core.Snapshot.Values["app.text"].Length == 0,
                "Confirming the dialog saved configuration without an explicit Save.");
            Check(settings.HasDraft, "The dialog-applied draft did not enable the actual Save button.");
            Pass("actual primary button confirmed exactly once and updated only the draft");
            ulong beforeSave = core.Snapshot.Revision;
            settings.InvokeSettingsSave();
            Check(core.Snapshot.Revision == beforeSave + 1, "Explicit Save did not advance the settings revision.");
            Check(core.Snapshot.Values["app.text"] == Notepad && !settings.HasDraft && !settings.HasError,
                "Explicit Save did not persist the selected opener exactly once.");
            Check(core.Snapshot.Values["app.code"].Length == 0 && openers.Draft("app.code").Length == 0,
                "The empty code opener no longer follows the text opener rule.");
            revision = core.Snapshot.Revision;
            Pass("actual Save persisted the confirmed opener and kept the empty code fallback");

            // 4. Every one of the eight opener keys maps to its own content-type recommendations and rule.
            foreach (string key in Keys)
            {
                string type = key[4..];
                var (keyDialog, keyView) = await OpenAsync(key);
                await LoadedAsync(keyView, $"The controlled catalog for {key} did not finish loading.");
                await RowsAsync(keyView);
                var keyRows = keyView.VisibleChoices;
                Check(!keyView.IsFailed && !keyView.HasError && keyRows.Count == 66, $"The picker for {key} lost its controlled rows.");
                Check(keyRows[1].Name == $"推荐A-{type}" && keyRows[2].Name == $"推荐B-{type}",
                    $"The picker for {key} did not receive the {type} recommendations.");
                Check(keyRows[0].Name == (type == "code" ? "跟随文本应用" : "使用系统默认应用"),
                    $"The default rule name for {key} changed.");
                Check(keyRows[0].Details == (type == "code" ? "保存后跟随文本打开应用" : "打开时使用 Windows 当前关联"),
                    $"The default rule behavior text for {key} changed.");
                Check(keyView.DefaultTextValue == (type == "code" ? "默认跟随文本应用" : $"Windows 默认：默认-{type}"),
                    $"The default application summary for {key} changed.");
                string expected = openers.Draft(key);
                Check(expected.Length == 0 ? keyView.SelectedValue == "" : keyView.SelectedValue == expected,
                    $"The picker for {key} did not preselect its current draft.");
                var keyEvents = new Observation(keyDialog).Attach();
                await PressAsync(keyDialog, "CloseButton", keyEvents);
                await WaitClosedAsync(keyDialog, keyView, keyEvents, primary: false);
                Check(openers.Draft(key) == expected && core.Refresh().Revision == revision,
                    $"Canceling the picker changed the {key} draft or saved configuration.");
            }
            Pass("all eight opener keys load their own recommendations, rule labels and cancel without saving");

            // 5. Cancel keeps a legacy draft that no catalog row can represent.
            openers.SetDraft("app.url", Legacy);
            Check(settings.HasDraft, "The legacy draft did not enable Save.");
            var (legacyDialog, legacyView) = await OpenAsync("app.url");
            await LoadedAsync(legacyView, "The controlled catalog for the legacy draft did not load.");
            Check(legacyView.SelectedValue is null && !legacyView.CanChoose && !legacyDialog.IsPrimaryButtonEnabled,
                "A legacy draft wrongly matched a catalog row and enabled confirm.");
            var legacyEvents = new Observation(legacyDialog).Attach();
            await PressAsync(legacyDialog, "CloseButton", legacyEvents);
            await WaitClosedAsync(legacyDialog, legacyView, legacyEvents, primary: false);
            Check(openers.Draft("app.url") == Legacy && settings.HasDraft && core.Refresh().Revision == revision,
                "Cancel discarded the legacy draft or saved configuration.");
            settings.InvokeSettingsReload();
            Check(!settings.HasDraft && !settings.HasError && openers.Draft("app.url") == core.Snapshot.Values["app.url"],
                "Reload did not discard the legacy draft after cancel.");
            Pass("cancel preserved a legacy draft and an explicit reload discarded it");

            // 6. Refresh failure keeps rows but blocks confirm; the actual refresh button retries successfully.
            bool failRefresh = false;
            openers.CatalogReader = (refresh, _) =>
            {
                catalogCalls++;
                if (refresh) refreshCalls++;
                return failRefresh
                    ? Task.FromException<ApplicationCatalogResult>(new CoreException("apps_busy", "owned busy fixture"))
                    : Task.FromResult(Controlled(refresh));
            };
            var (refreshDialog, refreshView) = await OpenAsync("app.image");
            await LoadedAsync(refreshView, "The refresh fixture picker did not load.");
            await RowsAsync(refreshView);
            Check(refreshView.VisibleChoices.Count == 66, "The refresh fixture lost its rows.");
            refreshView.Select(Notepad);
            Check(refreshView.CanChoose && refreshDialog.IsPrimaryButtonEnabled, "The refresh fixture selection was not confirmable.");
            int refreshBefore = refreshCalls;
            failRefresh = true;
            refreshView.InvokeRefresh();
            await LoadedAsync(refreshView, "The failing refresh did not finish.");
            Check(refreshView.IsFailed && refreshView.HasError, "The failed refresh was not reported.");
            Check(refreshView.FeedbackMessage.Contains("上一次读取仍在停止", StringComparison.Ordinal), "The apps_busy refresh message was not surfaced.");
            Check(refreshView.VisibleChoices.Count == 66 && refreshView.VisibleChoices.Any(choice => choice.Name == "推荐A-image"),
                "The failed refresh discarded the retained rows.");
            Check(refreshView.SelectedValue is null && !refreshView.CanChoose && !refreshDialog.IsPrimaryButtonEnabled,
                "The failed refresh left confirm enabled.");
            failRefresh = false;
            refreshView.InvokeRefresh();
            await WaitUntil(() => !refreshView.IsLoading && !refreshView.IsFailed && !refreshView.HasError,
                "The explicit refresh retry did not recover from the failure.");
            await RowsAsync(refreshView);
            Check(refreshView.VisibleChoices.Count == 66 && refreshView.CanChoose && refreshView.SelectedValue == Notepad
                && refreshDialog.IsPrimaryButtonEnabled, "The refresh retry did not restore rows, selection and confirm.");
            Check(refreshCalls == refreshBefore + 2, "The actual refresh button did not request exactly two refreshes.");
            var refreshEvents = new Observation(refreshDialog).Attach();
            await PressAsync(refreshDialog, "CloseButton", refreshEvents);
            await WaitClosedAsync(refreshDialog, refreshView, refreshEvents, primary: false);
            Check(openers.Draft("app.image").Length == 0 && core.Refresh().Revision == revision,
                "Canceling after a recovered refresh changed the draft or saved configuration.");
            Pass("refresh failure retained rows and blocked confirm; the actual refresh retry recovered, including apps_busy");

            // 7. A pending read that ignores cancellation cannot update a stale picker, and a manual draft edit invalidates it.
            var gate = new TaskCompletionSource<ApplicationCatalogResult>(TaskCreationOptions.RunContinuationsAsynchronously);
            openers.CatalogReader = (refresh, _) => { catalogCalls++; if (refresh) refreshCalls++; return gate.Task; };
            var (staleDialog, staleView) = await OpenAsync("app.video");
            Check(staleView.IsLoading && staleDialog.IsLoaded, "The delayed fixture picker did not start loading after rendering.");
            openers.SetDraft("app.video", Manual);
            Check(staleView.IsStale && !staleView.IsLoading && !staleView.CanChoose && !staleDialog.IsPrimaryButtonEnabled,
                "A manual draft edit did not invalidate the open picker selection.");
            Check(staleView.HasError && !staleView.IsFailed, "Stale invalidation did not show the warning state.");
            gate.SetResult(Controlled(true));
            await Task.Delay(250);
            Check(staleView.IsStale && staleView.VisibleChoices.Count == 0 && staleView.SelectedValue is null,
                "A late controlled result updated a stale picker.");
            lateIgnored = true;
            var staleEvents = new Observation(staleDialog).Attach();
            await PressAsync(staleDialog, "CloseButton", staleEvents);
            await WaitClosedAsync(staleDialog, staleView, staleEvents, primary: false);
            Check(openers.Draft("app.video") == Manual && core.Refresh().Revision == revision,
                "Cancel after invalidation did not preserve the manual draft.");
            settings.InvokeSettingsReload();
            Check(!settings.HasDraft && !settings.HasError, "Reload after the invalidated draft did not clear it.");
            Pass("manual draft edit invalidated the selection and a late result did not update the stale picker");

            // 8. External revision plus an ordinary draft keeps both drafts and the conflict warning through cancel.
            openers.CatalogReader = ControlledReader();
            var (conflictDialog, conflictView) = await OpenAsync("app.video");
            await LoadedAsync(conflictView, "The conflict fixture picker did not load.");
            await RowsAsync(conflictView);
            Check(conflictView.CanChoose && conflictDialog.IsPrimaryButtonEnabled, "The conflict fixture selection was not confirmable.");
            settings.SetSmokeLimit("777");
            Check(settings.DraftLimit == "777", "The ordinary draft was not accepted while the picker was open.");
            ulong beforeExternal = core.Snapshot.Revision;
            Check(core.Update(new Dictionary<string, string> { ["app.color_mode"] = "dark" }, beforeExternal),
                "The external configuration update was rejected.");
            Check(core.Snapshot.Revision == beforeExternal + 1, "The external configuration update did not advance the revision.");
            revision = core.Snapshot.Revision;
            Check(conflictView.IsStale && !conflictView.CanChoose && !conflictDialog.IsPrimaryButtonEnabled,
                "An external configuration revision did not invalidate the open picker.");
            Check(settings.HasError, "The external configuration conflict warning was not shown.");
            await WaitUntil(() => conflictDialog.RequestedTheme == ElementTheme.Dark,
                "The picker dialog did not follow the settings dialog theme.", 4);
            Check(settings.CurrentTheme == ElementTheme.Dark && main.CurrentTheme == ElementTheme.Dark,
                "The accepted dark theme was not applied while the picker was open.");
            Check(conflictView.SearchDraft.Length == 0 && conflictView.Filter == 0, "The conflict fixture unexpectedly carried search state.");
            var conflictEvents = new Observation(conflictDialog).Attach();
            await PressAsync(conflictDialog, "CloseButton", conflictEvents);
            await WaitClosedAsync(conflictDialog, conflictView, conflictEvents, primary: false);
            Check(settings.HasDraft && settings.DraftLimit == "777" && openers.Draft("app.video").Length == 0,
                "Cancel under conflict discarded the ordinary or opener draft.");
            Check(settings.HasError, "The conflict warning disappeared after cancel.");
            Check(core.Refresh().Revision == revision && core.Snapshot.Values["app.video"].Length == 0,
                "Cancel under conflict saved a draft.");
            settings.InvokeSettingsReload();
            Check(!settings.HasDraft && !settings.HasError && settings.DraftLimit == core.Snapshot.Values["app.persistent_limit"],
                "Reload under conflict did not follow the accepted configuration.");
            Check(ReferenceEquals(lifetime, main.OpenLifetime), "Settings or picker coverage replaced the shared open controller.");
            Check((await core.GetCaptureStatusAsync()).Starts == captureBefore.Starts
                && (await core.GetTargetStatusAsync()).Starts == targetBefore.Starts,
                "Picker coverage restarted shared clipboard or target services.");
            Pass("external revision invalidated the selection and cancel preserved both drafts plus the conflict warning, with dialog theme following settings");

            // 8b. An explicit settings reload invalidates an open picker selection without changing drafts.
            openers.CatalogReader = ControlledReader();
            var (reloadDialog, reloadView) = await OpenAsync("app.link");
            await LoadedAsync(reloadView, "The reload fixture picker did not load.");
            await RowsAsync(reloadView);
            Check(reloadView.CanChoose && reloadDialog.IsPrimaryButtonEnabled, "The reload fixture selection was not confirmable.");
            string reloadDraft = openers.Draft("app.link");
            settings.InvokeSettingsReload();
            Check(reloadView.IsStale && !reloadView.CanChoose && !reloadDialog.IsPrimaryButtonEnabled,
                "An explicit settings reload did not invalidate the open picker selection.");
            var reloadEvents = new Observation(reloadDialog).Attach();
            await PressAsync(reloadDialog, "CloseButton", reloadEvents);
            await WaitClosedAsync(reloadDialog, reloadView, reloadEvents, primary: false);
            Check(openers.Draft("app.link") == reloadDraft && core.Refresh().Revision == revision && !settings.HasDraft,
                "Cancel after a reload invalidation changed the draft or saved configuration.");
            Pass("an explicit settings reload invalidated the open picker selection without changing drafts");

            // 9. A late result must not update a picker disposed by the actual close button.
            gate = new TaskCompletionSource<ApplicationCatalogResult>(TaskCreationOptions.RunContinuationsAsynchronously);
            openers.CatalogReader = (refresh, _) => { catalogCalls++; if (refresh) refreshCalls++; return gate.Task; };
            var (disposedDialog, disposedView) = await OpenAsync("app.link");
            Check(disposedView.IsLoading, "The disposed fixture picker did not start loading.");
            var disposedEvents = new Observation(disposedDialog).Attach();
            await PressAsync(disposedDialog, "CloseButton", disposedEvents);
            await WaitClosedAsync(disposedDialog, disposedView, disposedEvents, primary: false);
            Check(disposedView.IsDisposed && !disposedView.HasRetainedData, "The disposed picker retained data.");
            gate.SetResult(Controlled(false));
            await Task.Delay(250);
            Check(!disposedView.HasRetainedData && disposedView.VisibleChoices.Count == 0,
                "A late controlled result updated the disposed picker.");
            Check(openers.Draft("app.link").Length == 0 && core.Refresh().Revision == revision,
                "The disposed fixture changed the draft or saved configuration.");
            lateIgnored = true;
            Pass("a late result that ignored cancellation did not update the disposed picker");

            // 10. Closing settings while loading clears delegate, drafts and the view without late effects.
            gate = new TaskCompletionSource<ApplicationCatalogResult>(TaskCreationOptions.RunContinuationsAsynchronously);
            openers.CatalogReader = (refresh, _) => { catalogCalls++; if (refresh) refreshCalls++; return gate.Task; };
            var (closingDialog, closingView) = await OpenAsync("app.file");
            var closingEditor = openers;
            main.CloseSettings();
            Check(main.Settings is null && !closingEditor.HasRetainedData && closingEditor.CatalogReader is null && !closingEditor.IsPicking,
                "Closing settings did not clear the picker delegate and draft state.");
            Check(closingView.IsDisposed && !closingView.HasRetainedData,
                "Closing settings did not dispose the loading picker view.");
            await WaitUnloadedAsync(closingDialog);
            settings = await ShowSettingsAsync();
            openers = settings.Openers;
            Check(!ReferenceEquals(closingEditor, openers) && !settings.HasDraft, "Reopened settings reused a disposed opener editor.");
            gate.SetResult(Controlled(false));
            await Task.Delay(250);
            Check(!closingView.HasRetainedData && closingView.VisibleChoices.Count == 0 && !settings.HasDraft,
                "A late catalog result changed the reopened settings window.");
            Check(core.Refresh().Revision == revision, "The reopened settings window saved configuration.");
            Pass("closing and reopening settings while loading cleared the delegate and view without late results");

            // 11. Quit while a catalog read is pending: the result is already complete when shutdown clears the view.
            gate = new TaskCompletionSource<ApplicationCatalogResult>(TaskCreationOptions.RunContinuationsAsynchronously);
            openers.CatalogReader = (refresh, _) => { catalogCalls++; if (refresh) refreshCalls++; return gate.Task; };
            var (quitDialog, quitView) = await OpenAsync("app.file");
            Check(quitView.IsLoading && quitDialog.IsLoaded, "The quit fixture picker did not start loading after rendering.");
            gate.SetResult(Controlled(false));
            bool quitOk = await main.Quit().WaitAsync(TimeSpan.FromSeconds(10));
            Check(quitOk, "Shared shutdown did not finish while the picker was loading.");
            quitVerified = true;
            Check(main.Settings is null && main.ShutdownComplete && main.OpenLifetime.IsStopped,
                "Shutdown retained settings or periodic work.");
            Check(!openers.HasRetainedData && !openers.IsPicking && openers.CatalogReader is null,
                "Shutdown retained the opener editor delegate or drafts.");
            Check(quitView.IsDisposed && !quitView.HasRetainedData && quitView.VisibleChoices.Count == 0,
                "Shutdown retained the loading picker view.");
            Check(core.Snapshot.Revision == revision, "Shutdown changed the settings revision.");
            try { core.Refresh(); } catch (ObjectDisposedException) { coreClosed = true; }
            Check(coreClosed, "Shutdown did not dispose the native core.");
            lateIgnored = true;
            await FinishAsync(null);
        }
        catch (Exception failure)
        {
            Environment.ExitCode = 1;
            await FinishAsync(failure.ToString());
        }
    }

    private async Task FinishAsync(string? error)
    {
        if (finished) return; finished = true;
        bool quitOk = false;
        string? quitError = null;
        try { quitOk = await main.Quit().WaitAsync(TimeSpan.FromSeconds(10)); }
        catch (Exception failure) { quitError = failure.ToString(); }
        if (!quitOk && quitError is null) shutdownProblems.Add("Shared shutdown did not report success.");
        if (quitError is not null) shutdownProblems.Add($"Shared shutdown failed: {quitError}");
        if (main.Settings is not null) shutdownProblems.Add("A settings window survived shutdown.");
        if (!main.ShutdownComplete && quitOk) shutdownProblems.Add("The main window did not record shutdown completion.");
        if (!main.OpenLifetime.IsStopped) shutdownProblems.Add("The open lifetime controller kept running.");
        if (openers is { } editor && (editor.HasRetainedData || editor.IsPicking || editor.CatalogReader is not null))
            shutdownProblems.Add("The last opener editor retained data after shutdown.");
        if (lastView is { } view && (!view.IsDisposed || view.HasRetainedData))
            shutdownProblems.Add("The last application picker retained data after shutdown.");
        try { if (!coreClosed) { core.Refresh(); } }
        catch (ObjectDisposedException) { coreClosed = true; }
        catch (Exception failure) { shutdownProblems.Add($"The native core probe failed: {failure.GetType().Name}."); }
        if (!coreClosed) shutdownProblems.Add("The native core handle stayed open after shutdown.");
        webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
            .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
        if (webView2ModuleLoaded) shutdownProblems.Add("WebView2 modules or assemblies were loaded.");
        bool passed = error is null && quitOk && quitError is null && shutdownProblems.Count == 0;
        if (error is not null) shutdownProblems.Insert(0, $"Acceptance failure: {error}");
        string? fixtureError = null;
        WriteReport(passed, shutdownProblems);
        if (passed)
        {
            try
            {
                string full = Path.GetFullPath(directory);
                string temp = Path.GetFullPath(Path.GetTempPath()).TrimEnd(Path.DirectorySeparatorChar);
                const string prefix = "TieZ-WinUI-原生-";
                Check(Path.GetDirectoryName(full) == temp, "Cleanup target is not directly under the canonical temp directory.");
                string name = Path.GetFileName(full);
                Check(name.StartsWith(prefix, StringComparison.Ordinal) && Guid.TryParseExact(name[prefix.Length..], "N", out _),
                    "Cleanup target lacks the owned GUID prefix.");
                Check((File.GetAttributes(full) & FileAttributes.ReparsePoint) == 0, "Cleanup target is a reparse point.");
                Directory.Delete(full, recursive: true);
                fixtureDeleted = !Directory.Exists(full);
                Check(fixtureDeleted, "The owned fixture directory survived deletion.");
            }
            catch (Exception failure) { fixtureError = failure.ToString(); }
        }
        if (fixtureError is not null)
        {
            passed = false;
            shutdownProblems.Add($"Cleanup failure: {fixtureError}");
        }
        // Record the final cleanup state synchronously; preserving the fixture on any failure keeps diagnosis possible.
        WriteReport(passed, shutdownProblems);
        Environment.ExitCode = passed ? 0 : 1;
    }

    private void WriteReport(bool passed, List<string> problems)
    {
        File.WriteAllText(report, JsonSerializer.Serialize(new
        {
            passed,
            error = problems.Count == 0 ? null : string.Join("\n", problems),
            scope = "application-picker",
            checks,
            passes,
            dialogsShown,
            confirmations,
            cancellations,
            catalogCalls,
            refreshCalls,
            realCatalogRows = realRows,
            realCatalogRealized = realRealized,
            realCatalogVirtualized = realVirtualized,
            lateResultIgnored = lateIgnored,
            quitVerified,
            shutdownProblems,
            coreClosed,
            webView2ModuleLoaded,
            fixtureDeleted,
            fixturePreserved = !fixtureDeleted,
            directory = Path.GetFullPath(directory),
            elapsedSeconds = elapsed.Elapsed.TotalSeconds
        }));
    }
}
