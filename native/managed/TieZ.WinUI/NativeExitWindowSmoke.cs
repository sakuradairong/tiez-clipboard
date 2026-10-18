using System.Diagnostics;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TieZ.Core;

namespace TieZ.Native;

// Real shared core/exports/locked-file retry; only a copied owned console fixture is launched.
internal sealed class NativeExitWindowSmoke(MainWindow main, NativeCore core, string directory, string report, string fixtureSource)
{
    private readonly Stopwatch elapsed = Stopwatch.StartNew();
    private bool finished, gateHeld;
    private SemaphoreSlim? heldGate;
    private FileStream? locked;
    private string? exported;
    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool PostMessage(nint window, uint message, nuint parameter, nint value);
    private int checks, launches, automaticPolls;
    private bool dispatcherResponsive, timerWasRunning;
    private void Check(bool condition, string message) { checks++; if (!condition) throw new Exception(message); }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject root)
    {
        yield return root;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
            foreach (var child in Descendants(VisualTreeHelper.GetChild(root, i))) yield return child;
    }
    private async Task WaitUntil(Func<bool> ready, string message)
    {
        var timeout = Stopwatch.StartNew();
        while (!ready() && timeout.Elapsed < TimeSpan.FromSeconds(8)) await Task.Delay(20);
        Check(ready(), message);
    }
    internal async void Start()
    {
        try
        {
            await WaitUntil(() => ((FrameworkElement)main.Content).XamlRoot is not null && main.ResidentMode, "Main window/shell was not ready");
            var owner = main.OpenLifetime;
            ulong revision = core.Snapshot.Revision;
            for (int i = 0; i < 3; i++)
            {
                main.OpenSettings(); main.CloseSettings();
                Check(ReferenceEquals(owner, main.OpenLifetime), "Settings reopen replaced the lifetime owner");
            }
            await owner.PollNowAsync();
            Check(owner.Status is { ActiveExports: 0, Closed: false }, "Readonly initial poll allocated exports");
            Check(core.Refresh().Revision == revision, "Cleanup poll/settings reopen saved settings");
            string fixture = Path.Combine(directory, "编辑器 %20 ' &"); Directory.CreateDirectory(fixture);
            foreach (string file in new[] { "TieZ.Core.Smoke.exe", "TieZ.Core.Smoke.dll", "TieZ.Core.Smoke.deps.json", "TieZ.Core.Smoke.runtimeconfig.json", "TieZ.Core.dll" })
                File.Copy(Path.Combine(fixtureSource, file), Path.Combine(fixture, file));
            File.WriteAllText(Path.Combine(fixture, "open-fixture.marker"), "owned child");
            Check(core.Update(new Dictionary<string, string> { ["app.text"] = Path.Combine(fixture, "TieZ.Core.Smoke.exe") }, core.Snapshot.Revision), "Fixture handler was not saved");
            const string body = "  shutdown 原始\r\n\t  ";
            long id = (await core.AddManualAsync(body)).Id;
            ulong historyRevision = (await core.GetHistoryAsync(new())).Revision;
            var opened = await core.OpenContentAsync(id, historyRevision);
            Check(opened.Exported && opened.Route == "desktop", "Real core did not launch the owned editor");
            launches++;
            string observedPath = Path.Combine(fixture, "open-observed.json");
            await WaitUntil(() => File.Exists(observedPath), "Owned child did not report its export");
            using (var observed = JsonDocument.Parse(File.ReadAllText(observedPath)))
            {
                exported = observed.RootElement.GetProperty("Path").GetString()!;
                Check(Convert.FromBase64String(observed.RootElement.GetProperty("Body").GetString()!).SequenceEqual(Encoding.UTF8.GetBytes(body)), "Fixture received changed bytes");
                int pid = observed.RootElement.GetProperty("Pid").GetInt32();
                try { using var process = Process.GetProcessById(pid); await process.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(5)); }
                catch (ArgumentException) { }
            }
            await owner.PollNowAsync();
            Check(owner.Status is { ActiveExports: 1 }, "Cleanup owner did not report active export");
            // Accelerate only this fixture's existing timer, without constructing another owner.
            var timer = (DispatcherQueueTimer)typeof(OpenLifetimeController).GetField("timer", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(owner)!;
            timerWasRunning = timer.IsRunning;
            Check(timerWasRunning && !owner.IsStopped, "Cleanup timer was not started");
            timer.Stop(); timer.Interval = TimeSpan.FromMilliseconds(80); timer.Start();
            int before = owner.CompletedPolls;
            main.HideToTray(); Check(!main.AppWindow.IsVisible, "Fixture did not hide to tray");
            await WaitUntil(() => owner.CompletedPolls >= before + 2, "Cleanup polling stopped while hidden");
            automaticPolls = owner.CompletedPolls - before;
            timer.Stop(); timer.Interval = TimeSpan.FromSeconds(30); timer.Start();
            Check(core.Refresh().Revision == revision + 1, "Background cleanup wrote settings");
            main.OpenSettings(); main.OpenAdvancedSettings();
            locked = new FileStream(exported, FileMode.Open, FileAccess.Read, FileShare.None);
            heldGate = (SemaphoreSlim)typeof(NativeCore).GetField("openGate", BindingFlags.Instance | BindingFlags.NonPublic)!.GetValue(core)!;
            heldGate.Wait(); gateHeld = true;
            Task<bool> first = main.Quit();
            Check(!first.IsCompleted && main.ShutdownPending && core.IsClosing, "Exit blocked its initiating dispatcher");
            Check(ReferenceEquals(first, main.Quit()), "Duplicate quit created another shutdown task");
            var pulse = new TaskCompletionSource<bool>();
            Check(main.DispatcherQueue.TryEnqueue(() => pulse.TrySetResult(true)), "Could not queue dispatcher heartbeat");
            dispatcherResponsive = await pulse.Task.WaitAsync(TimeSpan.FromSeconds(2));
            Check(dispatcherResponsive, "Dispatcher stalled while exit waited");
            // Window.Close programmatically destroys the Window, bypassing the system
            // close affordance. Exercise WM_CLOSE on our own HWND instead.
            Check(PostMessage(WinRT.Interop.WindowNative.GetWindowHandle(main), 0x0010, 0, 0), "Could not post the owned window close request");
            var closeProcessed = new TaskCompletionSource<bool>();
            main.DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () => closeProcessed.TrySetResult(true));
            await closeProcessed.Task.WaitAsync(TimeSpan.FromSeconds(2));
            Check(main.AppWindow is { IsVisible: true } && ReferenceEquals(first, main.ShutdownTask), "Close button destroyed the waiting main window");
            heldGate.Release(); gateHeld = false;
            Check(!await first.WaitAsync(TimeSpan.FromSeconds(8)), "Locked export falsely reported complete exit");
            Check(main.AppWindow.IsVisible && !main.ShutdownComplete && main.ExitRetryEnabled, "Failed exit lost visible retry controls");
            Check(main.ExitMessage.Contains("重试退出") && !main.ExitMessage.Contains(exported), "Exit prompt missing action or exposed private path");
            Check(owner.IsStopped && main.Settings is null && main.AdvancedSettings is null, "Shutdown retained timers/settings windows");
            Check(main.CompactPreview.WindowHandle == 0 && !main.CompactPreview.HasPending && !main.History.HasLayoutResources, "Shutdown retained preview/history resources");
            main.OpenSettings(); main.OpenAdvancedSettings(); main.HideToTray();
            Check(main.Settings is null && main.AdvancedSettings is null && main.AppWindow.IsVisible, "Closing state reopened business windows or hid retry");
            Check(File.Exists(exported), "Failed cleanup lost the locked export");
            locked.Dispose(); locked = null;
            var retry = Descendants((DependencyObject)main.Content).OfType<Button>().Single(b => b.Name == "RetryExitButton");
            Check(retry.IsEnabled, "Native retry button is disabled");
            main.Closed += (_, _) => Finish(main.ShutdownComplete && !File.Exists(exported), null);
            ((IInvokeProvider)new ButtonAutomationPeer(retry).GetPattern(PatternInterface.Invoke)).Invoke();
            await WaitUntil(() => main.ShutdownTask is { IsCompleted: true }, "Retry button did not complete shutdown");
        }
        catch (Exception error)
        {
            if (gateHeld) { heldGate!.Release(); gateHeld = false; }
            locked?.Dispose(); locked = null;
            Finish(false, error.ToString());
            await main.Quit();
        }
    }
    private void Finish(bool passed, string? error)
    {
        if (finished) return; finished = true;
        bool webView2ModuleLoaded = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(m => m.ModuleName)
            .Concat(AppDomain.CurrentDomain.GetAssemblies().Select(a => a.GetName().Name ?? ""))
            .Any(name => name.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
        try
        {
            Check(!webView2ModuleLoaded, "WebView2 loaded during native shutdown");
            if (passed) Directory.Delete(directory, recursive: true);
        }
        catch (Exception failure) { passed = false; error = $"{error}\n{failure}"; }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, scope = "native-shutdown", checks,
            realNativeEditorLaunches = launches, dispatcherResponsive, timerWasRunning, automaticPolls, webView2ModuleLoaded, elapsedSeconds = elapsed.Elapsed.TotalSeconds }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
