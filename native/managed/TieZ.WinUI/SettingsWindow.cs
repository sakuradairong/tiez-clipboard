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
    private static readonly string[] QuickPasteModifiers = ["disabled", "ctrl", "alt", "shift", "win"];
    private readonly NativeCore core;
    private readonly Border surface = new();
    private readonly StackPanel panel = new() { Spacing = 12, Padding = new Thickness(20, 16, 20, 20) };
    private readonly TextBlock appearanceHeading = new(), captureHeading = new(), storageHeading = new();
    private ulong revisionValue;
    private TextBlock headingText = new();
    private TextBlock intro = new();

    private void ApplyLanguage()
    {
        // Windows' own template parts follow the same accepted language.
        FrameworkChrome.ApplyDeferred(this);
        var text = SettingsStrings.Current;
        Title = text.WindowTitle;
        headingText.Text = text.Heading;
        intro.Text = WorkspaceStrings.Current.SettingsHint;
        appearanceHeading.Text = WorkspaceStrings.Current.Appearance;
        captureHeading.Text = WorkspaceStrings.Current.Capture;
        storageHeading.Text = WorkspaceStrings.Current.Storage;
        language.Header = text.LanguageHeader;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(language, text.LanguageHeader);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(language, "InterfaceLanguageAutonyms");
        ToolTipService.SetToolTip(language, text.LanguageHint);
        var themeIndex = theme.SelectedIndex;
        theme.Header = text.ThemeHeader;
        // Replacing the item source clears the selection and raises SelectionChanged, which must not be read as a
        // user edit while the window is only re-labelling itself.
        applying = true;
        try
        {
            theme.ItemsSource = new[] { text.ThemeSystem, text.ThemeLight, text.ThemeDark };
            theme.SelectedIndex = Math.Max(0, themeIndex);
        }
        finally { applying = false; }
        hotkey.Header = text.HotkeyHeader;
        ToolTipService.SetToolTip(hotkey, text.HotkeyHint);
        searchHotkey.Header = text.SearchHotkeyHeader;
        ToolTipService.SetToolTip(searchHotkey, text.SearchHotkeyHint);
        var quickPasteIndex = quickPaste.SelectedIndex;
        quickPaste.Header = text.QuickPasteHeader;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(quickPaste, text.QuickPasteHeader);
        ToolTipService.SetToolTip(quickPaste, text.QuickPasteHint);
        applying = true;
        try
        {
            quickPaste.ItemsSource = new[] { text.QuickPasteOff, "Ctrl", "Alt", "Shift", "Win" };
            quickPaste.SelectedIndex = Math.Max(0, quickPasteIndex);
        }
        finally { applying = false; }
        deduplicate.Header = text.Deduplicate;
        showSearch.Header = text.ShowSearch;
        compact.Header = text.Compact;
        persistent.Header = text.PersistentHeader;
        persistent.OffContent = text.PersistentOff;
        persistent.OnContent = text.PersistentOn;
        captureFiles.Header = text.CaptureFiles;
        captureRichText.Header = text.CaptureRichText;
        deleteAfterPaste.Header = text.DeleteAfterPaste;
        moveToTop.Header = text.MoveToTop;
        edgeDocking.Header = text.EdgeDocking;
        privacy.Header = text.Privacy;
        limitEnabled.Header = text.LimitEnabled;
        // A toggle switch renders Windows' own on/off words until the app supplies its own.
        foreach (var toggle in new[] { deduplicate, showSearch, compact, captureFiles, captureRichText, deleteAfterPaste, moveToTop, edgeDocking, privacy, limitEnabled })
        {
            toggle.OnContent = CommonStrings.Current.On;
            toggle.OffContent = CommonStrings.Current.Off;
        }        limit.Header = text.LimitHeader;
        advanced.Content = text.Advanced;
        openerSection.Header = text.Openers;
        openers.ApplyLanguage();
        reload.Content = text.Reload;
        save.Content = text.Save;
        revision.Text = string.Format(text.RevisionFormat, revisionValue);
    }
    private readonly ComboBox language = new() { ItemsSource = new[] { "简体中文", "English", "繁體中文" }, HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly ComboBox theme = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly TextBox hotkey = new() { PlaceholderText = "Ctrl+Alt+V" };
    private readonly TextBox searchHotkey = new() { PlaceholderText = "Alt+F" };
    private readonly ComboBox quickPaste = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly ToggleSwitch deduplicate = new();
    private readonly ToggleSwitch showSearch = new();
    private readonly ToggleSwitch compact = new();
    private readonly ToggleSwitch persistent = new();
    private readonly ToggleSwitch captureFiles = new();
    private readonly ToggleSwitch captureRichText = new();
    private readonly ToggleSwitch deleteAfterPaste = new();
    private readonly ToggleSwitch moveToTop = new();
    private readonly ToggleSwitch edgeDocking = new();
    private readonly ToggleSwitch privacy = new();
    private readonly ToggleSwitch limitEnabled = new();
    private readonly TextBox limit = new();
    private readonly Button advanced = new();
    internal event Action? AdvancedRequested;
    private readonly OpenerSettingsEditor openers;
    private readonly Expander openerSection = new() { HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly Button reload = new();
    internal OpenerSettingsEditor Openers => openers;
    private readonly Button save = new() { IsEnabled = false };
    private readonly InfoBar message = new() { IsOpen = false, IsClosable = true };
    private readonly TextBlock revision = new() { TextWrapping = TextWrapping.Wrap };
    private bool loading;
    private bool dirty, applying;
    private ulong expectedRevision;
    private SettingsSnapshot? loadedSnapshot;
    internal ulong DisplayedRevision => expectedRevision;

    public SettingsWindow(NativeCore core)
    {
        this.core = core;
        openers = new OpenerSettingsEditor(() => WinRT.Interop.WindowNative.GetWindowHandle(this), core.GetApplicationCatalogAsync) { IconReader = core.GetApplicationIconAsync };
        Title = SettingsStrings.Current.WindowTitle;
        LanguageService.Changed += ApplyLanguage;
        Closed += (_, _) => LanguageService.Changed -= ApplyLanguage;
        AppWindow.Resize(new SizeInt32(620, 820));
        panel.Children.Add(headingText = new TextBlock { FontSize = 26, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        intro = new TextBlock { TextWrapping = TextWrapping.Wrap };
        intro.Style = revision.Style = (Style)Application.Current.Resources["NativeCaptionStyle"];
        panel.Children.Add(intro);
        panel.Children.Add(SettingsCard(appearanceHeading, language, theme, hotkey, searchHotkey, quickPaste, ToggleRow(showSearch), ToggleRow(compact)));
        ToolTipService.SetToolTip(hotkey, SettingsStrings.Current.HotkeyHint);
        panel.Children.Add(SettingsCard(captureHeading, ToggleRow(deduplicate), ToggleRow(captureFiles), ToggleRow(captureRichText), ToggleRow(deleteAfterPaste), ToggleRow(moveToTop), ToggleRow(edgeDocking), ToggleRow(privacy)));
        panel.Children.Add(SettingsCard(storageHeading, ToggleRow(persistent), ToggleRow(limitEnabled), limit));
        openerSection.Content = openers; panel.Children.Add(openerSection);
        openers.Changed += MarkDirty; openers.Error += ShowError;
        panel.Children.Add(advanced);
        advanced.Click += (_, _) => AdvancedRequested?.Invoke();
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        save.Style = (Style)Application.Current.Resources["AccentButtonStyle"];
        save.MinWidth = 100;
        actions.Children.Add(reload); actions.Children.Add(save);
        var footer = new StackPanel { Spacing = 8, Padding = new Thickness(20, 12, 20, 16) };
        footer.Children.Add(message); footer.Children.Add(actions);
        // Keep save/reload and validation feedback reachable while the settings themselves scroll.
        ToolTipService.SetToolTip(actions, revision);
        var layout = new Grid();
        layout.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        layout.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        layout.Children.Add(new ScrollViewer { Content = panel, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled });
        Grid.SetRow(footer, 1); layout.Children.Add(footer);
        surface.Child = layout;
        surface.Style = (Style)Application.Current.Resources["NativeSettingsSurfaceStyle"];
        Content = surface;
        language.SelectionChanged += (_, _) => MarkDirty();
        theme.SelectionChanged += (_, _) => MarkDirty();
        quickPaste.SelectionChanged += (_, _) => MarkDirty();
        deduplicate.Toggled += (_, _) => MarkDirty();
        showSearch.Toggled += (_, _) => MarkDirty();
        compact.Toggled += (_, _) => MarkDirty();
        foreach (var toggle in new[] { persistent, captureFiles, captureRichText, deleteAfterPaste, moveToTop, edgeDocking, privacy, limitEnabled })
            toggle.Toggled += (_, _) => MarkDirty();
        foreach (var input in new[] { hotkey, searchHotkey, limit })
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

    private static Border SettingsCard(TextBlock heading, params UIElement[] controls)
    {
        heading.FontSize = 16;
        heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        var content = new StackPanel { Spacing = 16 };
        content.Children.Add(heading);
        foreach (var control in controls) content.Children.Add(control);
        return new Border { Style = (Style)Application.Current.Resources["NativeSurfaceStyle"], Child = content };
    }

    private static Grid ToggleRow(ToggleSwitch toggle)
    {
        var row = new Grid { ColumnSpacing = 16, MinHeight = 40 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var label = new TextBlock { TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Center };
        label.SetBinding(TextBlock.TextProperty, new Microsoft.UI.Xaml.Data.Binding
        {
            Source = toggle, Path = new PropertyPath("Header"), Mode = Microsoft.UI.Xaml.Data.BindingMode.OneWay
        });
        toggle.HeaderTemplate = (DataTemplate)Application.Current.Resources["NativeHiddenSettingHeader"];
        toggle.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(toggle, 1);
        row.Children.Add(label); row.Children.Add(toggle);
        return row;
    }

    private void MarkDirty()
    {
        if (loading || applying || loadedSnapshot is null) return;
        dirty = CurrentValues().Any(pair => loadedSnapshot.Values[pair.Key] != pair.Value);
        save.IsEnabled = dirty && !openers.IsPicking;
        if (dirty && core.Snapshot.Revision != expectedRevision) ShowError(SettingsStrings.Current.Conflict);
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
            searchHotkey.Text = snapshot.Values["app.search_hotkey"];
            quickPaste.SelectedIndex = Math.Max(0, Array.IndexOf(QuickPasteModifiers, snapshot.Values["app.quick_paste_modifier"]));
            language.SelectedIndex = Array.IndexOf(Languages, snapshot.Values["app.language"]);
            theme.SelectedIndex = Array.IndexOf(Themes, snapshot.Values["app.color_mode"]);
            deduplicate.IsOn = snapshot.Values["app.deduplicate"] == "true";
            showSearch.IsOn = snapshot.Values["app.show_search_box"] == "true";
            compact.IsOn = snapshot.Values["app.compact_mode"] == "true";
            persistent.IsOn = snapshot.Values["app.persistent"] == "true";
            captureFiles.IsOn = snapshot.Values["app.capture_files"] == "true";
            captureRichText.IsOn = snapshot.Values["app.capture_rich_text"] == "true";
            deleteAfterPaste.IsOn = snapshot.Values["app.delete_after_paste"] == "true";
            moveToTop.IsOn = snapshot.Values["app.move_to_top_after_paste"] == "true";
            edgeDocking.IsOn = snapshot.Values["app.edge_docking"] == "true";
            privacy.IsOn = snapshot.Values["app.privacy_protection"] == "true";
            limitEnabled.IsOn = snapshot.Values["app.persistent_limit_enabled"] == "true";
            limit.Text = snapshot.Values["app.persistent_limit"];
            panel.RequestedTheme = surface.RequestedTheme = ViewTheme.From(snapshot);
            revisionValue = snapshot.Revision;
        revision.Text = string.Format(SettingsStrings.Current.RevisionFormat, revisionValue);
        ApplyLanguage();
            quickPaste.SelectedIndex = Math.Max(0, Array.IndexOf(QuickPasteModifiers, snapshot.Values["app.quick_paste_modifier"]));
            dirty = false; save.IsEnabled = false;
        }
        finally { loading = false; }
    }

    private void OnChanged(SettingsSnapshot snapshot)
    {
        panel.RequestedTheme = surface.RequestedTheme = ViewTheme.From(snapshot);
        if (dirty && snapshot.Revision != expectedRevision)
        {
            openers.InvalidatePickerChoice();
            ShowError(SettingsStrings.Current.Conflict);
            return;
        }
        Load(snapshot);
    }

    private Dictionary<string, string> CurrentValues()
    {
        var values = new Dictionary<string, string>
        {
            ["app.hotkey"] = hotkey.Text,
            ["app.search_hotkey"] = searchHotkey.Text,
            ["app.quick_paste_modifier"] = QuickPasteModifiers[Math.Clamp(quickPaste.SelectedIndex, 0, QuickPasteModifiers.Length - 1)],
            ["app.language"] = Languages[Math.Clamp(language.SelectedIndex, 0, Languages.Length - 1)],
            ["app.color_mode"] = Themes[Math.Clamp(theme.SelectedIndex, 0, Themes.Length - 1)],
            ["app.deduplicate"] = deduplicate.IsOn ? "true" : "false",
            ["app.show_search_box"] = showSearch.IsOn ? "true" : "false",
            ["app.compact_mode"] = compact.IsOn ? "true" : "false",
            ["app.persistent"] = persistent.IsOn ? "true" : "false",
            ["app.capture_files"] = captureFiles.IsOn ? "true" : "false",
            ["app.capture_rich_text"] = captureRichText.IsOn ? "true" : "false",
            ["app.delete_after_paste"] = deleteAfterPaste.IsOn ? "true" : "false",
            ["app.move_to_top_after_paste"] = moveToTop.IsOn ? "true" : "false",
            ["app.edge_docking"] = edgeDocking.IsOn ? "true" : "false",
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
            message.Message = core.LastSideEffectError is null ? SettingsStrings.Current.Saved
                : string.Format(SettingsStrings.Current.SavedWarningFormat, core.LastSideEffectError);
            message.IsOpen = true;
        }
        catch (Exception e)
        {
            save.IsEnabled = true;
            ShowError(e is CoreException { Code: "hotkey_conflict" }
                ? SettingsStrings.Current.HotkeyConflict : e.Message);
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
