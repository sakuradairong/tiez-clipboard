using Microsoft.UI.Xaml;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

public sealed partial class MainWindow : Window
{
    private readonly NativeCore core;
    private readonly CompactPreviewController compactPreview;
    internal CompactPreviewController CompactPreview => compactPreview;
    private bool residentMode, quitting, active;
    internal SettingsWindow? Settings { get; private set; }
    internal AdvancedSettingsWindow? AdvancedSettings { get; private set; }
    internal bool SearchVisible => HistoryWorkspace.SearchVisible;
    internal ElementTheme CurrentTheme => MainRoot.RequestedTheme;
    internal HistoryView History => HistoryWorkspace;

    public MainWindow(NativeCore core, string directory, bool captureOnStartup = true)
    {
        this.core = core;
        InitializeComponent();
        compactPreview = new CompactPreviewController(this, core);
        HistoryWorkspace.PreviewRequested += compactPreview.Request;
        HistoryWorkspace.PreviewInvalidated += compactPreview.Cancel;
        Title = "TieZ · 原生迁移版";
        AppWindow.Resize(new SizeInt32(1080, 760));
        Activated += (_, args) =>
        {
            active = args.WindowActivationState != WindowActivationState.Deactivated;
            if (!active) compactPreview.Cancel();
        };
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
            CloseAdvancedSettings();
            core.Changed -= Apply;
            HistoryWorkspace.PreviewRequested -= compactPreview.Request;
            HistoryWorkspace.PreviewInvalidated -= compactPreview.Cancel;
            compactPreview.Dispose();
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
        compactPreview.Cancel();
        if (residentMode && !quitting) AppWindow.Hide();
    }
    internal void ToggleMain()
    {
        if (AppWindow.IsVisible && active && residentMode) HideToTray(); else ShowMain();
    }
    internal void ClearShellWarning() => ShellError.IsOpen = false;
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
        quitting = true; CloseAdvancedSettings(); CloseSettings(); Close();
    }
    internal bool ResidentMode => residentMode;

    internal void OpenSettings()
    {
        if (Settings is null)
        {
            var created = new SettingsWindow(core); Settings = created;
            created.AdvancedRequested += OpenAdvancedSettings;
            created.Closed += (_, _) =>
            {
                created.AdvancedRequested -= OpenAdvancedSettings;
                if (ReferenceEquals(Settings, created)) Settings = null;
            };
        }
        Settings.Activate();
    }

    internal void OpenAdvancedSettings()
    {
        if (quitting) return;
        if (AdvancedSettings is null)
        {
            var created = new AdvancedSettingsWindow(core); AdvancedSettings = created;
            created.Closed += (_, _) => { if (ReferenceEquals(AdvancedSettings, created)) AdvancedSettings = null; };
        }
        AdvancedSettings.Activate();
    }
    internal void CloseAdvancedSettings() => AdvancedSettings?.Close();
    internal void CloseSettings() => Settings?.Close();

    private void Apply(SettingsSnapshot snapshot)
    {
        MainRoot.RequestedTheme = ViewTheme.From(snapshot);
        HistoryWorkspace.CompactMode = snapshot.Values["app.compact_mode"] == "true";
        compactPreview.Apply(HistoryWorkspace.CompactMode, MainRoot.RequestedTheme);
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
