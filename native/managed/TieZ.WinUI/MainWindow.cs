using Microsoft.UI.Xaml;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

public sealed partial class MainWindow : Window
{
    private readonly NativeCore core;
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
