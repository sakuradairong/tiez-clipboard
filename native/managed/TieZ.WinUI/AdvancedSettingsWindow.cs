using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

// Borrows the shared core; owns only its rule drafts and view subscriptions.
public sealed class AdvancedSettingsWindow : Window
{
    private readonly NativeCore core;
    private readonly Border surface = new();
    private readonly StackPanel panel = new() { Spacing = 18, Padding = new Thickness(28) };
    private readonly TextBox kinds = new() { Header = "敏感匹配类别（逗号分隔）", PlaceholderText = "phone,idcard,email,secret" };
    private readonly TextBox privacy = new() { Header = "自定义敏感正则（每行一条）", AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 90 };
    private readonly TextBox cleanup = new() { Header = "清理规则（每行：正则 => 替换文本）", AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 90 };
    private readonly AppPolicyEditor policies = new();
    private readonly Button save = new() { Content = "保存高级设置", IsEnabled = false };
    private readonly Button reload = new() { Content = "放弃草稿并重新载入" };
    private readonly InfoBar feedback = new() { IsOpen = false, IsClosable = true };
    private readonly TextBlock revision = new() { Opacity = 0.6 };
    private SettingsSnapshot? loaded;
    private bool loading, dirty, closed;
    internal ulong DisplayedRevision => loaded?.Revision ?? 0;
    internal bool HasDraft => dirty && save.IsEnabled;
    internal bool HasError => feedback.IsOpen && feedback.Severity == InfoBarSeverity.Error;
    internal bool IsContentReady => panel.IsLoaded && panel.XamlRoot is not null;
    internal string CleanupDraft => cleanup.Text;
    internal string PrivacyDraft => privacy.Text;
    internal string PolicyDraft => policies.Text;
    internal AppPolicyEditor PolicyEditor => policies;
    internal string KindsDraft => kinds.Text;
    internal ElementTheme CurrentTheme => surface.RequestedTheme;

    public AdvancedSettingsWindow(NativeCore core)
    {
        this.core = core;
        Title = "TieZ · 高级设置";
        AppWindow.Resize(new SizeInt32(640, 800));
        panel.Children.Add(new TextBlock { Text = "高级设置", FontSize = 28, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        panel.Children.Add(new TextBlock { Text = "规则在保存时由核心校验。关闭此窗口不影响剪贴板采集。", TextWrapping = TextWrapping.Wrap });
        panel.Children.Add(kinds); panel.Children.Add(privacy);
        panel.Children.Add(new TextBlock { Text = "敏感规则使用正则表达式，一行一条；留空表示不添加自定义规则。", TextWrapping = TextWrapping.Wrap, Opacity = 0.7 });
        panel.Children.Add(cleanup);
        panel.Children.Add(new TextBlock { Text = "例如：foo => bar。空行和以 # 开头的注释行会忽略。", TextWrapping = TextWrapping.Wrap, Opacity = 0.7 });
        panel.Children.Add(policies);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        actions.Children.Add(save); actions.Children.Add(reload);
        panel.Children.Add(actions); panel.Children.Add(revision); panel.Children.Add(feedback);
        surface.Child = new ScrollViewer { Content = panel };
        surface.Style = (Style)Application.Current.Resources["NativeSettingsSurfaceStyle"];
        Content = surface;
        foreach (var input in new[] { kinds, privacy, cleanup })
            input.TextChanging += (_, _) => MarkDirty();
        policies.Changed += MarkDirty;
        save.Click += (_, _) => Save();
        reload.Click += (_, _) => Reload();
        core.Changed += OnChanged;
        Closed += (_, _) =>
        {
            closed = true; core.Changed -= OnChanged; policies.Changed -= MarkDirty; loaded = null;
            loading = true; kinds.Text = privacy.Text = cleanup.Text = "";
            policies.Clear();
        };
        Load(core.Snapshot);
    }
    private Dictionary<string, string> Values() => new()
    {
        ["app.privacy_protection_kinds"] = StoredDraft(kinds, "app.privacy_protection_kinds"),
        ["app.privacy_protection_custom_rules"] = StoredDraft(privacy, "app.privacy_protection_custom_rules"),
        ["app.cleanup_rules"] = StoredDraft(cleanup, "app.cleanup_rules"),
        ["app.app_cleanup_policies"] = loaded is not null && Lines(policies.Text) == Lines(loaded.Values["app.app_cleanup_policies"])
            ? loaded.Values["app.app_cleanup_policies"] : policies.Text,
    };
    private static string Lines(string value) => value.Replace("\r\n", "\n").Replace("\r", "\n");
    private string StoredDraft(TextBox input, string key) => loaded is not null
        && Lines(input.Text) == Lines(loaded.Values[key]) ? loaded.Values[key] : input.Text;
    private void MarkDirty()
    {
        if (closed || loading || loaded is null) return;
        dirty = Values().Any(pair => loaded.Values[pair.Key] != pair.Value);
        save.IsEnabled = dirty; feedback.IsOpen = false;
    }
    private void Load(SettingsSnapshot snapshot)
    {
        if (closed) return;
        loading = true;
        try
        {
            loaded = snapshot;
            kinds.Text = snapshot.Values["app.privacy_protection_kinds"];
            privacy.Text = snapshot.Values["app.privacy_protection_custom_rules"];
            cleanup.Text = snapshot.Values["app.cleanup_rules"];
            policies.Load(snapshot.Values["app.app_cleanup_policies"]);
            surface.RequestedTheme = ViewTheme.From(snapshot);
            revision.Text = $"已保存配置版本 {snapshot.Revision}";
            dirty = false; save.IsEnabled = false;
        }
        finally { loading = false; }
    }
    private void OnChanged(SettingsSnapshot snapshot)
    {
        if (closed) return;
        // Theme follows accepted settings even when the rule draft is conflicted.
        surface.RequestedTheme = ViewTheme.From(snapshot);
        if (dirty && snapshot.Revision != DisplayedRevision)
        {
            ShowError("配置已在其他窗口变化。当前草稿已保留，请重新载入后再修改。");
            return;
        }
        Load(snapshot);
    }
    private void Save()
    {
        if (closed || loaded is null) return;
        save.IsEnabled = false;
        try
        {
            core.Update(Values(), loaded.Revision);
            Load(core.Snapshot);
            feedback.Severity = core.LastSideEffectError is null ? InfoBarSeverity.Success : InfoBarSeverity.Warning;
            feedback.Message = core.LastSideEffectError is null ? "高级设置已保存并应用于采集。"
                : $"高级设置已保存，但运行时应用未完整完成：{core.LastSideEffectError}";
            feedback.IsOpen = true;
        }
        catch (Exception error)
        {
            save.IsEnabled = dirty;
            ShowError(error is CoreException { Code: "revision_conflict" }
                ? "配置已在其他窗口变化，未保存。当前草稿已保留，请重新载入后再修改。" : error.Message);
        }
    }
    private void Reload()
    {
        if (closed) return;
        try { Load(core.Refresh()); feedback.IsOpen = false; }
        catch (Exception error) { ShowError(error.Message); }
    }
    private void ShowError(string message)
    { feedback.Severity = InfoBarSeverity.Error; feedback.Message = message; feedback.IsOpen = true; }
    internal void SetRules(string kindValue, string privacyValue, string cleanupValue, string policyValue)
    { kinds.Text = kindValue; privacy.Text = privacyValue; cleanup.Text = cleanupValue; policies.Text = policyValue; }
    internal void InvokeSave() => Invoke(save);
    internal void InvokeReload() => Invoke(reload);
    private static void Invoke(Button button)
    {
        if (!button.IsEnabled) throw new InvalidOperationException("Advanced settings action is disabled.");
        ((IInvokeProvider)new ButtonAutomationPeer(button).GetPattern(PatternInterface.Invoke)).Invoke();
    }
}
