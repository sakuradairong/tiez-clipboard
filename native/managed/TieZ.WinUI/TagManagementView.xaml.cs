using System.Collections.ObjectModel;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;
using Color = Windows.UI.Color;
using Colors = Microsoft.UI.Colors;

namespace TieZ.Native;

public sealed partial class TagManagementView : UserControl, IDisposable
{
    private const uint PageSize = 100;
    private readonly ObservableCollection<TagRow> rows = [];
    private readonly CancellationTokenSource lifetime = new();
    private CancellationTokenSource? readCancellation;
    private DispatcherQueueTimer? poll;
    private NativeCore? core;
    private TagRow? selected;
    private ulong? revision;
    private string? after, acceptedColor, draftColor;
    private Color? knownPickerColor;
    private int generation;
    private bool active, disposed, loading, saving, polling, binding, dirty, stale, hasMore, refreshOnActivate;
    internal event Action<string>? FilterRequested;
    internal bool IsBusy => loading || saving;
    internal bool IsActive => active;
    internal bool IsContentReady => IsLoaded && XamlRoot is not null;
    internal bool HasDraft => dirty;
    internal bool HasConflict => stale;
    internal bool HasError => Feedback.IsOpen && Feedback.Severity == InfoBarSeverity.Error;
    internal bool CanSave => SaveColorButton.IsEnabled;
    internal bool HasInvalidColor => dirty && draftColor is not null && !NativeTagColor.TryParse(draftColor, out _);
    internal int Count => rows.Count;
    internal bool HasMore => hasMore;
    internal ulong? DisplayedRevision => revision;
    internal string? SelectedTag => selected?.Info.Name;
    internal string ColorDraft => ColorText.Text;
    internal string NewNameDraft { get => NewTagName.Text; set => NewTagName.Text = value; }
    internal ElementTheme CurrentTheme => ActualTheme;
    internal int RealizedCount => Enumerable.Range(0, rows.Count).Count(index => TagList.ContainerFromIndex(index) is not null);
    internal bool HasRetainedData => core is not null || rows.Count > 0 || selected is not null || revision is not null
        || acceptedColor is not null || draftColor is not null || ColorText.Text.Length > 0 || NewTagName.Text.Length > 0;

    public TagManagementView()
    {
        InitializeComponent(); TagList.ItemsSource = rows;
        TagList.SelectionChanged += (_, _) =>
        {
            if (binding || disposed || ReferenceEquals(selected, TagList.SelectedItem)) return;
            if (dirty && !ReferenceEquals(selected, TagList.SelectedItem))
            {
                binding = true; TagList.SelectedItem = selected; binding = false;
                Message("请先保存颜色，或刷新以放弃当前颜色修改。", InfoBarSeverity.Warning); return;
            }
            LoadSelected(TagList.SelectedItem as TagRow);
        };
        NewTagName.TextChanging += (_, _) => UpdateCommands();
        NewTagName.BeforeTextChanging += (_, args) =>
        {
            if (!binding && args.NewText.IndexOfAny(['\r', '\n']) >= 0)
            { args.Cancel = true; Message("标签名称请保持在一行。", InfoBarSeverity.Warning); }
        };
        ColorText.TextChanging += (_, _) => ColorChanged();
        Picker.ColorChanged += (_, args) =>
        {
            if (binding || disposed || saving || args.NewColor != Picker.Color || knownPickerColor == args.NewColor) return;
            knownPickerColor = args.NewColor;
            ColorText.Text = NativeTagColor.TryParse(acceptedColor, out var original) && original == args.NewColor
                ? acceptedColor! : NativeTagColor.Css(args.NewColor);
        };
        CreateButton.Click += async (_, _) => await SaveAsync(create: true);
        SaveColorButton.Click += async (_, _) => await SaveAsync(create: false);
        ClearColorButton.Click += (_, _) => ColorText.Text = "";
        ReloadButton.Click += async (_, _) => await LoadAsync(reset: true);
        MoreButton.Click += async (_, _) => await LoadAsync(reset: false);
        FilterButton.Click += (_, _) => { if (!IsBusy && !dirty && !stale && selected is not null) FilterRequested?.Invoke(selected.Info.Name); };
        LoadSelected(null); UpdateCommands();
    }
    internal void Initialize(NativeCore sharedCore)
    {
        if (core is not null) throw new InvalidOperationException("Tag page already initialized.");
        core = sharedCore; poll = DispatcherQueue.CreateTimer(); poll.Interval = TimeSpan.FromSeconds(1); poll.Tick += OnPoll;
    }
    internal void SetActive(bool value)
    {
        if (disposed || active == value) return;
        active = value;
        if (!value)
        {
            poll?.Stop(); generation++; readCancellation?.Cancel(); loading = false; UpdateCommands(); return;
        }
        poll?.Start(); UpdateCommands();
        if (revision is null || refreshOnActivate) _ = LoadAsync(reset: true);
    }
    private void LoadSelected(TagRow? row)
    {
        selected = row; acceptedColor = draftColor = row?.Info.Color; dirty = false;
        binding = true;
        try
        {
            SelectedName.Text = row?.Label ?? "选择一个标签"; SelectedCounts.Text = row?.Details ?? "";
            ColorText.Text = acceptedColor ?? "";
            BindPicker(acceptedColor);
        }
        finally { binding = false; }
        DescribeColor(); UpdateCommands();
    }
    private void BindPicker(string? value)
    {
        knownPickerColor = NativeTagColor.TryParse(value, out var color) ? color : Colors.Transparent;
        Picker.Color = knownPickerColor.Value;
    }
    private void ColorChanged()
    {
        if (binding || disposed || selected is null) return;
        string? value = ColorText.Text.Length == 0 ? null : ColorText.Text;
        // Semantically unchanged native colors retain their original CSS/HEX spelling.
        if (Lines(value) == Lines(acceptedColor) || (NativeTagColor.TryParse(value, out var current)
            && NativeTagColor.TryParse(acceptedColor, out var original) && current == original)) value = acceptedColor;
        draftColor = value; dirty = draftColor != acceptedColor;
        binding = true;
        try { if (NativeTagColor.TryParse(value, out _)) BindPicker(value); }
        finally { binding = false; }
        DescribeColor(); if (!stale) Feedback.IsOpen = false; UpdateCommands();
    }
    private static string? Lines(string? value) => value?.Replace("\r\n", "\n").Replace("\r", "\n");
    private void DescribeColor() => ColorHint.Text = draftColor is null ? "使用默认颜色。"
        : NativeTagColor.TryParse(draftColor, out _) ? "支持透明度；使用颜色选择器可直接调整。"
        : dirty ? "无法解析此颜色。请输入 HEX、RGB、RGBA 或颜色名称后再保存。"
        : "此颜色格式暂不支持预览；未修改的原值会保留，也可选择新的颜色。";
    private void UpdateCommands()
    {
        if (disposed) return;
        bool ready = active && !IsBusy && revision is not null;
        CreateButton.IsEnabled = ready && !dirty && !stale && !string.IsNullOrWhiteSpace(NewTagName.Text);
        ReloadButton.IsEnabled = active && !IsBusy;
        MoreButton.IsEnabled = ready && hasMore && !dirty && !stale;
        MoreButton.Visibility = hasMore ? Visibility.Visible : Visibility.Collapsed;
        TagList.IsEnabled = active && !IsBusy;
        NewTagName.IsEnabled = active && !IsBusy;
        ColorText.IsEnabled = Picker.IsEnabled = ready && selected is not null;
        ClearColorButton.IsEnabled = ready && selected is not null && draftColor is not null;
        SaveColorButton.IsEnabled = ready && selected is not null && dirty && !stale && !HasInvalidColor;
        FilterButton.IsEnabled = ready && selected is not null && !dirty && !stale;
        Status.Text = loading ? "正在读取标签…" : saving ? "正在保存…" : $"已载入 {rows.Count} 个标签" + (hasMore ? " · 可继续载入" : "");
    }
    private void Message(string message, InfoBarSeverity severity)
    { if (!disposed) { Feedback.Message = message; Feedback.Severity = severity; Feedback.IsOpen = true; } }
    private async Task<bool> LoadAsync(bool reset, string? preferred = null)
    {
        if (disposed || !active || core is null || IsBusy || (!reset && (!hasMore || dirty || stale))) return false;
        preferred ??= selected?.Info.Name;
        int desired = reset ? Math.Max((int)PageSize, rows.Count) : (int)PageSize;
        readCancellation?.Cancel(); readCancellation?.Dispose();
        var cancellation = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token); readCancellation = cancellation;
        int current = ++generation; loading = true; UpdateCommands();
        try
        {
            string? cursor = reset ? null : after; ulong? expected = reset ? null : revision;
            var staged = new List<TagRow>(); TagPage page;
            do
            {
                page = await core.GetTagsAsync(new TagQuery(PageSize, cursor, expected), cancellation.Token);
                if (disposed || !active || current != generation || cancellation.IsCancellationRequested) return false;
                if (page.HasMore && (page.NextAfter is null || page.NextAfter == cursor)) throw new InvalidDataException("Tag paging did not advance.");
                staged.AddRange(page.Items.Select(info => new TagRow(info))); cursor = page.NextAfter; expected = page.Revision;
            } while (page.HasMore && staged.Count < desired);
            binding = true;
            try
            {
                if (reset) { TagList.SelectedItem = null; rows.Clear(); }
                foreach (var row in staged) rows.Add(row);
                TagList.SelectedItem = rows.FirstOrDefault(row => row.Info.Name == preferred);
            }
            finally { binding = false; }
            revision = page.Revision; after = page.NextAfter; hasMore = page.HasMore; stale = false; refreshOnActivate = false;
            LoadSelected(TagList.SelectedItem as TagRow); Feedback.IsOpen = false; return true;
        }
        catch (OperationCanceledException) { return false; }
        catch (Exception error)
        {
            if (!disposed && current == generation)
            {
                if (error is CoreException { Code: "history_conflict" }) stale = true;
                Message($"读取标签失败，当前内容和草稿已保留：{error.Message}", InfoBarSeverity.Error);
            }
            return false;
        }
        finally
        {
            if (ReferenceEquals(readCancellation, cancellation)) { readCancellation = null; cancellation.Dispose(); }
            if (!disposed && current == generation) { loading = false; UpdateCommands(); }
        }
    }
    private async Task SaveAsync(bool create)
    {
        if (disposed || core is null || revision is not { } expected || IsBusy || stale || !active
            || (create ? !CreateButton.IsEnabled : !SaveColorButton.IsEnabled)) return;
        string name = create ? NewTagName.Text.Trim() : selected!.Info.Name;
        string submittedName = NewTagName.Text; string? value = draftColor;
        saving = true; UpdateCommands();
        try
        {
            var result = create ? await core.CreateTagAsync(name, expected, lifetime.Token)
                : await core.SetTagColorAsync(name, value, expected, lifetime.Token);
            if (disposed) return;
            revision = result.Revision;
            if (create) { if (NewTagName.Text == submittedName) NewTagName.Text = ""; }
            else { acceptedColor = draftColor = value; dirty = false; }
            refreshOnActivate = true; saving = false;
            bool reloaded = active && await LoadAsync(reset: true, preferred: name);
            if (!reloaded) stale = true; // The old rows cannot authorize another metadata change after a committed save.
            if (!disposed) Message(reloaded ? (result.Changed ? "标签已保存。" : "内容未变化，无需重复保存。")
                : "保存已完成；目录尚未刷新，返回标签页后刷新查看。", reloaded ? InfoBarSeverity.Success : InfoBarSeverity.Warning);
        }
        catch (OperationCanceledException) { }
        catch (Exception error)
        {
            if (!disposed)
            {
                if (error is CoreException { Code: "history_conflict" }) stale = true;
                Message($"未能保存，草稿已保留。请刷新后重试：{error.Message}", InfoBarSeverity.Error);
            }
        }
        finally { saving = false; if (!disposed) UpdateCommands(); }
    }
    private async void OnPoll(DispatcherQueueTimer sender, object args)
    {
        if (disposed || !active || IsBusy || polling || stale || revision is not { } expected) return;
        polling = true;
        try
        {
            var changes = await core!.GetHistoryChangesAsync(expected, lifetime.Token);
            if (!disposed && active && !IsBusy && revision == expected && changes.Revision != expected)
            {
                stale = true; Message("标签或历史已有更新。当前草稿已保留；刷新后可继续操作。", InfoBarSeverity.Warning); UpdateCommands();
            }
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { if (!disposed && active) Message($"检查更新失败：{error.Message}", InfoBarSeverity.Warning); }
        finally { polling = false; }
    }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; active = false; dirty = stale = hasMore = loading = saving = false; generation++; lifetime.Cancel(); readCancellation?.Cancel();
        poll?.Stop(); if (poll is not null) poll.Tick -= OnPoll;
        readCancellation?.Dispose(); readCancellation = null; lifetime.Dispose();
        binding = true;
        TagList.SelectedItem = null; rows.Clear(); selected = null; acceptedColor = draftColor = after = null; revision = null; core = null;
        NewTagName.Text = ColorText.Text = SelectedName.Text = SelectedCounts.Text = Status.Text = "";
        Picker.Color = Colors.Transparent; knownPickerColor = null; Feedback.IsOpen = false; Feedback.Message = ""; FilterRequested = null;
    }
    internal void Select(string name) => TagList.SelectedItem = rows.FirstOrDefault(row => row.Info.Name == name);
    internal void EditColor(string value) => ColorText.Text = value;
    internal void ChooseColor(Color value) => Picker.Color = value;
    internal void Invoke(string command)
    {
        var button = command switch { "create" => CreateButton, "save" => SaveColorButton, "clear" => ClearColorButton,
            "reload" => ReloadButton, "more" => MoreButton, "filter" => FilterButton, _ => throw new ArgumentException(nameof(command)) };
        if (button.IsEnabled) ((IInvokeProvider)new ButtonAutomationPeer(button).GetPattern(PatternInterface.Invoke)).Invoke();
    }
}
