using Microsoft.UI.Xaml;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

public sealed partial class MainWindow : Window
{
    private readonly NativeCore core;
    private readonly CompactPreviewController compactPreview;
    internal CompactPreviewController CompactPreview => compactPreview;
    private bool residentMode, quitting, active, tagPageActive;
    private bool shutdownComplete, workspacesStopped;
    private Task<bool>? quitTask;
    private readonly OpenLifetimeController openLifetime;
    private readonly EdgeDockController edgeDock;
    internal event Action? ShutdownStarted;
    internal OpenLifetimeController OpenLifetime => openLifetime;
    internal bool ShutdownPending => quitting;
    internal bool ShutdownComplete => shutdownComplete;
    internal Task<bool>? ShutdownTask => quitTask;
    internal string ExitMessage => ExitError.Message;
    internal bool ExitRetryEnabled => QuitButton.IsEnabled && quitting && !shutdownComplete;
    internal SettingsWindow? Settings { get; private set; }
    internal AdvancedSettingsWindow? AdvancedSettings { get; private set; }
    internal bool SearchVisible => HistoryWorkspace.SearchVisible;
    internal ElementTheme CurrentTheme => MainRoot.RequestedTheme;
    internal HistoryView History => HistoryWorkspace;
    internal TagManagementView Tags => TagWorkspace;

    public MainWindow(NativeCore core, string directory, bool captureOnStartup = true)
    {
        this.core = core;
        InitializeComponent();
        compactPreview = new CompactPreviewController(this, core);
        openLifetime = new OpenLifetimeController(this, core);
        HistoryWorkspace.PreviewRequested += compactPreview.Request;
        HistoryWorkspace.IsExplicitPreviewActive = () => compactPreview.IsExplicitActive;
        HistoryWorkspace.PreviewInvalidated += compactPreview.Cancel;
        Title = ShellStrings.Current.WindowTitle;
        AppWindow.Resize(new SizeInt32(1080, 760));
        Activated += (_, args) =>
        {
            active = args.WindowActivationState != WindowActivationState.Deactivated;
            if (!active) compactPreview.Cancel();
        };
        AppWindow.Closing += (sender, args) =>
        {
            if (shutdownComplete) return;
            args.Cancel = true;
            if (residentMode && !quitting) HideToTray(); else _ = Quit();
        };
        AppWindow.Changed += (_, _) => HistoryWorkspace.SetWorkspaceActive(AppWindow.IsVisible && !tagPageActive && !quitting
            && (AppWindow.Presenter is not Microsoft.UI.Windowing.OverlappedPresenter presenter
                || presenter.State != Microsoft.UI.Windowing.OverlappedPresenterState.Minimized));
        HideButton.Click += (_, _) => HideToTray();
        QuitButton.Click += async (_, _) => await Quit();
        RetryExitButton.Click += (_, _) => _ = Quit();
        Navigation.Loaded += (_, _) => { ApplyLanguage(); UpdateNavigationChrome(); };
        Navigation.SizeChanged += (_, _) => UpdateNavigationChrome();
        Navigation.PaneOpened += (_, _) => UpdateNavigationChrome();
        Navigation.PaneClosed += (_, _) => UpdateNavigationChrome();
        Navigation.DisplayModeChanged += (_, _) => UpdateNavigationChrome();
        UpdateNavigationChrome();
        Navigation.SelectedItem = HistoryNav;
        Navigation.SelectionChanged += (_, args) =>
        {
            if (args.IsSettingsSelected)
            {
                OpenSettings(); Navigation.SelectedItem = tagPageActive ? TagsNav : HistoryNav; return;
            }
            tagPageActive = ReferenceEquals(args.SelectedItem, TagsNav);
            HistoryWorkspace.CancelPreview();
            HistoryWorkspace.SetWorkspaceActive(!tagPageActive && AppWindow.IsVisible);
            HistoryWorkspace.Visibility = tagPageActive ? Visibility.Collapsed : Visibility.Visible;
            TagWorkspace.Visibility = tagPageActive ? Visibility.Visible : Visibility.Collapsed;
            TagWorkspace.SetActive(tagPageActive && AppWindow.IsVisible);
        };
        TagWorkspace.Initialize(core); TagWorkspace.FilterRequested += FilterByTag;
        HistoryWorkspace.Initialize(core, captureOnStartup);
        edgeDock = new EdgeDockController(this, core, () => HistoryWorkspace.HasDialog, "main");
        ShutdownStarted += edgeDock.Dispose;
        core.Changed += Apply;
        Closed += (_, _) => { openLifetime.Dispose(); StopWorkspaces(); };
        Apply(core.Snapshot);
    }

    internal void SetResidentMode(bool enabled)
    {
        residentMode = enabled && !quitting;
        HideButton.IsEnabled = residentMode;
    }
    private bool adjustingNavigation;
    private void SetExitProgress(string text)
    {
        ExitProgress.Text = text;
        UpdateNavigationChrome();
    }
    private void UpdateNavigationChrome()
    {
        if (adjustingNavigation) return;
        adjustingNavigation = true;
        try
        {
            // Auto keeps a narrow footer when the overlay opens. A measured wide window uses the full left pane.
            if (Navigation.ActualWidth >= 1200)
            {
                if (Navigation.PaneDisplayMode != Microsoft.UI.Xaml.Controls.NavigationViewPaneDisplayMode.Left)
                    Navigation.PaneDisplayMode = Microsoft.UI.Xaml.Controls.NavigationViewPaneDisplayMode.Left;
                if (!Navigation.IsPaneOpen) Navigation.IsPaneOpen = true;
            }
            else if (Navigation.ActualWidth > 0)
            {
                if (Navigation.PaneDisplayMode != Microsoft.UI.Xaml.Controls.NavigationViewPaneDisplayMode.LeftCompact)
                    Navigation.PaneDisplayMode = Microsoft.UI.Xaml.Controls.NavigationViewPaneDisplayMode.LeftCompact;
                if (Navigation.IsPaneOpen) Navigation.IsPaneOpen = false;
            }
            bool labels = Navigation.DisplayMode == Microsoft.UI.Xaml.Controls.NavigationViewDisplayMode.Expanded
                || Navigation.PaneDisplayMode == Microsoft.UI.Xaml.Controls.NavigationViewPaneDisplayMode.Left;
            var labelVisibility = labels ? Visibility.Visible : Visibility.Collapsed;
            HideLabel.Visibility = QuitLabel.Visibility = PaneTitle.Visibility = MigrationNote.Visibility = labelVisibility;
            ExitProgress.Visibility = labels && ExitProgress.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
            var alignment = labels ? HorizontalAlignment.Left : HorizontalAlignment.Center;
            HideButton.HorizontalContentAlignment = QuitButton.HorizontalContentAlignment = alignment;
            var padding = labels ? new Thickness(12, 8, 12, 8) : new Thickness(4);
            HideButton.Padding = QuitButton.Padding = padding;
            LifecyclePane.Margin = labels ? new Thickness(8, 4, 8, 12) : new Thickness(4, 8, 4, 8);
            LifecyclePane.Width = labels ? double.NaN : Math.Max(32, Navigation.CompactPaneLength - 8);
        }
        finally { adjustingNavigation = false; }
    }
    internal void SetShellUnavailable(string message)
    {
        ShellError.Message = message; ShellError.IsOpen = true;
    }
    internal void HideToTray()
    {
        compactPreview.Cancel();
        if (residentMode && !quitting) { HistoryWorkspace.SetWorkspaceActive(false); HistoryWorkspace.DismissTagPicker(); TagWorkspace.SetActive(false); AppWindow.Hide(); }
    }
    internal void ToggleMain()
    {
        if (AppWindow.IsVisible && active && residentMode) HideToTray(); else ShowMain();
    }
    internal void ClearShellWarning() => ShellError.IsOpen = false;
    internal void ShowMain()
    {
        if (shutdownComplete) return;
        if (AppWindow.Presenter is Microsoft.UI.Windowing.OverlappedPresenter presenter
            && presenter.State == Microsoft.UI.Windowing.OverlappedPresenterState.Minimized) presenter.Restore();
        AppWindow.Show(); Activate();
        if (!quitting) { TagWorkspace.SetActive(tagPageActive); HistoryWorkspace.SetWorkspaceActive(!tagPageActive); }
    }
    internal Task<bool> Quit()
    {
        if (shutdownComplete) return Task.FromResult(true);
        if (quitTask is { IsCompleted: false }) return quitTask;
        quitTask = QuitAsync();
        return quitTask;
    }
    private async Task<bool> QuitAsync()
    {
        quitting = true;
        SetResidentMode(false);
        QuitButton.IsEnabled = RetryExitButton.IsEnabled = false;
        SetExitProgress(ShellStrings.Current.Exiting);
        ExitError.Title = ShellStrings.Current.ExitTitle; ExitError.Message = ShellStrings.Current.ExitStoppingMessage;
        ExitError.Severity = Microsoft.UI.Xaml.Controls.InfoBarSeverity.Informational;
        ExitError.IsOpen = true;
        ShowMain();
        HistoryNav.IsEnabled = TagsNav.IsEnabled = false; Navigation.IsSettingsVisible = false;
        CloseAdvancedSettings(); CloseSettings();
        if (!workspacesStopped) { ShutdownStarted?.Invoke(); openLifetime.Dispose(); StopWorkspaces(); }
        try
        {
            await core.DisposeAsync();
            shutdownComplete = true;
            Close();
            return true;
        }
        catch (Exception error)
        {
            ExitError.Title = ShellStrings.Current.ExitFailedTitle;
            ExitError.Message = error is CoreException { Code: "open_cleanup" }
                ? ShellStrings.Current.ExitLockedMessage
                : ShellStrings.Current.ExitFailedMessage;
            ExitError.Severity = Microsoft.UI.Xaml.Controls.InfoBarSeverity.Error;
            SetExitProgress(ShellStrings.Current.ExitRetry); QuitButton.IsEnabled = RetryExitButton.IsEnabled = true;
            ShowMain();
            return false;
        }
    }
    private void StopWorkspaces()
    {
        if (workspacesStopped) return;
        workspacesStopped = true;
        core.Changed -= Apply;
        HistoryWorkspace.PreviewRequested -= compactPreview.Request;
        HistoryWorkspace.PreviewInvalidated -= compactPreview.Cancel;
        compactPreview.Dispose();
        TagWorkspace.FilterRequested -= FilterByTag; TagWorkspace.Dispose();
        HistoryWorkspace.Dispose();
    }
    internal void SetOpenCleanupWarning(string? code)
    {
        if (quitting) return;
        OpenCleanupError.Message = code is null ? "" : ShellStrings.Current.OpenCleanupMessage;
        OpenCleanupError.IsOpen = code is not null;
    }
    internal bool ResidentMode => residentMode;
    internal void NavigateTags() => Navigation.SelectedItem = TagsNav;
    internal void NavigateHistory() => Navigation.SelectedItem = HistoryNav;
    private void FilterByTag(string name) { NavigateHistory(); HistoryWorkspace.ShowTag(name); }

    internal void OpenSettings()
    {
        if (quitting) return;
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
        // Every live surface follows the accepted language; unchanged values notify nobody.
        LanguageService.Apply(snapshot.Values["app.language"]);
        ApplyLanguage();
        MainRoot.RequestedTheme = ViewTheme.From(snapshot);
        HistoryWorkspace.CompactMode = snapshot.Values["app.compact_mode"] == "true";
        HistoryWorkspace.ApplyViewPreferences(snapshot);
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
