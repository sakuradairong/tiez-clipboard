using Microsoft.UI.Xaml;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

public sealed partial class MainWindow : Window
{
    private readonly NativeCore core;
    private bool residentMode, quitting;
    internal SettingsWindow? Settings { get; private set; }
    internal bool SearchVisible => HistoryWorkspace.SearchVisible;
    internal ElementTheme CurrentTheme => MainRoot.RequestedTheme;
    internal HistoryView History => HistoryWorkspace;

    public MainWindow(NativeCore core, string directory, bool captureOnStartup = true)
    {
        this.core = core;
        InitializeComponent();
        Title = "TieZ · 原生迁移版";
        AppWindow.Resize(new SizeInt32(1080, 760));
        AppWindow.Closing += (_, args) =>
        {
            if (residentMode && !quitting) { args.Cancel = true; HideToTray(); }
        };
        HideButton.Click += (_, _) => HideToTray();
        QuitButton.Click += (_, _) => Quit();
        Navigation.SelectedItem = HistoryNav;
        Navigation.ItemInvoked += (_, args) =>
        {
            if (args.IsSettingsInvoked) OpenSettings();
            Navigation.SelectedItem = HistoryNav;
        };
        HistoryWorkspace.Initialize(core, captureOnStartup);
        core.Changed += Apply;
        Closed += (_, _) =>
        {
            core.Changed -= Apply;
            HistoryWorkspace.Dispose();
        };
        Apply(core.Snapshot);
    }

    internal void SetResidentMode(bool enabled)
    {
        residentMode = enabled;
        HideButton.IsEnabled = enabled;
    }
    internal void SetShellUnavailable(string message)
    {
        ShellError.Message = message; ShellError.IsOpen = true;
    }
    internal void HideToTray()
    {
        if (residentMode && !quitting) AppWindow.Hide();
    }
    internal void ShowMain()
    {
        if (quitting) return;
        if (AppWindow.Presenter is Microsoft.UI.Windowing.OverlappedPresenter presenter
            && presenter.State == Microsoft.UI.Windowing.OverlappedPresenterState.Minimized) presenter.Restore();
        AppWindow.Show(); Activate();
    }
    internal void Quit()
    {
        if (quitting) return;
        quitting = true; CloseSettings(); Close();
    }
    internal bool ResidentMode => residentMode;

    internal void OpenSettings()
    {
        if (Settings is null)
        {
            Settings = new SettingsWindow(core);
            Settings.Closed += (_, _) => Settings = null;
        }
        Settings.Activate();
    }

    internal void CloseSettings() => Settings?.Close();

    private void Apply(SettingsSnapshot snapshot)
    {
        MainRoot.RequestedTheme = ViewTheme.From(snapshot);
        HistoryWorkspace.SearchVisible = snapshot.Values["app.show_search_box"] == "true";
    }
}

internal static class ViewTheme
{
    internal static ElementTheme From(SettingsSnapshot snapshot) => snapshot.Values["app.color_mode"] switch
    {
        "dark" => ElementTheme.Dark,
        "light" => ElementTheme.Light,
        _ => ElementTheme.Default
    };
}
