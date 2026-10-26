using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

public sealed class SettingsWindow : Window
{
    private static readonly string[] Languages = ["zh", "en", "tw"];
    private static readonly string[] Themes = ["system", "light", "dark"];
    private readonly NativeCore core;
    private readonly StackPanel panel = new() { Spacing = 18, Padding = new Thickness(28) };
    private readonly ComboBox language = new() { Header = "语言（配置值）", ItemsSource = new[] { "简体中文", "English", "繁體中文" }, HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly ComboBox theme = new() { Header = "外观", ItemsSource = new[] { "跟随系统", "浅色", "深色" }, HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly TextBox hotkey = new() { Header = "主窗口快捷键（留空禁用）", PlaceholderText = "Ctrl+Alt+V" };
    private readonly ToggleSwitch deduplicate = new() { Header = "自动去重" };
    private readonly ToggleSwitch showSearch = new() { Header = "显示主窗口搜索框" };
    private readonly ToggleSwitch compact = new() { Header = "紧凑列表（鼠标悬停 1 秒显示预览）" };
    private readonly ToggleSwitch persistent = new() { Header = "保存采集历史到磁盘", OffContent = "仅保留本次会话", OnContent = "持久化保存" };
    private readonly ToggleSwitch captureFiles = new() { Header = "采集文件路径" };
    private readonly ToggleSwitch captureRichText = new() { Header = "采集富文本和表格原始格式" };
    private readonly ToggleSwitch privacy = new() { Header = "自动标记敏感内容" };
    private readonly ToggleSwitch limitEnabled = new() { Header = "限制磁盘历史数量（置顶和有标签条目不计入）" };
    private readonly TextBox limit = new() { Header = "最多保留普通历史条数（0–1000000）" };
    private readonly Button advanced = new() { Content = "高级设置 · 隐私与清理规则" };
    internal event Action? AdvancedRequested;
    private readonly OpenerSettingsEditor openers;
    private readonly Expander openerSection = new() { Header = "打开记录使用的应用", HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly Button reload = new() { Content = "重新载入" };
    internal OpenerSettingsEditor Openers => openers;
    private readonly Button save = new() { Content = "保存设置", IsEnabled = false };
    private readonly InfoBar message = new() { IsOpen = false, IsClosable = true };
    private readonly TextBlock revision = new() { Opacity = 0.6 };
    private bool loading;
    private bool dirty;
    private ulong expectedRevision;
    private SettingsSnapshot? loadedSnapshot;
    internal ulong DisplayedRevision => expectedRevision;

    public SettingsWindow(NativeCore core)
    {
        this.core = core;
        openers = new OpenerSettingsEditor(() => WinRT.Interop.WindowNative.GetWindowHandle(this), core.GetApplicationCatalogAsync) { IconReader = core.GetApplicationIconAsync };
        Title = "TieZ · 设置";
        AppWindow.Resize(new SizeInt32(520, 740));
        panel.Children.Add(new TextBlock { Text = "设置", FontSize = 28, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        panel.Children.Add(new TextBlock { Text = "原生窗口 · 与主窗口共享同一配置核心", Opacity = 0.6 });
        panel.Children.Add(language); panel.Children.Add(theme); panel.Children.Add(hotkey);
        ToolTipService.SetToolTip(hotkey, "Win+V 可能被 Windows 占用。支持键盘组合；留空可禁用。");
        panel.Children.Add(deduplicate); panel.Children.Add(showSearch); panel.Children.Add(compact);
        panel.Children.Add(persistent); panel.Children.Add(captureFiles); panel.Children.Add(captureRichText);
        panel.Children.Add(privacy); panel.Children.Add(limitEnabled); panel.Children.Add(limit);
        openerSection.Content = openers; panel.Children.Add(openerSection);
        openers.Changed += MarkDirty; openers.Error += ShowError;
        panel.Children.Add(advanced);
        advanced.Click += (_, _) => AdvancedRequested?.Invoke();
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        actions.Children.Add(save); actions.Children.Add(reload);
        panel.Children.Add(actions); panel.Children.Add(revision); panel.Children.Add(message);
        panel.ActualThemeChanged += (_, _) => panel.Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(
            panel.ActualTheme == ElementTheme.Dark ? Microsoft.UI.ColorHelper.FromArgb(255, 30, 30, 30) : Microsoft.UI.ColorHelper.FromArgb(255, 250, 250, 250));
        Content = new ScrollViewer { Content = panel };
        language.SelectionChanged += (_, _) => MarkDirty();
        theme.SelectionChanged += (_, _) => MarkDirty();
        deduplicate.Toggled += (_, _) => MarkDirty();
        showSearch.Toggled += (_, _) => MarkDirty();
        compact.Toggled += (_, _) => MarkDirty();
        foreach (var toggle in new[] { persistent, captureFiles, captureRichText, privacy, limitEnabled })
            toggle.Toggled += (_, _) => MarkDirty();
        foreach (var input in new[] { hotkey, limit })
            // TextChanged is deferred; drafts must be current before Save or an external snapshot.
            input.TextChanging += (_, _) => MarkDirty();
        save.Click += (_, _) => Save();
        reload.Click += (_, _) =>
        {
            try { Load(core.Refresh()); message.IsOpen = false; }
            catch (Exception e) { ShowError(e.Message); }
        };
        core.Changed += OnChanged;
        Closed += (_, _) =>
        {
            core.Changed -= OnChanged; loadedSnapshot = null; dirty = false; save.IsEnabled = false;
            openers.Changed -= MarkDirty; openers.Error -= ShowError; openers.Dispose();
        };
        Load(core.Snapshot);
    }

    private void MarkDirty()
    {
        if (loading || loadedSnapshot is null) return;
        dirty = CurrentValues().Any(pair => loadedSnapshot.Values[pair.Key] != pair.Value);
        save.IsEnabled = dirty && !openers.IsPicking;
        if (dirty && core.Snapshot.Revision != expectedRevision) ShowError("配置已变化。请重新载入后再修改；当前草稿尚未保存。");
        else message.IsOpen = false;
    }

    private void Load(SettingsSnapshot snapshot)
    {
        loading = true;
        try
        {
            expectedRevision = snapshot.Revision;
            loadedSnapshot = snapshot;
            openers.Bind(snapshot);
            hotkey.Text = snapshot.Values["app.hotkey"];
            language.SelectedIndex = Array.IndexOf(Languages, snapshot.Values["app.language"]);
            theme.SelectedIndex = Array.IndexOf(Themes, snapshot.Values["app.color_mode"]);
            deduplicate.IsOn = snapshot.Values["app.deduplicate"] == "true";
            showSearch.IsOn = snapshot.Values["app.show_search_box"] == "true";
            compact.IsOn = snapshot.Values["app.compact_mode"] == "true";
            persistent.IsOn = snapshot.Values["app.persistent"] == "true";
            captureFiles.IsOn = snapshot.Values["app.capture_files"] == "true";
            captureRichText.IsOn = snapshot.Values["app.capture_rich_text"] == "true";
            privacy.IsOn = snapshot.Values["app.privacy_protection"] == "true";
            limitEnabled.IsOn = snapshot.Values["app.persistent_limit_enabled"] == "true";
            limit.Text = snapshot.Values["app.persistent_limit"];
            panel.RequestedTheme = ViewTheme.From(snapshot);
            revision.Text = $"已保存配置版本 {snapshot.Revision}";
            dirty = false; save.IsEnabled = false;
        }
        finally { loading = false; }
    }

    private void OnChanged(SettingsSnapshot snapshot)
    {
        panel.RequestedTheme = ViewTheme.From(snapshot);
        if (dirty && snapshot.Revision != expectedRevision)
        {
            openers.InvalidatePickerChoice();
            ShowError("配置已变化。请重新载入后再修改；当前草稿尚未保存。");
            return;
        }
        Load(snapshot);
    }

    private Dictionary<string, string> CurrentValues()
    {
        var values = new Dictionary<string, string>
        {
            ["app.hotkey"] = hotkey.Text,
            ["app.language"] = Languages[language.SelectedIndex],
            ["app.color_mode"] = Themes[theme.SelectedIndex],
            ["app.deduplicate"] = deduplicate.IsOn ? "true" : "false",
            ["app.show_search_box"] = showSearch.IsOn ? "true" : "false",
            ["app.compact_mode"] = compact.IsOn ? "true" : "false",
            ["app.persistent"] = persistent.IsOn ? "true" : "false",
            ["app.capture_files"] = captureFiles.IsOn ? "true" : "false",
            ["app.capture_rich_text"] = captureRichText.IsOn ? "true" : "false",
            ["app.privacy_protection"] = privacy.IsOn ? "true" : "false",
            ["app.persistent_limit_enabled"] = limitEnabled.IsOn ? "true" : "false",
            ["app.persistent_limit"] = limit.Text,
        };
        foreach (var (key, value) in openers.Values()) values[key] = value;
        return values;
    }

    private void Save()
    {
        if (openers.IsPicking) return;
        save.IsEnabled = false;
        try
        {
            core.Update(CurrentValues(), expectedRevision);
            Load(core.Snapshot);
            message.Severity = core.LastSideEffectError is null ? InfoBarSeverity.Success : InfoBarSeverity.Warning;
            message.Message = core.LastSideEffectError is null ? "设置已保存，主窗口已同步。"
                : $"设置已保存，但快捷键未完整应用：{core.LastSideEffectError}。可退出并重新打开应用恢复。";
            message.IsOpen = true;
        }
        catch (Exception e)
        {
            save.IsEnabled = true;
            ShowError(e is CoreException { Code: "hotkey_conflict" }
                ? "快捷键已被系统或其他应用占用。原配置和绑定保持，当前草稿已保留。" : e.Message);
        }
    }

    private void ShowError(string text)
    {
        message.Severity = InfoBarSeverity.Error; message.Message = text; message.IsOpen = true;
    }

    internal ElementTheme CurrentTheme => panel.RequestedTheme;
    internal bool IsContentReady => panel.IsLoaded && panel.XamlRoot is not null;
    internal bool HasError => message.IsOpen && message.Severity == InfoBarSeverity.Error;
    internal string DraftLimit => limit.Text;
    internal string DraftHotkey => hotkey.Text;
    internal void SetSmokeHotkey(string value) => hotkey.Text = value;
    internal bool CompactMode => compact.IsOn;
    internal void SetCompactMode(bool enabled) => compact.IsOn = enabled;
    internal bool HasDraft => dirty && save.IsEnabled;
    internal void SetSmokeLimit(string value) => limit.Text = value;
    internal void OpenAdvanced()
    {
        ((IInvokeProvider)new ButtonAutomationPeer(advanced).GetPattern(PatternInterface.Invoke)).Invoke();
    }
    internal void InvokeSettingsSave() => InvokeSaveButton();
    internal void ShowOpenerSettings() => openerSection.IsExpanded = true;
    internal void InvokeSettingsReload() => ((IInvokeProvider)new ButtonAutomationPeer(reload).GetPattern(PatternInterface.Invoke)).Invoke();

    internal bool CaptureOptionsLoaded => captureFiles.IsOn && captureRichText.IsOn && !persistent.IsOn
        && privacy.IsOn && limitEnabled.IsOn && limit.Text == "500";

    internal void InvokeSmokeSave()
    {
        language.SelectedIndex = 2; theme.SelectedIndex = 2;
        deduplicate.IsOn = false; showSearch.IsOn = false;
        captureFiles.IsOn = true; captureRichText.IsOn = true;
        InvokeSaveButton();
    }

    private void InvokeSaveButton()
    {
        var peer = new ButtonAutomationPeer(save);
        ((IInvokeProvider)peer.GetPattern(PatternInterface.Invoke)).Invoke();
    }
}
