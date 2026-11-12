using System.Diagnostics;
using System.Reflection;
using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

internal sealed class WorkspaceAppearanceWindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private int checks;
    private bool finished;
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private static T Field<T>(object owner, string name) => (T)owner.GetType().GetField(name, BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(owner)!;
    private void Check(bool value, string message) { checks++; if (!value) throw new InvalidOperationException(message); }
    private async Task Wait(Func<bool> condition, string message)
    {
        var watch = Stopwatch.StartNew(); while (!condition() && watch.Elapsed < TimeSpan.FromSeconds(10)) await Task.Delay(15);
        Check(condition(), message);
    }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject node)
    {
        yield return node;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(node); i++)
            foreach (var child in Descendants(VisualTreeHelper.GetChild(node, i))) yield return child;
    }
    private void Invoke(UIElement control)
    {
        Check(FrameworkElementAutomationPeer.CreatePeerForElement(control).GetPattern(PatternInterface.Invoke) is IInvokeProvider, "Native action has no Invoke pattern");
        ((IInvokeProvider)FrameworkElementAutomationPeer.CreatePeerForElement(control).GetPattern(PatternInterface.Invoke)).Invoke();
    }
    private static async Task Capture(FrameworkElement surface, string path)
    {
        // Allow XAML transitions and virtualized containers to finish their queued layout before the raster.
        await Task.Delay(400);
        surface.UpdateLayout();
        await NativeSmokeCapture.CaptureAsync(surface, path);
    }
    private static bool Within(FrameworkElement element, FrameworkElement surface)
    {
        var point = element.TransformToVisual(surface).TransformPoint(default);
        return element.IsLoaded && element.ActualWidth >= 24 && element.ActualHeight >= 24
            && point.X >= -1 && point.Y >= -1 && point.X + element.ActualWidth <= surface.ActualWidth + 1;
    }
    internal async void Start()
    {
        try
        {
            var view = main.History;
            await Wait(() => main.ResidentMode && view.IsLoaded && !view.IsBusy, "Workspace did not load");
            var navigation = Field<NavigationView>(main, "Navigation");
            var hide = Field<Button>(main, "HideButton"); var quit = Field<Button>(main, "QuitButton");
            var hideLabel = Field<TextBlock>(main, "HideLabel"); var quitLabel = Field<TextBlock>(main, "QuitLabel");
            var migration = Field<TextBlock>(main, "MigrationNote");
            var root = (FrameworkElement)main.Content;
            var detail = (FrameworkElement)view.FindName("DetailBody"); var hint = (FrameworkElement)view.FindName("DetailHint");
            main.AppWindow.Resize(new SizeInt32(680, 760)); navigation.IsPaneOpen = false;
            await Wait(() => Within(hide, navigation) && Within(quit, navigation)
                && hideLabel.Visibility == Visibility.Collapsed && quitLabel.Visibility == Visibility.Collapsed
                && migration.Visibility == Visibility.Collapsed, "Collapsed navigation clipped lifecycle actions");
            Check(AutomationProperties.GetName(hide) == "隐藏到托盘" && AutomationProperties.GetName(quit) == "退出 TieZ", "Navigation lost action labels");
            Check(FrameworkElementAutomationPeer.CreatePeerForElement(hide).GetAutomationControlType() == AutomationControlType.Button
                && FrameworkElementAutomationPeer.CreatePeerForElement(quit).GetAutomationControlType() == AutomationControlType.Button,
                "Lifecycle actions are not native buttons");
            Check(hide.IsTabStop && quit.IsTabStop && hide.IsEnabled && quit.IsEnabled && hide.ActualHeight >= 36 && quit.ActualWidth >= 36,
                "Lifecycle actions lost keyboard reach or hit target");
            await Capture(root, Path.Combine(Path.GetDirectoryName(report)!, "collapsed-pane"));
            Check(hint.Visibility == Visibility.Visible && detail.Visibility == Visibility.Collapsed, "Empty detail displayed a blank editor");
            var detailPanel = (FrameworkElement)view.FindName("DetailPanel");
            var records = (FrameworkElement)view.FindName("RecordsSurface");
            Check(detailPanel.Visibility == Visibility.Collapsed && records.ActualHeight > view.ActualHeight * .5,
                "Empty detail or header chrome still consumes most of the history workspace");
            long text = (await core.AddManualAsync("界面示例 · 原始内容保持不变", tags: ["ui"])).Id;
            foreach (string sample in new[] { "UI 示例：会议安排\n周五下午 3 点，讨论下一版功能。", "https://example.com/design-notes", "const greeting = 'Hello, TieZ';",
                "UI 示例：常用地址和工作笔记，可随时搜索。", "UI 示例：支持原格式复制，也可以选择纯文本。" })
                await core.AddManualAsync(sample);
            long privateId = (await core.AddManualAsync("workspace-private-canary", tags: ["password"])).Id;
            await view.ReloadAsync(text); await Wait(() => view.CanCopy && detail.Visibility == Visibility.Visible, "Selected detail did not load");
            await Wait(() => Grid.GetRow(detailPanel) == 1 && detailPanel.ActualWidth > 200, "Narrow window did not place detail below the list");
            Check(Math.Abs(detailPanel.ActualWidth - records.ActualWidth) < 2, "Narrow detail clipped the history list");
            await Capture(root, Path.Combine(Path.GetDirectoryName(report)!, "narrow-selected"));
            var filters = (Button)view.FindName("FiltersButton");
            Invoke(filters);
            await Wait(() => Field<TextBox>(view, "TagFilter").IsLoaded && Field<ComboBox>(view, "OrderChoice").IsLoaded,
                "Filters flyout lost tag or sort controls");
            await Task.Delay(250);
            Check(VisualTreeHelper.GetOpenPopupsForXamlRoot(view.XamlRoot).Any(popup => popup.IsOpen
                && popup.Child is { } child && Descendants(child).Contains(Field<TextBox>(view, "TagFilter"))), "Filters closed before the user could interact");
            Check(Field<ComboBox>(view, "LayoutChoice").IsEnabled, "View selection is not reachable from filters");
            filters.Flyout.Hide();
            Check(view.Detail == "界面示例 · 原始内容保持不变", "Presentation changed stored text");
            Check(((TextBlock)view.FindName("DetailTitle")).Text == "文本", "Type identifiers leaked into detail heading");
            Check(((TextBlock)view.FindName("SelectionInfo")).Text.Contains("1"), "Selection status did not update");
            view.SelectMany(); await Wait(() => hint.Visibility == Visibility.Visible && detail.Visibility == Visibility.Collapsed, "Clearing selection retained detail editor");
            Check(detailPanel.Visibility == Visibility.Collapsed, "Clearing selection retained an empty detail panel");
            view.Search("no-match-ui-fixture-20261030");
            await Wait(() => !view.IsBusy && ((FrameworkElement)view.FindName("EmptyHistory")).Visibility == Visibility.Visible,
                "An empty search did not explain the result");
            Check(((TextBlock)view.FindName("EmptyTitle")).Text == WorkspaceStrings.Current.NoResults, "Filtered empty state is misleading");
            view.Search(""); await Wait(() => !view.IsBusy && Field<ListView>(view, "HistoryList").Items.Count > 0, "Clearing search did not restore history");
            await view.ReloadAsync(privateId); await Wait(() => view.CanReveal, "Private row did not remain concealed");
            Check(detail.Visibility == Visibility.Collapsed && hint.Visibility == Visibility.Visible && view.Detail.Length == 0, "Private detail bypassed concealment");
            view.Invoke("reveal"); await Wait(() => view.CanCopy && detail.Visibility == Visibility.Visible, "Private reveal did not show detail");
            Check(view.Detail == "workspace-private-canary", "Reveal changed private content");
            await view.ReloadAsync(privateId); await Wait(() => view.CanReveal && detail.Visibility == Visibility.Collapsed, "Reload retained private reveal");
            core.Update(new Dictionary<string, string> { ["app.color_mode"] = "light" }, core.Snapshot.Revision);
            await Wait(() => root.ActualTheme == ElementTheme.Light, "Light theme did not propagate");
            var light = ((SolidColorBrush)((Grid)root).Background).Color;
            core.Update(new Dictionary<string, string> { ["app.color_mode"] = "dark" }, core.Snapshot.Revision);
            await Wait(() => root.ActualTheme == ElementTheme.Dark, "Dark theme did not propagate");
            Check(((SolidColorBrush)((Grid)root).Background).Color != light, "Semantic surface did not change between light/dark");
            main.AppWindow.Resize(new SizeInt32(1800, 1000)); navigation.IsPaneOpen = true;
            var expanded = Stopwatch.StartNew();
            while (expanded.Elapsed < TimeSpan.FromSeconds(4)
                && !(Within(hide, navigation) && Within(quit, navigation)
                    && hideLabel.Visibility == Visibility.Visible && quitLabel.Visibility == Visibility.Visible
                    && hideLabel.ActualWidth >= 48 && hideLabel.ActualHeight <= 32
                    && migration.Visibility == Visibility.Visible && migration.ActualWidth >= 120 && migration.ActualHeight <= 48))
                await Task.Delay(30);
            Check(Within(hide, navigation) && Within(quit, navigation)
                && hideLabel.Visibility == Visibility.Visible && hideLabel.ActualWidth >= 48 && hideLabel.ActualHeight <= 32
                && migration.Visibility == Visibility.Visible && migration.ActualWidth >= 120 && migration.ActualHeight <= 48,
                $"Expanded navigation clipped lifecycle actions: mode={navigation.DisplayMode} open={navigation.IsPaneOpen} window={navigation.ActualWidth:0} label={hideLabel.ActualWidth:0}x{hideLabel.ActualHeight:0} vis={hideLabel.Visibility} note={migration.ActualWidth:0}x{migration.ActualHeight:0} visNote={migration.Visibility}");
            Check(hideLabel.Text == "隐藏到托盘" && quitLabel.Text == "退出 TieZ"
                && quitLabel.ActualWidth >= 48 && quitLabel.ActualHeight <= 32,
                "Expanded action labels are missing or vertical");
            Check(migration.Text.Contains("独立数据库", StringComparison.Ordinal), "Expanded pane lost the isolated-database warning");
            await view.ReloadAsync(text); await Wait(() => view.CanCopy && !view.IsBusy, "Public detail unavailable");
            await Wait(() => Grid.GetColumn(detailPanel) == 1 && Grid.GetRow(detailPanel) == 0 && detailPanel.ActualWidth >= 280,
                "Wide window did not place detail beside the list");
            await Capture(root, Path.Combine(Path.GetDirectoryName(report)!, "expanded-pane"));
            core.Update(new Dictionary<string, string> { ["app.color_mode"] = "light" }, core.Snapshot.Revision);
            await Wait(() => root.ActualTheme == ElementTheme.Light, "Light workspace did not update");
            await Capture(root, Path.Combine(Path.GetDirectoryName(report)!, "expanded-light"));
            core.Update(new Dictionary<string, string> { ["app.color_mode"] = "dark" }, core.Snapshot.Revision);
            var commands = (CommandBar)view.FindName("RecordCommands");
            var more = Descendants(commands).OfType<Button>().Single(button => button.Name == "MoreButton");
            Invoke(more);
            await Wait(() => Field<AppBarButton>(view, "CopyPlainButton").IsLoaded
                && Field<AppBarButton>(view, "PreviewButton").IsLoaded && Field<AppBarButton>(view, "ClearButton").IsLoaded,
                "More menu did not expose original secondary operations");
            Check(Field<AppBarButton>(view, "CopyPlainButton").IsEnabled && Field<AppBarButton>(view, "ClearButton").IsEnabled,
                "Secondary commands lost their original availability");
            foreach (var popup in VisualTreeHelper.GetOpenPopupsForXamlRoot(view.XamlRoot)) popup.IsOpen = false;
            Invoke(hide); await Wait(() => !main.AppWindow.IsVisible, "Actual navigation Hide action did not hide main");
            Check((await core.PollShellAsync()).Status.Running, "Navigation Hide stopped the shared shell");
            main.OpenSettings(); await Wait(() => main.Settings is { IsContentReady: true }, "Settings did not open independently while hidden");
            var settingsRoot = (FrameworkElement)main.Settings!.Content;
            var settingsSave = Field<Button>(main.Settings, "save");
            Check(Within(settingsSave, settingsRoot) && settingsSave.TransformToVisual(settingsRoot).TransformPoint(default).Y + settingsSave.ActualHeight <= settingsRoot.ActualHeight + 1,
                "Save settings is below the visible viewport");
            await Capture((FrameworkElement)main.Settings!.Content, Path.Combine(Path.GetDirectoryName(report)!, "settings-surface"));
            main.CloseSettings(); main.ShowMain();
            await Wait(() => main.AppWindow.IsVisible && quit.IsLoaded, "Main did not return");
            main.Closed += (_, _) => Finish(true, null);
            // Optional local visual review keeps only this isolated fixture alive; the real Quit action still
            // owns shutdown and the same guarded fixture cleanup. Ordinary acceptance runs never pause here.
            if (Environment.GetCommandLineArgs().Contains("--hold-ui-preview")) return;
            Invoke(quit);
        }
        catch (Exception failure) { Finish(false, failure.ToString()); await main.Quit(); }
    }
    private void Finish(bool passed, string? error)
    {
        if (finished) return; finished = true;
        bool webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
            .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
        try
        {
            Check(!webView2ModuleLoaded, "Workspace loaded WebView2");
            if (passed)
            {
                Check(main.ShutdownComplete && main.Settings is null && main.OpenLifetime.IsStopped, "Actual Quit action retained shared core/windows");
                string path = Path.GetFullPath(directory), name = Path.GetFileName(path), prefix = "TieZ-WinUI-原生-";
                Check(Path.GetDirectoryName(path) == Path.TrimEndingDirectorySeparator(Path.GetFullPath(Path.GetTempPath()))
                    && name.StartsWith(prefix, StringComparison.Ordinal) && Guid.TryParseExact(name[prefix.Length..], "N", out _)
                    && (File.GetAttributes(path) & FileAttributes.ReparsePoint) == 0, "Workspace fixture cleanup left its namespace");
                Directory.Delete(path, true);
            }
        }
        catch (Exception failure) { passed = false; error = $"{error}; {failure}"; }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "workspace-appearance", checks,
            webView2ModuleLoaded, elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
