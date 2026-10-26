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
        Title = "TieZ · 原生迁移版";
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
        QuitButton.Click += (_, _) => _ = Quit();
        RetryExitButton.Click += (_, _) => _ = Quit();
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
        core.Changed += Apply;
        Closed += (_, _) => { openLifetime.Dispose(); StopWorkspaces(); };
        Apply(core.Snapshot);
    }

    internal void SetResidentMode(bool enabled)
    {
        residentMode = enabled && !quitting;
        HideButton.IsEnabled = residentMode;
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
        QuitButton.Content = "正在退出…";
        ExitError.Title = "正在退出"; ExitError.Message = "正在停止后台服务并清理临时文件，请稍候。";
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
            ExitError.Title = "退出尚未完成";
            ExitError.Message = error is CoreException { Code: "open_cleanup" }
                ? "临时导出文件仍被外部应用占用。请先关闭对应编辑器，再选择“重试退出”。剪贴板采集和其他后台服务已停止，临时文件仍由核心跟踪。"
                : "核心资源释放失败。请重试退出；当前窗口会保留，避免丢失清理重试入口。";
            ExitError.Severity = Microsoft.UI.Xaml.Controls.InfoBarSeverity.Error;
            QuitButton.Content = "重试退出"; QuitButton.IsEnabled = RetryExitButton.IsEnabled = true;
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
        OpenCleanupError.Message = code is null ? "" : "部分临时导出文件尚未清理。关闭使用这些文件的编辑器后，应用将自动重试；原始历史内容保持不变。";
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
