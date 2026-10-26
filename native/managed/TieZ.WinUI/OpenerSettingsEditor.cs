using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;
using Windows.Storage.Pickers;

namespace TieZ.Native;

// Owns drafts and picker lifetime only. The shared core validates/saves all values.
internal sealed class OpenerSettingsEditor : StackPanel, IDisposable
{
    private static readonly (string Key, string Label, string Default)[] Definitions = [
        ("app.text", "文本", "使用系统默认应用"),
        ("app.code", "代码", "跟随文本应用"),
        ("app.rich_text", "富文本与表格", "使用系统默认应用"),
        ("app.image", "图片", "使用系统默认应用"),
        ("app.file", "文件", "使用系统默认应用"),
        ("app.video", "视频文件", "使用系统默认应用"),
        ("app.url", "网址", "使用系统默认应用"),
        ("app.link", "旧链接类型", "使用系统默认应用")
    ];
    private sealed record Row(TextBox Input, Button Browse, Button Default, Button Apps);
    private readonly Dictionary<string, Row> rows = [];
    private readonly Dictionary<string, string> original = [];
    private Func<nint>? owner;
    private CancellationTokenSource? pickerCancellation;
    private ContentDialog? applicationDialog;
    private ApplicationPickerView? applicationPicker;
    internal ApplicationPickerView? ActiveApplications => applicationPicker;
    internal ContentDialog? ApplicationDialog => applicationDialog;
    internal Func<bool, CancellationToken, Task<ApplicationCatalogResult>>? CatalogReader { get; set; }
    internal Func<ulong, InstalledApplication, uint, CancellationToken, Task<ApplicationIconRaster>>? IconReader { get; set; }
    private int generation;
    private bool binding, disposed;
    internal event Action? Changed;
    internal event Action<string>? Error;
    internal bool IsPicking => pickerCancellation is not null;
    internal bool HasRetainedData => rows.Count > 0 || original.Count > 0 || owner is not null || PickerReader is not null || CatalogReader is not null || IconReader is not null || applicationDialog is not null || applicationPicker is not null;
    // A delayed file result can model user selection/cancellation without choosing a real user file.
    internal Func<CancellationToken, Task<string?>>? PickerReader { get; set; }
    internal OpenerSettingsEditor(Func<nint> owner, Func<bool, CancellationToken, Task<ApplicationCatalogResult>> catalogReader)
    {
        this.owner = owner; CatalogReader = catalogReader; Spacing = 16; ActualThemeChanged += FollowDialogTheme;
        Children.Add(new TextBlock { Text = "选择的应用在保存后用于打开相应记录。空值使用下方默认规则；代码留空时跟随文本应用。", TextWrapping = TextWrapping.Wrap });
        Children.Add(new TextBlock { Text = "可从应用列表选择、浏览本地 .exe，或填写已知程序路径/打包应用标识。已有配置会保留，能否处理内容在打开时检查。.bat/.cmd 和 DOS 程序暂不支持，可改选原生程序或默认规则。", TextWrapping = TextWrapping.Wrap, Opacity = 0.7 });
        foreach (var definition in Definitions)
        {
            string key = definition.Key;
            var input = new TextBox { Header = definition.Label, PlaceholderText = definition.Default, TextWrapping = TextWrapping.Wrap, MaxHeight = 100 };
            var browse = new Button { Content = "浏览 .exe…" };
            var reset = new Button { Content = definition.Default };
            var apps = new Button { Content = "从应用列表选择…" };
            AutomationProperties.SetName(apps, $"从列表选择{definition.Label}打开应用");
            AutomationProperties.SetName(browse, $"选择{definition.Label}的打开程序");
            AutomationProperties.SetName(reset, $"{definition.Label}：{definition.Default}");
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            actions.Children.Add(apps); actions.Children.Add(browse);
            var group = new StackPanel { Spacing = 6 }; group.Children.Add(input); group.Children.Add(actions); group.Children.Add(reset);
            rows.Add(key, new(input, browse, reset, apps)); Children.Add(group);
            input.TextChanging += (_, _) => { if (!binding && !disposed) { if (applicationPicker is not null) InvalidatePickerChoice(); Changed?.Invoke(); } };
            reset.Click += (_, _) => { if (!disposed && !IsPicking) input.Text = ""; };
            browse.Click += async (_, _) => await BrowseAsync(key);
            apps.Click += async (_, _) => await ChooseApplicationAsync(key, definition.Label, definition.Default);
        }
    }
    private static string Lines(string value) => value.Replace("\r\n", "\n").Replace("\r", "\n");
    internal Dictionary<string,string> Values() => rows.ToDictionary(pair => pair.Key, pair =>
        original.TryGetValue(pair.Key, out string? value) && Lines(value) == Lines(pair.Value.Input.Text) ? value : pair.Value.Input.Text);
    internal void Bind(SettingsSnapshot snapshot)
    {
        if (disposed) return;
        InvalidatePickerChoice(); binding = true;
        try
        {
            original.Clear();
            foreach (var (key, row) in rows) { original[key] = snapshot.Values[key]; row.Input.Text = snapshot.Values[key]; }
        }
        finally { binding = false; }
    }
    private static async Task<string?> PickNativeAsync(nint owner, CancellationToken token)
    {
        var picker = new FileOpenPicker { SuggestedStartLocation = PickerLocationId.ComputerFolder, ViewMode = PickerViewMode.List };
        WinRT.Interop.InitializeWithWindow.Initialize(picker, owner);
        picker.FileTypeFilter.Add(".exe");
        var operation = picker.PickSingleFileAsync();
        using var registration = token.Register(() => { try { operation.Cancel(); } catch { } });
        try { var file = await operation; return file?.Path; }
        catch (Exception) when (token.IsCancellationRequested) { return null; }
    }
    private async Task BrowseAsync(string key)
    {
        if (disposed || IsPicking) return;
        var cancellation = new CancellationTokenSource();
        pickerCancellation = cancellation;
        int current = ++generation;
        string before = Values()[key];
        string? error = null;
        SetPickerButtons(false); Changed?.Invoke();
        try
        {
            var task = PickerReader is { } read ? read(cancellation.Token) : PickNativeAsync(owner!(), cancellation.Token);
            string? selected = await task.WaitAsync(cancellation.Token);
            if (!disposed && selected is not null)
            {
                if (current != generation || Values()[key] != before) error = "配置或当前应用草稿已变化，本次选择未应用，请重新选择。";
                else rows[key].Input.Text = selected;
            }
        }
        catch (OperationCanceledException) { }
        catch (Exception) { if (!disposed) error = "无法选择程序，请重试或直接填写已知程序路径。"; }
        finally
        {
            if (ReferenceEquals(pickerCancellation, cancellation)) pickerCancellation = null;
            cancellation.Dispose();
            if (!disposed)
            {
                SetPickerButtons(true); Changed?.Invoke();
                if (error is not null) Error?.Invoke(error);
            }
        }
    }
    private static void TryHide(ContentDialog? dialog) { try { dialog?.Hide(); } catch { } }
    private void FollowDialogTheme(FrameworkElement sender, object args)
    {
        if (applicationDialog is { } dialog) dialog.RequestedTheme = ActualTheme;
    }
    private async Task ChooseApplicationAsync(string key, string label, string defaultLabel)
    {
        if (disposed || IsPicking || CatalogReader is null) return;
        var cancellation = new CancellationTokenSource(); pickerCancellation = cancellation;
        int current = ++generation; string before = Values()[key]; string? error = null;
        var picker = new ApplicationPickerView(key[4..], before, defaultLabel, CatalogReader, cancellation.Token) { IconReader = IconReader };
        var dialog = new ContentDialog { Title = $"选择{label}打开应用", Content = picker, XamlRoot = XamlRoot,
            RequestedTheme = ActualTheme, PrimaryButtonText = "使用所选", CloseButtonText = "取消", IsPrimaryButtonEnabled = false };
        applicationPicker = picker; applicationDialog = dialog;
        picker.StateChanged += () => { if (!disposed && ReferenceEquals(applicationDialog, dialog)) dialog.IsPrimaryButtonEnabled = picker.CanChoose; };
        SetPickerButtons(false); Changed?.Invoke();
        try
        {
            using var registration = cancellation.Token.Register(() =>
            {
                if (DispatcherQueue.HasThreadAccess) TryHide(dialog);
                else DispatcherQueue.TryEnqueue(() => TryHide(dialog));
            });
            var shown = dialog.ShowAsync().AsTask();
            _ = picker.LoadAsync(false);
            var result = await shown.WaitAsync(cancellation.Token);
            cancellation.Token.ThrowIfCancellationRequested();
            if (!disposed && result == ContentDialogResult.Primary && picker.SelectedValue is { } selected)
            {
                if (current != generation || Values()[key] != before) error = "配置或当前应用草稿已变化，本次选择未应用，请重新选择。";
                else rows[key].Input.Text = selected;
            }
        }
        catch (OperationCanceledException) { }
        catch (Exception) { if (!disposed) error = "无法打开应用选择，请关闭其他对话框后重试。"; }
        finally
        {
            TryHide(dialog); picker.Dispose(); dialog.Content = null;
            if (ReferenceEquals(applicationPicker, picker)) applicationPicker = null;
            if (ReferenceEquals(applicationDialog, dialog)) applicationDialog = null;
            if (ReferenceEquals(pickerCancellation, cancellation)) pickerCancellation = null;
            cancellation.Dispose();
            if (!disposed) { SetPickerButtons(true); Changed?.Invoke(); if (error is not null) Error?.Invoke(error); }
        }
    }
    private void SetPickerButtons(bool enabled)
    {
        foreach (var row in rows.Values) row.Browse.IsEnabled = row.Default.IsEnabled = row.Apps.IsEnabled = enabled;
    }
    internal void InvalidatePickerChoice() { generation++; applicationPicker?.Invalidate(); }
    internal void CancelPicker() { pickerCancellation?.Cancel(); TryHide(applicationDialog); }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; generation++; CancelPicker(); pickerCancellation = null;
        ActualThemeChanged -= FollowDialogTheme; applicationPicker?.Dispose(); applicationPicker = null; applicationDialog = null; CatalogReader = null; IconReader = null;
        binding = true;
        foreach (var row in rows.Values) row.Input.Text = "";
        rows.Clear(); original.Clear(); Children.Clear(); owner = null; PickerReader = null; Changed = null; Error = null;
    }
    internal string Draft(string key) => Values()[key];
    internal void SetDraft(string key, string value) => rows[key].Input.Text = value;
    internal void InvokeApplications(string key) => Invoke(rows[key].Apps);
    internal void InvokeBrowse(string key) => Invoke(rows[key].Browse);
    internal void InvokeDefault(string key) => Invoke(rows[key].Default);
    private static void Invoke(Button button) => ((IInvokeProvider)new ButtonAutomationPeer(button).GetPattern(PatternInterface.Invoke)).Invoke();
}
