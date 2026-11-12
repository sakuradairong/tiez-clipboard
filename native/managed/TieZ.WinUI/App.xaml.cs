using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;

namespace TieZ.Native;

public partial class App : Application
{
    private NativeCore? core;
    private MainWindow? main;
    private ShellController? shell;
    private ClipboardPopupWindow? popup;
    public App()
    {
        InitializeComponent();
        UnhandledException += (_, args) => WriteCrash(args.Exception);
    }

    // A smoke run reports a UI-thread crash the same way it reports a failed check, next to the report file.
    private static void WriteCrash(Exception error)
    {
        string[] arguments = Environment.GetCommandLineArgs();
        int index = Array.IndexOf(arguments, "--smoke");
        if (index < 0 || index + 1 >= arguments.Length) return;
        try { File.WriteAllText(arguments[index + 1] + ".crash.txt", error.ToString()); } catch { }
    }
    protected override async void OnLaunched(LaunchActivatedEventArgs args)
    {
        string[] arguments = Environment.GetCommandLineArgs();
        int smokeIndex = Array.IndexOf(arguments, "--smoke");
        string? report = smokeIndex >= 0 && smokeIndex + 1 < arguments.Length ? arguments[smokeIndex + 1] : null;
        string directory = report is null
            ? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "TieZ.Native.Prototype")
            : Path.Combine(Path.GetTempPath(), $"TieZ-WinUI-原生-{Guid.NewGuid():N}");
        try
        {
            Directory.CreateDirectory(directory);
            core = new NativeCore(Path.Combine(directory, "settings.db"));
            string? targetError = null;
            try { await core.StartTargetTrackingAsync(); }
            catch (CoreException error) { targetError = error.Message; }
            // The application owns capture even while the management view has never been loaded.
            main = new MainWindow(core, directory, captureOnStartup: false);
            if (targetError is not null) main.History.SetTargetError(targetError);
            bool quickPaste = report is null || arguments.Contains("--quick-paste-only");
            if (quickPaste)
            {
                main.History.ManageCaptureStartup = false;
                popup = new ClipboardPopupWindow(main, core);
            }
            shell = new ShellController(main, core, popup);
            await shell.StartAsync(bindMainHotkey: report is null);
            main.Closed += (_, _) =>
            {
                shell.Dispose();
                main.CloseSettings();
                // The window closes only after its shared asynchronous shutdown succeeds.
                Exit();
            };
            if (report is null)
            {
                try { await core.StartCaptureAsync(); }
                catch (Exception error) { popup!.SetMessage(error.Message); }
            }
            if (popup is not null) await popup.ShowAsync(); else main.Activate();
            if (!main.ResidentMode) main.ShowMain();
            if (report is not null)
            {
                if (arguments.Contains("--quick-paste-only")) new QuickPasteWindowSmoke(main, popup!, core, directory, report).Start();
                else if (arguments.Contains("--workspace-ui-only")) new WorkspaceAppearanceWindowSmoke(main, core, directory, report).Start();
                else if (arguments.Contains("--large-history-only")) new LargeHistoryWindowSmoke(main, core, directory, report).Start();
                else if (arguments.Contains("--app-icons-only")) new ApplicationIconsWindowSmoke(main, core, directory, report).Start();
                else if (arguments.Contains("--app-picker-only")) new ApplicationPickerWindowSmoke(main, core, directory, report).Start();
                else if (arguments.Contains("--apps-only")) new NativeAppsWindowSmoke(main, core, directory, report).Start();
                else if (arguments.Contains("--opener-settings-only"))
                {
                    int fixtureIndex = Array.IndexOf(arguments, "--fixture-directory");
                    if (fixtureIndex < 0 || fixtureIndex + 1 >= arguments.Length || !Path.IsPathFullyQualified(arguments[fixtureIndex + 1]))
                        throw new ArgumentException("Opener settings smoke requires an absolute --fixture-directory for the owned managed smoke binary.");
                    new OpenerSettingsWindowSmoke(main, core, directory, report, arguments[fixtureIndex + 1]).Start();
                }
                else if (arguments.Contains("--open-only"))
                {
                    int fixtureIndex = Array.IndexOf(arguments, "--fixture-directory");
                    if (fixtureIndex < 0 || fixtureIndex + 1 >= arguments.Length || !Path.IsPathFullyQualified(arguments[fixtureIndex + 1]))
                        throw new ArgumentException("Open smoke requires an absolute --fixture-directory for the owned managed smoke binary.");
                    new NativeOpenWindowSmoke(main, core, directory, report, arguments[fixtureIndex + 1]).Start();
                }
                else if (arguments.Contains("--exit-only"))
                {
                    int fixtureIndex = Array.IndexOf(arguments, "--fixture-directory");
                    if (fixtureIndex < 0 || fixtureIndex + 1 >= arguments.Length || !Path.IsPathFullyQualified(arguments[fixtureIndex + 1]))
                        throw new ArgumentException("Exit smoke requires an absolute --fixture-directory for the owned managed smoke binary.");
                    new NativeExitWindowSmoke(main, core, directory, report, arguments[fixtureIndex + 1]).Start();
                }
                else if (arguments.Contains("--i18n-only")) new LocalizationWindowSmoke(main, core, directory, report).Start();
                else if (arguments.Contains("--card-copy-only")) new HistoryCardCopyWindowSmoke(main, core, directory, report).Start();
                else if (arguments.Contains("--thumbnails-only")) new HistoryThumbnailWindowSmoke(main, core, directory, report).Start();
                else if (arguments.Contains("--layout-only")) new HistoryLayoutWindowSmoke(main, core, directory, report).Start();
                else if (arguments.Contains("--tag-picker-only")) new TagPickerWindowSmoke(main, core, directory, report).Start();
                else if (arguments.Contains("--tags-only")) new TagWindowSmoke(main, core, directory, report).Start();
                else new WindowSmoke(main, core, directory, report, rulesOnly: arguments.Contains("--rules-only")).Start();
            }
        }
        catch (Exception e)
        {
            shell?.Dispose();
            popup?.Dispose();
            if (core is not null) await core.DisposeAsync();
            if (report is not null)
            {
                File.WriteAllText(report, System.Text.Json.JsonSerializer.Serialize(new { passed = false, error = e.ToString() }));
                Environment.ExitCode = 1;
                Exit();
                return;
            }
            var error = new Window { Title = "TieZ 原生原型 · 启动失败", Content = new TextBlock
            {
                Text = $"无法打开隔离配置：{e.Message}", TextWrapping = TextWrapping.Wrap, Margin = new Thickness(24)
            } };
            main = null;
            error.Closed += (_, _) => Exit();
            error.Activate();
        }
    }
}
