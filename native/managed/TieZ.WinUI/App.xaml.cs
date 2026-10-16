using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;

namespace TieZ.Native;

public partial class App : Application
{
    private NativeCore? core;
    private MainWindow? main;
    public App() => InitializeComponent();

    protected override void OnLaunched(LaunchActivatedEventArgs args)
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
            main = new MainWindow(core, directory, captureOnStartup: report is null);
            main.Closed += (_, _) =>
            {
                main.CloseSettings();
                core.Dispose();
                Exit();
            };
            main.Activate();
            if (report is not null) new WindowSmoke(main, core, directory, report).Start();
        }
        catch (Exception e)
        {
            core?.Dispose();
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
