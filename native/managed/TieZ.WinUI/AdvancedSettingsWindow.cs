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
    private readonly TextBlock heading = new() { FontSize = 28, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold };
    private readonly TextBlock intro = new() { TextWrapping = TextWrapping.Wrap };
    private readonly TextBox kinds = new();
    private readonly TextBox privacy = new() { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 90 };
    private readonly TextBlock privacyHint = new() { TextWrapping = TextWrapping.Wrap, Opacity = 0.7 };
    private readonly CleanupRuleEditor cleanup = new();
    private readonly TextBlock rulesHint = new() { TextWrapping = TextWrapping.Wrap, Opacity = 0.7 };
    private readonly AppPolicyEditor policies = new();
    private readonly CleanupPreviewView cleanupPreview;
    private readonly Button save = new() { IsEnabled = false };
    private readonly Button reload = new();
    private readonly InfoBar feedback = new() { IsOpen = false, IsClosable = true };
    private readonly TextBlock revision = new() { Opacity = 0.6 };
    private Func<AdvancedStrings, string>? feedbackText;
    private ulong revisionValue;
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
    internal CleanupRuleEditor RulesEditor => cleanup;
    internal CleanupPreviewView RulePreview => cleanupPreview;
    internal string KindsDraft => kinds.Text;
    internal ElementTheme CurrentTheme => surface.RequestedTheme;

    public AdvancedSettingsWindow(NativeCore core)
    {
        this.core = core;
        cleanupPreview = new CleanupPreviewView(() => cleanup.Text);
        cleanupPreview.Initialize(core); policies.Initialize(core);
        LanguageService.Changed += ApplyLanguage;
        AppWindow.Resize(new SizeInt32(640, 800));
        panel.Children.Add(heading); panel.Children.Add(intro);
        panel.Children.Add(kinds); panel.Children.Add(privacy); panel.Children.Add(privacyHint);
        panel.Children.Add(cleanup); panel.Children.Add(rulesHint);
        panel.Children.Add(cleanupPreview); panel.Children.Add(policies);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        actions.Children.Add(save); actions.Children.Add(reload);
        panel.Children.Add(actions); panel.Children.Add(revision); panel.Children.Add(feedback);
        surface.Child = new ScrollViewer { Content = panel };
        surface.Style = (Style)Application.Current.Resources["NativeSettingsSurfaceStyle"];
        Content = surface;
        foreach (var input in new[] { kinds, privacy })
            input.TextChanging += (_, _) => MarkDirty();
        cleanup.Changed += MarkDirty;
        cleanup.Changed += cleanupPreview.Invalidate;
        policies.Changed += MarkDirty;
        save.Click += (_, _) => Save();
        reload.Click += (_, _) => Reload();
        core.Changed += OnChanged;
        Closed += (_, _) =>
        {
            closed = true; LanguageService.Changed -= ApplyLanguage;
            core.Changed -= OnChanged; policies.Changed -= MarkDirty; loaded = null;
            loading = true; kinds.Text = privacy.Text = "";
            cleanup.Changed -= MarkDirty; cleanup.Changed -= cleanupPreview.Invalidate; cleanup.Clear();
            cleanupPreview.Dispose(); policies.Clear();
        };
        Load(core.Snapshot);
    }
    private Dictionary<string, string> Values() => new()
    {
        ["app.privacy_protection_kinds"] = StoredDraft(kinds, "app.privacy_protection_kinds"),
        ["app.privacy_protection_custom_rules"] = StoredDraft(privacy, "app.privacy_protection_custom_rules"),
        ["app.cleanup_rules"] = loaded is not null && Lines(cleanup.Text) == Lines(loaded.Values["app.cleanup_rules"])
            ? loaded.Values["app.cleanup_rules"] : cleanup.Text,
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
        cleanupPreview.Invalidate();
        loading = true;
        try
        {
            loaded = snapshot;
            kinds.Text = snapshot.Values["app.privacy_protection_kinds"];
            privacy.Text = snapshot.Values["app.privacy_protection_custom_rules"];
            cleanup.Text = snapshot.Values["app.cleanup_rules"];
            policies.Load(snapshot.Values["app.app_cleanup_policies"]);
            surface.RequestedTheme = ViewTheme.From(snapshot);
            revisionValue = snapshot.Revision;
            ApplyLanguage();
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
            ShowStatus(InfoBarSeverity.Error, text => text.Conflict);
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
            if (core.LastSideEffectError is { } sideEffect) ShowStatus(InfoBarSeverity.Warning, text => string.Format(text.SavedWarningFormat, sideEffect));
            else ShowStatus(InfoBarSeverity.Success, text => text.Saved);
        }
        catch (Exception error)
        {
            save.IsEnabled = dirty;
            if (error is CoreException { Code: "revision_conflict" }) ShowStatus(InfoBarSeverity.Error, text => text.SaveConflict);
            else ShowError(error.Message);
        }
    }
    private void Reload()
    {
        if (closed) return;
        try { Load(core.Refresh()); feedback.IsOpen = false; }
        catch (Exception error) { ShowError(error.Message); }
    }
    private void ShowStatus(InfoBarSeverity severity, Func<AdvancedStrings, string> message)
    {
        feedbackText = message;
        feedback.Severity = severity;
        feedback.Message = message(AdvancedStrings.Current);
        feedback.IsOpen = true;
    }
    private void ShowError(string message)
    {
        // Retained core error text is data and is never translated.
        feedbackText = null;
        feedback.Severity = InfoBarSeverity.Error; feedback.Message = message; feedback.IsOpen = true;
    }
    // Re-derives this window and its three editors; drafts, revision and the open message severity stay put.
    internal void ApplyLanguage()
    {
        // Windows' own template parts follow the same accepted language.
        FrameworkChrome.ApplyDeferred(this);
        if (closed) return;
        var text = AdvancedStrings.Current;
        Title = text.WindowTitle;
        heading.Text = text.Heading;
        intro.Text = text.Intro;
        kinds.Header = text.KindsHeader;
        kinds.PlaceholderText = text.KindsPlaceholder;
        privacy.Header = text.PrivacyHeader;
        privacyHint.Text = text.PrivacyHint;
        rulesHint.Text = text.RulesExampleHint;
        save.Content = text.Save;
        reload.Content = text.Reload;
        revision.Text = string.Format(text.RevisionFormat, revisionValue);
        if (feedback.IsOpen && feedbackText is { } message) feedback.Message = message(text);
        cleanup.ApplyLanguage(); cleanupPreview.ApplyLanguage(); policies.ApplyLanguage();
    }
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
