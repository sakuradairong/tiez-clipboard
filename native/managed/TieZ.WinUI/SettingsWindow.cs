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
    private readonly ToggleSwitch deduplicate = new() { Header = "自动去重（待接入采集服务）" };
    private readonly ToggleSwitch showSearch = new() { Header = "显示主窗口搜索框" };
    private readonly Button save = new() { Content = "保存设置", IsEnabled = false };
    private readonly InfoBar message = new() { IsOpen = false, IsClosable = true };
    private readonly TextBlock revision = new() { Opacity = 0.6 };
    private bool loading;
    private bool dirty;
    private ulong expectedRevision;
    internal ulong DisplayedRevision => expectedRevision;

    public SettingsWindow(NativeCore core)
    {
        this.core = core;
        Title = "TieZ · 设置";
        AppWindow.Resize(new SizeInt32(520, 740));
        panel.Children.Add(new TextBlock { Text = "设置", FontSize = 28, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        panel.Children.Add(new TextBlock { Text = "原生窗口 · 与主窗口共享同一配置核心", Opacity = 0.6 });
        panel.Children.Add(language); panel.Children.Add(theme);
        panel.Children.Add(deduplicate); panel.Children.Add(showSearch);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        var reload = new Button { Content = "重新载入" };
        actions.Children.Add(save); actions.Children.Add(reload);
        panel.Children.Add(actions); panel.Children.Add(revision); panel.Children.Add(message);
        panel.ActualThemeChanged += (_, _) => panel.Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(
            panel.ActualTheme == ElementTheme.Dark ? Microsoft.UI.ColorHelper.FromArgb(255, 30, 30, 30) : Microsoft.UI.ColorHelper.FromArgb(255, 250, 250, 250));
        Content = new ScrollViewer { Content = panel };
        language.SelectionChanged += (_, _) => MarkDirty();
        theme.SelectionChanged += (_, _) => MarkDirty();
        deduplicate.Toggled += (_, _) => MarkDirty();
        showSearch.Toggled += (_, _) => MarkDirty();
        save.Click += (_, _) => Save();
        reload.Click += (_, _) =>
        {
            try { Load(core.Refresh()); message.IsOpen = false; }
            catch (Exception e) { ShowError(e.Message); }
        };
        core.Changed += OnChanged;
        Closed += (_, _) => core.Changed -= OnChanged;
        Load(core.Snapshot);
    }

    private void MarkDirty()
    {
        if (loading) return;
        dirty = true; save.IsEnabled = true; message.IsOpen = false;
    }

    private void Load(SettingsSnapshot snapshot)
    {
        loading = true;
        try
        {
            expectedRevision = snapshot.Revision;
            language.SelectedIndex = Array.IndexOf(Languages, snapshot.Values["app.language"]);
            theme.SelectedIndex = Array.IndexOf(Themes, snapshot.Values["app.color_mode"]);
            deduplicate.IsOn = snapshot.Values["app.deduplicate"] == "true";
            showSearch.IsOn = snapshot.Values["app.show_search_box"] == "true";
            panel.RequestedTheme = ViewTheme.From(snapshot);
            revision.Text = $"已保存配置版本 {snapshot.Revision}";
            dirty = false; save.IsEnabled = false;
        }
        finally { loading = false; }
    }

    private void OnChanged(SettingsSnapshot snapshot)
    {
        if (dirty && snapshot.Revision != expectedRevision)
        {
            ShowError("配置已变化。请重新载入后再修改；当前草稿尚未保存。");
            return;
        }
        Load(snapshot);
    }

    private void Save()
    {
        save.IsEnabled = false;
        try
        {
            core.Update(new Dictionary<string, string>
            {
                ["app.language"] = Languages[language.SelectedIndex],
                ["app.color_mode"] = Themes[theme.SelectedIndex],
                ["app.deduplicate"] = deduplicate.IsOn ? "true" : "false",
                ["app.show_search_box"] = showSearch.IsOn ? "true" : "false"
            }, expectedRevision);
            Load(core.Snapshot);
            message.Severity = InfoBarSeverity.Success;
            message.Message = "设置已保存，主窗口已同步。"; message.IsOpen = true;
        }
        catch (Exception e)
        {
            save.IsEnabled = true;
            ShowError(e.Message);
        }
    }

    private void ShowError(string text)
    {
        message.Severity = InfoBarSeverity.Error; message.Message = text; message.IsOpen = true;
    }

    internal void InvokeSmokeSave()
    {
        language.SelectedIndex = 2; theme.SelectedIndex = 2;
        deduplicate.IsOn = false; showSearch.IsOn = false;
        var peer = new ButtonAutomationPeer(save);
        ((IInvokeProvider)peer.GetPattern(PatternInterface.Invoke)).Invoke();
    }
}
