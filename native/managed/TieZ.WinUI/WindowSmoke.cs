using System.Diagnostics;
using System.Text.Json;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using TieZ.Core;

namespace TieZ.Native;

// Runs against real WinUI windows and the Rust DLL, using a disposable isolated database.
internal sealed class WindowSmoke(MainWindow main, NativeCore core, string directory, string report)
{
    private DispatcherQueueTimer? timer;
    private int phase;
    private bool? webView2ModuleLoaded;
    internal void Start()
    {
        timer = main.DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromMilliseconds(600);
        timer.Tick += Tick;
        timer.Start();
    }

    private static void Require(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }

    private void Tick(DispatcherQueueTimer sender, object args)
    {
        try
        {
            switch (phase++)
            {
                case 0:
                    Require(core.Snapshot.Revision == 0, "Initial window binding wrote configuration.");
                    main.OpenSettings();
                    var first = main.Settings;
                    main.OpenSettings();
                    Require(ReferenceEquals(first, main.Settings), "Repeated open created a duplicate settings window.");
                    break;
                case 1:
                    main.Settings!.InvokeSmokeSave();
                    break;
                case 2:
                    Require(core.Snapshot.Revision == 1 && core.Snapshot.Values["app.language"] == "tw", "Save did not reach core.");
                    Require(!main.SearchVisible && main.CurrentTheme == ElementTheme.Dark, "Main window did not synchronize.");
                    main.CloseSettings();
                    break;
                case 3:
                    Require(main.Settings is null, "Settings window was not released.");
                    main.OpenSettings();
                    Require(main.Settings!.DisplayedRevision == 1 && core.Refresh().Revision == 1, "Reopen lost state or wrote configuration.");
                    main.CloseSettings();
                    break;
                default:
                    var modules = Process.GetCurrentProcess().Modules.Cast<ProcessModule>().Select(m => m.ModuleName).ToArray();
                    var assemblies = AppDomain.CurrentDomain.GetAssemblies().Select(a => a.GetName().Name ?? "");
                    webView2ModuleLoaded = modules.Concat(assemblies).Any(m => m.Contains("WebView2", StringComparison.OrdinalIgnoreCase));
                    Require(webView2ModuleLoaded == false, "WebView2 module loaded.");
                    Finish(true, null);
                    break;
            }
        }
        catch (Exception e) { Finish(false, e.ToString()); }
    }

    private void Finish(bool passed, string? error)
    {
        timer!.Stop();
        ulong revision = core.Snapshot.Revision;
        try
        {
            main.Close();
            Directory.Delete(directory, recursive: true);
        }
        catch (Exception e) { passed = false; error = e.ToString(); }
        File.WriteAllText(report, JsonSerializer.Serialize(new { passed, error, phases = phase,
            framework = "WinUI 3", revision, webView2ModuleLoaded }));
        Environment.ExitCode = passed ? 0 : 1;
    }
}
