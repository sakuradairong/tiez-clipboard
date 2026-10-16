using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

public sealed class MainWindow : Window
{
    private readonly NativeCore core;
    private readonly Grid root = new();
    private readonly TextBlock status = new() { TextWrapping = TextWrapping.Wrap, Opacity = 0.7 };
    private readonly TextBox search = new() { PlaceholderText = "搜索剪贴板记录", IsEnabled = false };
    internal SettingsWindow? Settings { get; private set; }
    internal bool SearchVisible => search.Visibility == Visibility.Visible;
    internal ElementTheme CurrentTheme => root.RequestedTheme;

    public MainWindow(NativeCore core, string directory)
    {
        this.core = core;
        Title = "TieZ · WinUI 原生原型";
        AppWindow.Resize(new SizeInt32(920, 640));
        root.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(220) });
        root.ColumnDefinitions.Add(new ColumnDefinition());
        var navigation = new StackPanel { Spacing = 20, Padding = new Thickness(24) };
        navigation.Children.Add(new TextBlock { Text = "TieZ", FontSize = 32, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        navigation.Children.Add(new TextBlock { Text = "原生迁移预览", Opacity = 0.6 });
        navigation.Children.Add(new TextBlock { Text = "剪贴板", FontSize = 18 });
        var settings = new Button { Content = "打开设置 ↗", HorizontalAlignment = HorizontalAlignment.Stretch };
        settings.Click += (_, _) => OpenSettings();
        navigation.Children.Add(settings);
        navigation.Children.Add(status);
        var content = new StackPanel { Spacing = 20, Padding = new Thickness(32) };
        content.Children.Add(new TextBlock { Text = "剪贴板", FontSize = 28, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        content.Children.Add(search);
        content.Children.Add(new InfoBar { IsOpen = true, IsClosable = false, Severity = InfoBarSeverity.Informational,
            Title = "设置窗口与主窗口已拆分", Message = "当前验证原生窗口和设置同步。剪贴板历史、采集、粘贴尚未接入。" });
        content.Children.Add(new TextBlock { Text = "等待迁移剪贴板服务", FontSize = 22, Margin = new Thickness(0, 48, 0, 0) });
        content.Children.Add(new TextBlock { Text = "这里不会读取现有 TieZ 的数据。", Opacity = 0.65 });
        content.Children.Add(new TextBlock { Text = "原型数据目录\n" + directory, TextWrapping = TextWrapping.Wrap, Opacity = 0.5 });
        root.Children.Add(navigation);
        Grid.SetColumn(content, 1);
        root.Children.Add(content);
        root.ActualThemeChanged += (_, _) => root.Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(
            root.ActualTheme == ElementTheme.Dark ? Microsoft.UI.ColorHelper.FromArgb(255, 30, 30, 30) : Microsoft.UI.ColorHelper.FromArgb(255, 250, 250, 250));
        Content = root;
        core.Changed += Apply;
        Closed += (_, _) => core.Changed -= Apply;
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
        root.RequestedTheme = ViewTheme.From(snapshot);
        search.Visibility = snapshot.Values["app.show_search_box"] == "true" ? Visibility.Visible : Visibility.Collapsed;
        status.Text = $"配置版本 {snapshot.Revision}\n语言 {snapshot.Values["app.language"]}\n去重 {snapshot.Values["app.deduplicate"]}";
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
