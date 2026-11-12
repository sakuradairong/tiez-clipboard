using System.Diagnostics;
using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TieZ.Core;

namespace TieZ.Native;

internal sealed class PasteQueueWindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private int checks;
    private string? screenshot;
    private void Check(bool value, string reason) { checks++; if (!value) throw new InvalidOperationException(reason); }
    private async Task Wait(Func<bool> ready, string reason)
    {
        var watch = Stopwatch.StartNew();
        while (!ready() && watch.Elapsed.TotalSeconds < 12) await Task.Delay(25);
        Check(ready(), reason);
    }
    private static void Invoke(FrameworkElement control) =>
        ((IInvokeProvider)FrameworkElementAutomationPeer.CreatePeerForElement(control).GetPattern(PatternInterface.Invoke)).Invoke();
    private static IEnumerable<DependencyObject> Walk(DependencyObject root)
    {
        yield return root;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
            foreach (var child in Walk(VisualTreeHelper.GetChild(root, i))) yield return child;
    }
    internal async void Start()
    {
        bool passed = false; string? error = null; bool browser = false;
        try
        {
            var history = main.History;
            await Wait(() => history.IsLoaded && !history.IsBusy, "History window did not load");
            Check(!(await core.GetCaptureStatusAsync()).Running, "Fixture must not read the system clipboard");
            await core.StopTargetTrackingAsync();
            var a = await core.AddManualAsync("队列 first\r\n ");
            var b = await core.AddManualAsync("second");
            var c = await core.AddManualAsync("sensitive body", tags: ["sensitive"]);
            ulong settingsRevision = core.Snapshot.Revision;
            await history.ReloadAsync();
            history.SelectMany(a.Id, b.Id, c.Id);
            long[] displayed = history.Rows.Where(row => new[] { a.Id, b.Id, c.Id }.Contains(row.Id)).Select(row => row.Id).ToArray();
            var set = (AppBarButton)history.FindName("QueueSelectionButton");
            Check(set.IsEnabled, "Queue selection action is disabled");
            Invoke(set);
            await Wait(() => history.PasteQueue is { IsLoaded: true, IsBusy: false }, "Queue dialog did not load");
            var view = history.PasteQueue!;
            await Wait(() => !view.IsBusy && view.Ids.Count == displayed.Length, "Queue snapshot did not reach the dialog");
            Check(view.Ids.SequenceEqual(displayed), $"Queue order differs: expected {string.Join(',', displayed)}, actual {string.Join(',', view.Ids)}");
            Check(view.QueueList.Items.Cast<string>().All(text => !text.Contains("sensitive body")), "Sensitive body leaked into queue");
            view.QueueList.SelectedIndex = 1;
            Check(view.UpButton.IsEnabled, "Move-up action not available");
            Invoke(view.UpButton);
            await Wait(() => !view.IsBusy && view.Ids.Count == displayed.Length && view.Ids[0] == displayed[1], "Move-up did not save the new order");
            var before = view.Ids.ToArray();
            Invoke(view.NextButton);
            await Wait(() => !view.IsBusy && view.HasError, "Stopped target was not reported");
            Check(view.Ids.SequenceEqual(before), "Failed paste consumed queue");
            view.QueueList.SelectedIndex = 1;
            Invoke(view.RemoveButton);
            await Wait(() => !view.IsBusy && view.Ids.Count == 2, "Remove did not update shared queue");
            Check((await core.GetPasteQueueAsync()).Ids.SequenceEqual(view.Ids), "Queue UI diverged from core");
            core.Update(new Dictionary<string,string> { ["app.language"] = "en", ["app.color_mode"] = "dark" }, core.Snapshot.Revision);
            Check(view.QueueList.Items.Cast<string>().All(text => text.Contains("Record #")), "Open queue did not localize live");
            await Wait(() => Walk(view).OfType<TextBlock>().Any(text => text.Text.Contains("Record #")), "Realized queue rows did not switch to English");
            await Wait(() => view.ActualTheme == ElementTheme.Dark, "Open queue did not adopt the saved theme");
            screenshot = await NativeSmokeCapture.CaptureAsync(view, report);
            var retained = view.Ids.ToArray();
            var contentControls = Walk(view).ToHashSet();
            var close = Walk(history.QueueDialog!).OfType<Button>().First(button => button.Name == "CloseButton" && !contentControls.Contains(button));
            Invoke(close);
            await Wait(() => !history.HasDialog && !history.IsBusy, "Queue dialog did not close");
            Check(view.Disposed, "Closing queue left its timer/subscriptions active");
            Check((await core.GetPasteQueueAsync()).Ids.SequenceEqual(retained), "Closing queue cleared core state");
            Invoke((AppBarButton)history.FindName("QueueButton"));
            await Wait(() => history.PasteQueue is { IsLoaded: true, IsBusy: false } ready && ready.Ids.Count == retained.Length,
                "Queue dialog did not reopen with shared state");
            view = history.PasteQueue!;
            Invoke(view.ClearButton);
            await Wait(() => !view.IsBusy && view.Ids.Count == 0, "Clear did not empty queue");
            Check(!view.NextButton.IsEnabled, "Empty queue still permits paste");
            Check(core.Snapshot.Revision == settingsRevision + 1, "Queue actions saved configuration unexpectedly");
            main.OpenSettings();
            Check(main.Settings is not null, "Independent settings failed while queue open");
            main.CloseSettings();
            Check(!view.Disposed && (await core.GetPasteQueueAsync()).Ids.Count == 0, "Settings close released shared queue/core");
            browser = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Any(module => module.ModuleName.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
            Check(!browser, "WebView2 loaded");
            Check(await main.Quit(), "Quit did not close the shared core");
            Check(view.Disposed && view.Ids.Count == 0, "Queue controls/timer retained after quit");
            passed = true;
        }
        catch (Exception failure)
        {
            error = failure + "\n" + main.History.PasteQueue?.Diagnostic;
            try {
                error += "\ncore=" + JsonSerializer.Serialize(await core.GetPasteQueueAsync());
                if (main.History.PasteQueue is { } view) screenshot = await NativeSmokeCapture.CaptureAsync(view, report);
            } catch (Exception diagnosticError) { error += "\n" + diagnosticError.Message; }
        }
        finally
        {
            try { await main.Quit(); Directory.Delete(directory, recursive: true); }
            catch (Exception failure) { passed = false; error += "\n" + failure; }
            File.WriteAllText(report, JsonSerializer.Serialize(new { passed, checks, error, screenshot,
                scope = "sequential-queue-window", webView2ModuleLoaded = browser, physicalPasteVerified = false }));
            Environment.ExitCode = passed ? 0 : 1;
        }
    }
}
