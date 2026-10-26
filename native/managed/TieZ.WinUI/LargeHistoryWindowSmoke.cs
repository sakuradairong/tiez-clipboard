using System.Diagnostics;
using System.Reflection;
using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TieZ.Core;

namespace TieZ.Native;

// Actual native dialogs/buttons and the real DLL. Capture stays off for deterministic data.
internal sealed class LargeHistoryWindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private Microsoft.UI.Dispatching.DispatcherQueueTimer? heartbeat;
    private int checks, ticks, begins, commits;
    private string? screenshot;
    private readonly List<TaskCompletionSource> releases = [];
    private const BindingFlags Hidden = BindingFlags.Instance | BindingFlags.NonPublic;
    private static readonly FieldInfo Observer = typeof(NativeCore).GetField("largeHistoryObserved", Hidden)!;
    private void Check(bool value, string message) { checks++; if (!value) throw new InvalidOperationException(message); }
    private async Task Wait(Func<bool> ready, string message)
    {
        var watch = Stopwatch.StartNew();
        while (!ready() && watch.Elapsed < TimeSpan.FromSeconds(15)) await Task.Delay(15);
        Check(ready(), message);
    }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject root)
    {
        yield return root;
        for (int index = 0; index < VisualTreeHelper.GetChildrenCount(root); index++)
            foreach (var child in Descendants(VisualTreeHelper.GetChild(root, index))) yield return child;
    }
    private (ContentDialog Dialog, TextBox Input) Current()
    {
        var view = main.History;
        return ((ContentDialog)typeof(HistoryView).GetField("activeDialog", Hidden)!.GetValue(view)!,
            (TextBox)typeof(HistoryView).GetField("dialogInput", Hidden)!.GetValue(view)!);
    }
    private async Task<(ContentDialog Dialog, TextBox Input)> Open(string action)
    {
        await Wait(() => !main.History.IsBusy && !main.History.HasDialog, "History workspace did not become ready");
        main.History.Invoke(action);
        await Wait(() => main.History.IsDialogReady, "Actual native text dialog did not load");
        return Current();
    }
    private async Task Press(ContentDialog dialog, string name)
    {
        Button? button = null;
        await Wait(() => (button = Descendants(dialog).OfType<Button>().SingleOrDefault(item => item.Name == name
            && item.IsLoaded && item.IsEnabled && item.ActualWidth > 0 && item.ActualHeight > 0)) is not null,
            $"Actual current dialog {name} button was not ready");
        ((IInvokeProvider)new ButtonAutomationPeer(button!).GetPattern(PatternInterface.Invoke)).Invoke();
    }
    private async Task Closed(ContentDialog dialog, TextBox input)
    {
        await Wait(() => !main.History.HasDialog && !main.History.IsBusy, "Native dialog did not finish saving/closing");
        Check(input.Text.Length == 0 && dialog.Content is null, "Closed text dialog retained its large draft controls");
    }
    private void Observe(Func<string, JsonElement, Task>? observer) => Observer.SetValue(core, observer);
    private TaskCompletionSource<JsonElement> HoldBegin(out TaskCompletionSource release)
    {
        var entered = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        var held = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        releases.Add(held); release = held;
        Observe((op, result) =>
        {
            if (op == "history_upload_commit") Interlocked.Increment(ref commits);
            if (op != "history_upload_begin") return Task.CompletedTask;
            Interlocked.Increment(ref begins); entered.TrySetResult(result); return held.Task;
        });
        return entered;
    }
    private static string Body => string.Concat(Enumerable.Repeat(string.Concat("  窗口😀0123456789 ", (char)13, (char)10), 6000));
    internal async void Start()
    {
        bool passed = false, webView2ModuleLoaded = false; string? error = null;
        try
        {
            await Wait(() => main.ResidentMode && main.History.IsLoaded && !main.History.IsBusy, "Native main window not ready");
            Check(!(await core.GetCaptureStatusAsync()).Running, "Deterministic large history fixture started OS capture");
            ulong settingsRevision = core.Snapshot.Revision;
            heartbeat = main.DispatcherQueue.CreateTimer(); heartbeat.Interval = TimeSpan.FromMilliseconds(15);
            heartbeat.Tick += (_, _) => ticks++; heartbeat.Start();
            string body = Body;
            var cancelled = await Open("add"); main.History.SetDialogText(body);
            Check(cancelled.Input.Text.Length > 65536, "Native TextBox silently limited the large input");
            await Press(cancelled.Dialog, "CloseButton"); await Closed(cancelled.Dialog, cancelled.Input);
            Check((await core.GetHistoryAsync(new())).StoredCount == 0, "Cancelling an unsaved dialog wrote history");

            var created = await Open("add"); main.History.SetDialogText(body);
            string expected = created.Input.Text;
            var begin = HoldBegin(out var release);
            await Press(created.Dialog, "PrimaryButton");
            await begin.Task.WaitAsync(TimeSpan.FromSeconds(15));
            int before = ticks;
            await Wait(() => ticks >= before + 3, "Actual large-dialog save blocked its dispatcher");
            Check(!created.Input.IsEnabled && !created.Dialog.IsPrimaryButtonEnabled, "Busy save permitted duplicate edits/submissions");
            main.OpenSettings();
            await Wait(() => main.Settings is { IsContentReady: true }, "Independent settings did not open while a large save was staged");
            Check(core.Refresh().Revision == settingsRevision && !(await core.GetCaptureStatusAsync()).Running,
                "Staged save blocked the shared core or restarted capture");
            main.CloseSettings();
            Check(main.History.HasDialog && created.Input.Text == expected, "Closing independent settings lost the active large draft");
            screenshot = await NativeSmokeCapture.CaptureAsync((FrameworkElement)created.Dialog.Content, report);
            release.TrySetResult(); await Closed(created.Dialog, created.Input); Observe(null);
            var page = await core.GetHistoryAsync(new());
            long id = page.Items.Single().Id;
            Check((await core.GetEntryAsync(id)).Content == expected && begins == 1 && commits == 1,
                "Actual Save did not atomically retain the real TextBox payload");

            string original = string.Concat("  原始", (char)13, (char)10, body, (char)9, " ");
            long originalId = (await core.AddManualAsync(original)).Id;
            await main.History.ReloadAsync(originalId);
            await Wait(() => main.History.CanCopy && !main.History.IsBusy, "Original large detail did not load");
            ulong unchangedRevision = (await core.GetHistoryAsync(new())).Revision;
            var unchanged = await Open("edit");
            await Press(unchanged.Dialog, "PrimaryButton"); await Closed(unchanged.Dialog, unchanged.Input);
            Check((await core.GetEntryAsync(originalId)).Content == original
                && (await core.GetHistoryAsync(new())).Revision == unchangedRevision,
                "Unchanged native editor normalized original CRLF or performed a write");

            await main.History.ReloadAsync(originalId); await Wait(() => main.History.CanCopy, "Edit detail unavailable");
            var conflicted = await Open("edit"); main.History.SetDialogText(body + "conflicted draft");
            string draft = conflicted.Input.Text;
            begin = HoldBegin(out release);
            await Press(conflicted.Dialog, "PrimaryButton"); await begin.Task.WaitAsync(TimeSpan.FromSeconds(15));
            await core.SetPinnedAsync(originalId, true);
            release.TrySetResult();
            await Wait(() => !main.History.IsBusy && main.History.HasError && conflicted.Input.IsEnabled, "Real conflict did not leave a usable draft");
            Check(main.History.HasDialog && conflicted.Input.Text == draft && conflicted.Dialog.IsPrimaryButtonEnabled,
                "Failed large save discarded its draft or disabled retry/cancel");
            var preserved = await core.GetEntryAsync(originalId);
            Check(preserved.Content == original && preserved.IsPinned, "Rejected large save overwrote accepted content or pin state");
            await Press(conflicted.Dialog, "CloseButton"); await Closed(conflicted.Dialog, conflicted.Input); Observe(null);

            await main.History.ReloadAsync(originalId); await Wait(() => main.History.CanCopy, "Corrected edit detail unavailable");
            var corrected = await Open("edit"); main.History.SetDialogText(body + "corrected😀");
            expected = corrected.Input.Text;
            await Press(corrected.Dialog, "PrimaryButton"); await Closed(corrected.Dialog, corrected.Input);
            Check((await core.GetEntryAsync(originalId)).Content == expected, "Corrected real large edit did not save");
            long privateId = (await core.AddManualAsync(body, tags: ["password"])).Id;
            await main.History.ReloadAsync(privateId);
            await Wait(() => main.History.CanReveal && !main.History.IsBusy, "Sensitive large row did not remain concealed");
            Check(!main.History.CanCopy, "Concealed large row enabled content actions");
            try { main.History.Invoke("edit"); throw new InvalidOperationException("Concealed large edit was enabled"); }
            catch (InvalidOperationException failure) when (failure.Message == "edit is disabled.") { checks++; }
            main.History.Invoke("reveal"); await Wait(() => main.History.CanCopy, "Actual reveal did not enable explicit large editing");
            var sensitive = await Open("edit"); main.History.SetDialogText(body + "private corrected"); expected = sensitive.Input.Text;
            await Press(sensitive.Dialog, "PrimaryButton"); await Closed(sensitive.Dialog, sensitive.Input);
            Check((await core.GetEntryAsync(privateId)).Content == expected, "Explicit revealed large edit failed");
            Check(core.Refresh().Revision == settingsRevision && !(await core.GetCaptureStatusAsync()).Running,
                "Large dialogs changed settings or started capture");

            var quitting = await Open("add"); main.History.SetDialogText(body + "never commit on quit");
            begin = HoldBegin(out release); await Press(quitting.Dialog, "PrimaryButton");
            await begin.Task.WaitAsync(TimeSpan.FromSeconds(15));
            Task<bool> quit = main.Quit();
            Check(!quit.IsCompleted, "Quit released the core before its accepted staging response settled");
            before = ticks;
            await Wait(() => ticks >= before + 3, "Quit blocked the owner dispatcher during staging drain");
            release.TrySetResult();
            Check(await quit.WaitAsync(TimeSpan.FromSeconds(15)), "Application failed to close after large dialog cancellation");
            Check(quitting.Input.Text.Length == 0 && quitting.Dialog.Content is null,
                "Quit retained text dialog draft controls");
            Check(commits == 1, "Quit committed its cancelled staged draft");
            Check(main.Settings is null && main.OpenLifetime.IsStopped && core.IsClosing == false, "Quit retained borrowed windows/work or the core handle");
            webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(module => module.ModuleName)
                .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(assembly => assembly.GetName().Name ?? ""))
                .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
            Check(!webView2ModuleLoaded, "Large native dialogs loaded WebView2");
            string path = Path.GetFullPath(directory), prefix = "TieZ-WinUI-原生-", name = Path.GetFileName(path);
            Check(Path.GetDirectoryName(path) == Path.TrimEndingDirectorySeparator(Path.GetFullPath(Path.GetTempPath()))
                && name.StartsWith(prefix, StringComparison.Ordinal) && Guid.TryParseExact(name[prefix.Length..], "N", out _)
                && (File.GetAttributes(path) & FileAttributes.ReparsePoint) == 0, "Large dialog fixture left its owned cleanup namespace");
            Directory.Delete(path, true); passed = true;
        }
        catch (Exception failure)
        {
            error = failure.ToString();
            foreach (var release in releases) release.TrySetResult();
            Observe(null); await main.Quit();
        }
        finally
        {
            heartbeat?.Stop();
            File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "large-history-dialogs", checks, ticks,
                begins, commits, screenshot, webView2ModuleLoaded, elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
            Environment.ExitCode = passed ? 0 : 1;
        }
    }
}
