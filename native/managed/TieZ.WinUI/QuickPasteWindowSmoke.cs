using System.Diagnostics;
using System.Reflection;
using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using TieZ.Core;

namespace TieZ.Native;

// Isolated synthetic history. This suite never writes the OS clipboard or sends input to another application.
internal sealed class QuickPasteWindowSmoke(MainWindow main, ClipboardPopupWindow popup, NativeCore core, string directory, string report)
{
    private int checks;
    private static T Field<T>(object owner, string name) => (T)owner.GetType().GetField(name, BindingFlags.NonPublic | BindingFlags.Instance)!.GetValue(owner)!;
    private void Check(bool value, string reason) { checks++; if (!value) throw new InvalidOperationException(reason); }
    private async Task Wait(Func<bool> condition, string reason)
    {
        var watch = Stopwatch.StartNew();
        while (!condition() && watch.Elapsed.TotalSeconds < 12) await Task.Delay(25);
        Check(condition(), reason);
    }
    private static IEnumerable<DependencyObject> Walk(DependencyObject root)
    {
        yield return root;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
            foreach (var child in Walk(VisualTreeHelper.GetChild(root, i))) yield return child;
    }
    private static void Invoke(Button button) => ((IInvokeProvider)FrameworkElementAutomationPeer.CreatePeerForElement(button).GetPattern(PatternInterface.Invoke)).Invoke();
    internal async void Start()
    {
        bool passed = false; string? failure = null;
        try
        {
            EdgeDock.AssertFixtures();
            var root = (FrameworkElement)popup.Content;
            await Wait(() => root.IsLoaded && root.ActualHeight > 100 && !popup.IsBusy, "Popup failed to load");
            Check(Directory.Exists(directory), "Isolated fixture data missing");
            Check(!main.AppWindow.IsVisible && !main.History.ManageCaptureStartup, "Management still owns default entry/capture startup");
            Check(!Walk(root).OfType<NavigationView>().Any(), "Popup contains management navigation");
            await Wait(() => root.ActualWidth < 450 && root.ActualWidth > 300 && root.ActualHeight > 400,
                $"Popup DIP dimensions incorrect: {root.ActualWidth}x{root.ActualHeight}, scale={root.XamlRoot.RasterizationScale}");
            var first = await core.AddManualAsync("会议安排\n周五 15:00，讨论下一版。", tags: ["工作"]);
            await core.AddManualAsync("https://example.com/design-notes", "url");
            await core.AddManualAsync("const greeting = 'Hello, TieZ';", "code");
            await core.AddManualAsync("富文本示例", "rich_text", "<b>富文本示例</b>");
            long sensitive = (await core.AddManualAsync("quick-paste-private-canary", tags: ["password"])).Id;
            long image = (await core.AddManualAsync("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAYAAAC56t6BAAAAEUlEQVR4nGP4z8DQAMIMGAwAfcEI+4thakwAAAAASUVORK5CYII=", "image")).Id;
            long pinned = (await core.SetPinnedAsync(first.Id, true)).Id;
            await popup.ReloadAsync();
            Check(popup.Rows.Count == 6 && popup.Rows[0].Id == pinned, "Pinned ordering or row count incorrect");
            await Wait(() => popup.Rows.Single(row => row.Id == image).Thumbnail is not null, "Native image thumbnail missing");
            Check(popup.Rows.Single(row => row.Id == sensitive).Preview.Contains("敏感"), "Sensitive summary exposed content");
            Check(!Walk(root).OfType<TextBlock>().Any(t => t.Text.Contains("quick-paste-private-canary")), "Private content in visual tree");
            await Task.Delay(450); await NativeSmokeCapture.CaptureAsync(root, Path.Combine(Path.GetDirectoryName(report)!, "quick-paste-dark"));
            popup.Search("会议");
            await Wait(() => !popup.IsBusy && popup.Rows.Count == 1, "Search did not narrow results");
            Check(popup.SelectedId == pinned, "Search did not select first result");
            popup.Search("not-a-matching-fixture");
            await Wait(() => !popup.IsBusy && popup.Rows.Count == 0, "Empty search retained stale records");
            Check(Field<StackPanel>(popup, "Empty").Visibility == Visibility.Visible, "No-results hint missing");
            popup.Search(""); await Wait(() => !popup.IsBusy && popup.Rows.Count == 6, "Clearing search failed");
            popup.Select(pinned);
            var publicRow = popup.Rows.First(row => row.Id == pinned);
            var previewTask = popup.PreviewAsync(publicRow);
            await Wait(() => popup.HasDialog, "Explicit preview unavailable");
            Check(popup.AppWindow.IsVisible && popup.SelectedId == pinned, "Preview pasted or hid the panel");
            await core.EditEntryAsync(pinned, "changed after preview");
            await Wait(() => popup.HasPendingChanges && !popup.HasDialog, "Revision change retained stale preview");
            await previewTask;
            Check(!Walk(root).OfType<TextBox>().Any(t => t.Text.Contains("周五")), "Closed preview retained old text");
            Check(popup.SelectedId == pinned, "Background refresh moved selected identity");
            await popup.ReloadAsync();
            var privateRow = popup.Rows.Single(row => row.Id == sensitive);
            var consentTask = popup.PreviewAsync(privateRow);
            await Wait(() => popup.HasDialog, "Sensitive preview bypassed confirmation");
            Check(Field<ContentDialog>(popup, "dialog").Content is string, "Sensitive body loaded before consent");
            Field<ContentDialog>(popup, "dialog").Hide(); await consentTask;
            await popup.ReloadAsync();
            popup.IsComposing = true;
            await popup.UseAsync(popup.Rows.First());
            Check(popup.AppWindow.IsVisible && !popup.HasDialog, "IME composition triggered an action");
            popup.IsComposing = false;
            await core.StopTargetTrackingAsync();
            popup.HidePopup(); await popup.ShowAsync();
            await popup.UseAsync(popup.Rows.Single(row => row.Id == pinned));
            Check(popup.AppWindow.IsVisible && popup.Message == PopupStrings.NoTarget, "Missing target hid popup or claimed success");
            Field<InfoBar>(popup, "Feedback").IsOpen = false;
            core.Update(new Dictionary<string, string> { ["app.color_mode"] = "light" }, core.Snapshot.Revision);
            Check(root.RequestedTheme == ElementTheme.Light, "Popup ignored theme");
            await Task.Delay(450); await NativeSmokeCapture.CaptureAsync(root, Path.Combine(Path.GetDirectoryName(report)!, "quick-paste-light"));
            Field<ToggleButton>(popup, "KeepOpen").IsChecked = true;
            Check(((Microsoft.UI.Windowing.OverlappedPresenter)popup.AppWindow.Presenter).IsAlwaysOnTop, "Keep-open state not applied");
            Field<ToggleButton>(popup, "KeepOpen").IsChecked = false;
            var capture = await core.GetCaptureStatusAsync();
            popup.HidePopup(); main.ShowMain();
            await Wait(() => main.History.IsLoaded && !main.History.IsBusy, "Management did not open");
            Check((await core.GetCaptureStatusAsync()).Starts == capture.Starts, "Management started a second capture worker");
            main.HideToTray(); await popup.ShowAsync();
            Check(popup.Rows.Count == 6 && !main.AppWindow.IsVisible, "Popup did not reopen with shared history");
            // Paging/duplicate and stable-selection checks on >1 page.
            for (int i = 0; i < 55; i++) await core.AddManualAsync($"分页示例 {i:D2}");
            await popup.ReloadAsync(); popup.Select(pinned);
            Check(popup.Rows.Count == 50, "First page not bounded");
            await popup.ReloadAsync(false);
            Check(popup.Rows.Count == 61 && popup.Rows.Select(row => row.Id).Distinct().Count() == 61 && popup.SelectedId == pinned, "Paging duplicates or lost selection");
            popup.HidePopup();
            Check(popup.Rows.All(row => row.Thumbnail is null && row.ThumbnailRequest is null), "Hidden popup retained image resources");
            await popup.ShowAsync();
            core.Update(new Dictionary<string, string> { ["app.language"] = "en" }, core.Snapshot.Revision);
            await Wait(() => !popup.IsBusy, "Language reload failed");
            Check(Field<TextBox>(popup, "SearchBox").PlaceholderText == "Search clipboard…", "Popup chrome did not localize");
            core.Update(new Dictionary<string, string> { ["app.language"] = "zh" }, core.Snapshot.Revision);
            await Wait(() => !popup.IsBusy, "Language restoration failed");
            Check(!Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Any(m => m.ModuleName.Contains("WebView2", StringComparison.OrdinalIgnoreCase)), "WebView2 loaded");
            if (Environment.GetCommandLineArgs().Contains("--hold-ui-preview")) return;
            passed = true;
        }
        catch (Exception error) { failure = error.ToString(); }
        finally
        {
            if (!Environment.GetCommandLineArgs().Contains("--hold-ui-preview") || failure is not null)
            {
                File.WriteAllText(report, JsonSerializer.Serialize(new { passed, checks, error = failure, scope = "quick-paste-window", physicalPasteVerified = false }));
                Environment.ExitCode = passed ? 0 : 1;
                await main.Quit();
            }
        }
    }
}
