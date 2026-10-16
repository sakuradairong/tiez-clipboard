using System.Collections.ObjectModel;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using TieZ.Core;

namespace TieZ.Native;

public sealed partial class HistoryView : UserControl, IDisposable
{
    private static readonly string?[] Types = [null, "text", "code", "url", "rich_text", "image", "file", "video"];
    private readonly ObservableCollection<HistoryRow> rows = [];
    private readonly CancellationTokenSource lifetime = new();
    private readonly HashSet<TextBox> compositions = [];
    private NativeCore? core;
    private CaptureStatus? captureStatus;
    private TargetTrackingStatus? targetStatus;
    private bool targetBusy;
    private bool startCapture, captureBusy;
    private int captureGeneration;
    private DispatcherQueueTimer? debounce;
    private DispatcherQueueTimer? poll;
    private ScrollViewer? listScroll;
    private CancellationTokenSource? loadCancellation;
    private CancellationTokenSource? detailCancellation;
    private ulong? revision;
    private ulong? observedRevision;
    private uint offset;
    private int generation;
    private int detailGeneration;
    private bool started, disposed, loading, mutating, polling, filterPending, hasMore, externalPending;
    private ClipboardEntry? selectedEntry;
    private ContentDialog? activeDialog;
    private TextBox? dialogInput;

    public HistoryView()
    {
        InitializeComponent();
        HistoryList.ItemsSource = rows;
        foreach (var input in new[] { SearchText, TagFilter })
        {
            input.TextCompositionStarted += (_, _) =>
            {
                compositions.Add(input);
                InvalidateRead();
                debounce?.Stop();
            };
            input.TextCompositionEnded += (_, _) =>
            {
                compositions.Remove(input);
                ScheduleSearch();
            };
            // Invalidate selection synchronously; debounce only the query itself.
            input.TextChanging += (_, _) => ScheduleSearch();
        }
        TypeFilter.SelectionChanged += (_, _) => ScheduleSearch();
        PinnedOnly.Checked += (_, _) => ScheduleSearch();
        PinnedOnly.Unchecked += (_, _) => ScheduleSearch();
        TagOnly.Checked += (_, _) => ScheduleSearch();
        TagOnly.Unchecked += (_, _) => ScheduleSearch();
        HistoryList.SelectionChanged += (_, _) => _ = LoadDetailAsync(false);
        HistoryList.Loaded += (_, _) => AttachScroll();
        Loaded += (_, _) =>
        {
            if (started || disposed) return;
            started = true;
            poll!.Start();
            _ = ReloadAsync();
            _ = SetCaptureAsync(startCapture);
        };
        CaptureButton.Click += async (_, _) => await SetCaptureAsync(captureStatus?.Running != true);
        AddButton.Click += async (_, _) => await ShowTextDialogAsync("新增条目", "", value => core!.AddManualAsync(value));
        ReturnTargetButton.Click += async (_, _) => await ReturnTargetAsync();
        StartTargetButton.Click += async (_, _) => await StartTargetAsync();
        PasteButton.Click += async (_, _) => await PasteSelectedAsync(false);
        PastePlainButton.Click += async (_, _) => await PasteSelectedAsync(true);
        CopyButton.Click += async (_, _) => await CopySelectedAsync(false);
        CopyPlainButton.Click += async (_, _) => await CopySelectedAsync(true);
        EditButton.Click += async (_, _) =>
        {
            if (selectedEntry is { } entry)
                await ShowTextDialogAsync("编辑内容", entry.Content, value => core!.EditEntryAsync(entry.Id, value));
        };
        TagsButton.Click += async (_, _) =>
        {
            if (HistoryList.SelectedItem is HistoryRow row)
                await ShowTextDialogAsync("标签（每行一个）", string.Join("\n", row.Summary.Tags),
                    value => core!.SetTagsAsync(row.Id, value.Replace("\r\n", "\n").Replace("\r", "\n").Split('\n')));
        };
        PinButton.Click += async (_, _) =>
        {
            if (HistoryList.SelectedItem is HistoryRow row)
                await MutateAsync(() => core!.SetPinnedAsync(row.Id, !row.Summary.IsPinned));
        };
        DeleteButton.Click += async (_, _) => await DeleteSelectedAsync();
        ClearButton.Click += async (_, _) => await ShowClearDialogAsync();
        RefreshButton.Click += async (_, _) => await ReloadAsync();
        MoreButton.Click += async (_, _) => await LoadAsync(false);
        RevealButton.Click += async (_, _) => await LoadDetailAsync(true);
        UpdateCommands();
    }

    internal void Initialize(NativeCore sharedCore, bool captureOnStartup)
    {
        if (core is not null) throw new InvalidOperationException("A history view borrows exactly one core.");
        core = sharedCore;
        startCapture = captureOnStartup;
        debounce = DispatcherQueue.CreateTimer();
        debounce.Interval = TimeSpan.FromMilliseconds(250);
        debounce.IsRepeating = false;
        debounce.Tick += OnDebounce;
        poll = DispatcherQueue.CreateTimer();
        poll.Interval = TimeSpan.FromSeconds(1);
        poll.Tick += OnPoll;
    }

    internal void SetTargetError(string message) => TargetInfo.Text = $"目标跟踪不可用：{message}";

    private void ApplyTarget(TargetTrackingStatus status)
    {
        if (targetStatus is not null && status.Generation < targetStatus.Generation) return;
        targetStatus = status;
        TargetInfo.Text = status.HasTarget ? $"上次输入窗口：{status.Application}。返回窗口不会粘贴内容。"
            : status.Running ? "尚无输入目标：先切换到需要粘贴的应用，再返回 TieZ。" : "窗口跟踪已停止，可点击启用目标跟踪。";
        if (status.LastError is not null) TargetInfo.Text += $" 服务错误：{status.LastError}";
        StartTargetButton.Visibility = status.Running ? Visibility.Collapsed : Visibility.Visible;
        UpdateCommands();
    }
    private async Task StartTargetAsync()
    {
        if (disposed || targetBusy) return;
        targetBusy = true; UpdateCommands();
        try { var status = await core!.StartTargetTrackingAsync(lifetime.Token); if (!disposed) ApplyTarget(status); }
        catch (OperationCanceledException) { }
        catch (Exception error) { if (!disposed) SetTargetError(error.Message); }
        finally { targetBusy = false; if (!disposed) UpdateCommands(); }
    }
    private async Task ReturnTargetAsync()
    {
        if (disposed || targetBusy || targetStatus is not { HasTarget: true } target) return;
        targetBusy = true; UpdateCommands();
        try
        {
            await core!.RestoreTargetFocusAsync(target.Generation, lifetime.Token);
            if (!disposed) { Feedback.Title = "已返回目标窗口"; Feedback.Message = "未发送粘贴按键。"; Feedback.Severity = InfoBarSeverity.Success; Feedback.IsOpen = true; }
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { if (!disposed) ShowError(error.Message); }
        finally { targetBusy = false; if (!disposed) UpdateCommands(); }
    }

    private void ApplyCapture(CaptureStatus status)
    {
        captureStatus = status;
        CaptureButton.Label = status.Running ? "暂停采集" : "开始采集";
        CaptureButton.Icon = new SymbolIcon(status.Running ? Symbol.Pause : Symbol.Play);
        CaptureInfo.Severity = status.LastError is null ? InfoBarSeverity.Informational : InfoBarSeverity.Warning;
        string state = status.Running ? "采集中" : "采集已停止";
        CaptureInfo.Message = $"{state} · 已接收 {status.Accepted} 条 · 忽略 {status.Ignored} 条 · 失败 {status.Failed} 次。使用独立数据库；粘贴仍在迁移。"
            + (status.LastError is null ? "" : $" 服务错误：{status.LastError}");
    }

    private async Task SetCaptureAsync(bool enabled)
    {
        if (disposed || captureBusy) return;
        captureBusy = true;
        captureGeneration++;
        CaptureButton.IsEnabled = false;
        try
        {
            var status = enabled ? await core!.StartCaptureAsync(lifetime.Token)
                : await core!.StopCaptureAsync(lifetime.Token);
            if (!disposed) ApplyCapture(status);
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (Exception error)
        {
            if (!disposed)
            {
                CaptureInfo.Severity = InfoBarSeverity.Error;
                CaptureInfo.Message = $"采集状态未确认：{error.Message}";
                ShowError(error.Message);
            }
        }
        finally
        {
            captureBusy = false;
            if (!disposed) CaptureButton.IsEnabled = true;
        }
    }

    internal bool SearchVisible
    {
        get => SearchText.Visibility == Visibility.Visible;
        set
        {
            SearchText.Visibility = value ? Visibility.Visible : Visibility.Collapsed;
            if (!value) SearchText.Text = "";
        }
    }
    private void InvalidateRead()
    {
        if (!started || disposed) return;
        generation++;
        loadCancellation?.Cancel();
        HistoryList.SelectedItems.Clear();
        loading = false;
        filterPending = true;
        UpdateCommands();
    }
    private void ScheduleSearch()
    {
        if (!started || disposed) return;
        InvalidateRead();
        debounce!.Stop();
        if (compositions.Count == 0) debounce.Start();
    }
    private void OnDebounce(DispatcherQueueTimer sender, object args) => _ = ReloadAsync();
    internal Task ReloadAsync(long? selectId = null) => LoadAsync(true, selectId);

    private async Task LoadAsync(bool reset, long? selectId = null)
    {
        if (disposed || core is null || (!reset && (loading || mutating || !hasMore))) return;
        if (!reset && externalPending) { await ReloadAsync(); return; }
        if (reset)
        {
            debounce?.Stop();
            generation++;
            loadCancellation?.Cancel();
            loadCancellation?.Dispose();
            loadCancellation = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
            filterPending = false;
        }
        var cancellation = loadCancellation!;
        var current = generation;
        var query = new HistoryQuery(Limit: 50, Offset: reset ? 0 : offset,
            ExpectedRevision: reset ? null : revision, Search: SearchText.Text,
            ContentType: Types[Math.Max(0, TypeFilter.SelectedIndex)],
            Tag: string.IsNullOrWhiteSpace(TagFilter.Text) ? null : TagFilter.Text.Trim(),
            TagOnly: TagOnly.IsChecked == true, PinnedOnly: PinnedOnly.IsChecked == true);
        loading = true;
        UpdateCommands();
        try
        {
            HistoryPage page;
            do
            {
                page = await core.GetHistoryAsync(query, cancellation.Token);
                if (disposed || current != generation || cancellation.IsCancellationRequested) return;
                if (page.HasMore && page.NextOffset <= query.Offset)
                    throw new InvalidOperationException("History paging did not advance.");
                query = query with { Offset = page.NextOffset, ExpectedRevision = page.Revision };
            } while (page.Items.Count == 0 && page.HasMore);
            if (reset)
            {
                selectId ??= (HistoryList.SelectedItem as HistoryRow)?.Id;
                rows.Clear();
                externalPending = false;
                RefreshButton.Label = "刷新";
            }
            foreach (var item in page.Items)
            {
                // ID replacement/revision resets must not leave duplicate containers.
                if (!rows.Any(row => row.Id == item.Id)) rows.Add(new(item));
            }
            revision = page.Revision;
            observedRevision = page.Revision;
            offset = page.NextOffset;
            hasMore = page.HasMore;
            ListStatus.Text = $"已载入 {rows.Count} 条 · 本地 {page.StoredCount} 条 · 会话 {page.SessionCount} 条";
            if (selectId is { } selected)
                HistoryList.SelectedItem = rows.FirstOrDefault(row => row.Id == selected);
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (CoreException error) when (error.Code == "history_conflict")
        {
            if (!disposed && current == generation) await ReloadAsync(selectId);
        }
        catch (Exception error)
        {
            if (!disposed && current == generation) ShowError(error.Message);
        }
        finally
        {
            if (!disposed && current == generation)
            {
                loading = false;
                UpdateCommands();
            }
        }
    }

    private async void OnPoll(DispatcherQueueTimer sender, object args)
    {
        if (disposed || loading || mutating || polling || filterPending || revision is null) return;
        polling = true;
        int current = generation;
        try
        {
            if (!captureBusy)
            {
                int captureRead = captureGeneration;
                var status = await core!.GetCaptureStatusAsync(lifetime.Token);
                if (disposed) return;
                if (captureRead == captureGeneration && !captureBusy) ApplyCapture(status);
            }
            if (!targetBusy)
            {
                var target = await core!.GetTargetStatusAsync(lifetime.Token);
                if (disposed) return;
                if (!targetBusy) ApplyTarget(target);
            }
            var changes = await core!.GetHistoryChangesAsync(observedRevision ?? revision.Value, lifetime.Token);
            if (disposed || current != generation || loading || mutating || changes.Revision == observedRevision) return;
            observedRevision = changes.Revision;
            if (externalPending || activeDialog is not null || HistoryList.SelectedItem is not null || (listScroll?.VerticalOffset ?? 0) > 32)
            {
                externalPending = true;
                RefreshButton.Label = "刷新（有更新）";
                var changedIds = changes.Changes.SelectMany(change => change.RemovedIds.Append(change.Id)
                    .Concat(change.ReplacedId is { } replaced ? new[] { replaced } : Array.Empty<long>())).ToHashSet();
                for (int index = 0; index < rows.Count; index++)
                    if (changes.ResyncRequired || changedIds.Contains(rows[index].Id))
                        rows[index] = new HistoryRow(rows[index].Summary, isStale: true);
                if ((HistoryList.SelectedItem as HistoryRow)?.IsStale == true) HistoryList.SelectedItem = null;
                UpdateCommands();
                return;
            }
            await ReloadAsync();
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (Exception error) { if (!disposed) ShowError(error.Message); }
        finally { polling = false; }
    }

    private async Task LoadDetailAsync(bool reveal)
    {
        if (disposed) return;
        detailGeneration++;
        var current = detailGeneration;
        detailCancellation?.Cancel();
        detailCancellation?.Dispose();
        detailCancellation = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
        var token = detailCancellation.Token;
        selectedEntry = null;
        ImagePreview.Clear();
        DetailText.Visibility = Visibility.Visible;
        DetailText.Text = "";
        RevealButton.Visibility = Visibility.Collapsed;
        if (disposed) return;
        if (HistoryList.SelectedItems.Count != 1 || HistoryList.SelectedItem is not HistoryRow row)
        {
            DetailTitle.Text = HistoryList.SelectedItems.Count > 1 ? $"已选择 {HistoryList.SelectedItems.Count} 条" : "选择一条记录";
            DetailMetadata.Text = "";
            UpdateCommands();
            return;
        }
        if (row.IsStale) { UpdateCommands(); return; }
        DetailTitle.Text = row.Summary.ContentType;
        DetailMetadata.Text = row.Metadata;
        UpdateCommands();
        if (row.Summary.IsSensitive && !reveal)
        {
            RevealButton.Visibility = Visibility.Visible;
            return;
        }
        try
        {
            var entry = await core!.GetEntryAsync(row.Id, token);
            if (disposed || token.IsCancellationRequested || current != detailGeneration ||
                (HistoryList.SelectedItem as HistoryRow)?.Id != row.Id) return;
            if (entry.ContentType == "image")
            {
                try
                {
                    var preview = await core.GetImagePreviewAsync(entry.Id, revision!.Value, reveal,
                        cancellationToken: token);
                    if (disposed || token.IsCancellationRequested || current != detailGeneration) return;
                    await ImagePreview.ShowAsync(preview, token);
                    if (disposed || token.IsCancellationRequested || current != detailGeneration) return;
                    DetailText.Visibility = Visibility.Collapsed;
                }
                catch (CoreException error) when (error.Code is not "history_conflict" and not "sensitive_confirmation")
                {
                    if (disposed || token.IsCancellationRequested || current != detailGeneration) return;
                    DetailText.Text = "图片预览不可用，原始内容未修改。";
                    ShowError(error.Message);
                }
            }
            else DetailText.Text = entry.Content;
            selectedEntry = entry;
            UpdateCommands();
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (Exception error)
        {
            if (!disposed && current == detailGeneration) ShowError(error.Message);
        }
    }

    private void UpdateCommands()
    {
        if (disposed) return;
        bool busy = loading || mutating || filterPending || targetBusy;
        var selected = HistoryList.SelectedItems.Count == 1 ? HistoryList.SelectedItem as HistoryRow : null;
        bool available = !busy && activeDialog is null;
        ReturnTargetButton.IsEnabled = available && targetStatus?.HasTarget == true;
        StartTargetButton.IsEnabled = available;
        AddButton.IsEnabled = available && !externalPending;
        EditButton.IsEnabled = available && !externalPending && selectedEntry is not null;
        CopyButton.IsEnabled = CopyPlainButton.IsEnabled = available && !externalPending && selectedEntry is not null && selected is { IsStale: false };
        PasteButton.IsEnabled = PastePlainButton.IsEnabled = CopyButton.IsEnabled && targetStatus is { Running: true, HasTarget: true };
        PinButton.IsEnabled = TagsButton.IsEnabled = available && !externalPending && selected is { IsStale: false };
        DeleteButton.IsEnabled = available && !externalPending && HistoryList.SelectedItems.Count > 0
            && HistoryList.SelectedItems.Cast<HistoryRow>().All(row => !row.IsStale);
        DeleteButton.Label = HistoryList.SelectedItems.Count > 1 ? $"删除所选（{HistoryList.SelectedItems.Count}）" : "删除";
        PinButton.Label = selected?.Summary.IsPinned == true ? "取消置顶" : "置顶";
        ClearButton.IsEnabled = available && !externalPending;
        RefreshButton.IsEnabled = available;
        MoreButton.IsEnabled = available && hasMore;
        MoreButton.Visibility = hasMore ? Visibility.Visible : Visibility.Collapsed;
        RevealButton.IsEnabled = available && !externalPending;
        LoadingIndicator.IsActive = busy;
    }

    private async Task CopySelectedAsync(bool plainText)
    {
        if (disposed || mutating || loading || filterPending || externalPending || activeDialog is not null
            || selectedEntry is not { } entry || HistoryList.SelectedItem is not HistoryRow { IsStale: false } row) return;
        mutating = true;
        UpdateCommands();
        try
        {
            await core!.CopyEntryAsync(entry.Id, plainText, allowSensitive: row.Summary.IsSensitive, lifetime.Token);
            if (!disposed)
            {
                Feedback.Title = "已复制";
                Feedback.Message = plainText ? "已复制为纯文本。" : "已按原始格式复制到剪贴板。";
                Feedback.Severity = InfoBarSeverity.Success;
                Feedback.IsOpen = true;
            }
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (Exception error) { if (!disposed) ShowError(error.Message); }
        finally { mutating = false; if (!disposed) UpdateCommands(); }
    }

    private async Task PasteSelectedAsync(bool plainText)
    {
        if (disposed || mutating || loading || filterPending || externalPending || targetBusy || activeDialog is not null
            || targetStatus is not { Running: true, HasTarget: true } target || selectedEntry is not { } entry
            || HistoryList.SelectedItem is not HistoryRow { IsStale: false } row) return;
        mutating = true;
        UpdateCommands();
        try
        {
            await core!.PasteEntryAsync(entry.Id, target.Generation, plainText,
                allowSensitive: row.Summary.IsSensitive, lifetime.Token);
            if (!disposed)
            {
                Feedback.Title = "粘贴按键已发送";
                Feedback.Message = "目标程序的接收结果需在目标窗口确认。";
                Feedback.Severity = InfoBarSeverity.Success;
                Feedback.IsOpen = true;
            }
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (Exception error) { if (!disposed) ShowError(error.Message); }
        finally { mutating = false; if (!disposed) UpdateCommands(); }
    }

    private void ShowError(string message)
    {
        Feedback.Severity = InfoBarSeverity.Error;
        Feedback.Title = "操作未完成";
        Feedback.Message = message;
        Feedback.IsOpen = true;
    }

    private async Task<bool> MutateAsync(Func<Task<HistoryMutation>> action)
    {
        if (disposed || mutating || loading || filterPending) return false;
        if (externalPending)
        {
            ShowError("历史记录已更新，请取消后刷新再修改。草稿将保留在当前对话框中。");
            return false;
        }
        mutating = true;
        UpdateCommands();
        try
        {
            var result = await action();
            if (disposed) return true;
            Feedback.IsOpen = false;
            await ReloadAsync(result.RemovedIds.Contains(result.Id) ? null : result.Id);
            return true;
        }
        catch (Exception error)
        {
            if (!disposed) ShowError(error.Message);
            return false;
        }
        finally
        {
            mutating = false;
            if (!disposed) UpdateCommands();
        }
    }

    private async Task ShowTextDialogAsync(string title, string initial, Func<string, Task<HistoryMutation>> save)
    {
        if (disposed || activeDialog is not null || mutating || loading || filterPending) return;
        var input = new TextBox { Text = initial, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap,
            MinWidth = 360, MaxHeight = 300, PlaceholderText = "输入内容" };
        // TextBox normalizes line endings. Opening and accepting without edits must preserve the original payload.
        string initialInput = input.Text;
        var error = new TextBlock { TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
        var content = new StackPanel { Spacing = 12 };
        content.Children.Add(input);
        content.Children.Add(error);
        var dialog = new ContentDialog { Title = title, Content = content, PrimaryButtonText = "保存",
            CloseButtonText = "取消", DefaultButton = ContentDialogButton.Primary, XamlRoot = XamlRoot };
        dialog.PrimaryButtonClick += async (_, args) =>
        {
            var deferral = args.GetDeferral();
            dialog.IsPrimaryButtonEnabled = false;
            input.IsEnabled = false;
            try
            {
                string value = input.Text == initialInput ? initial : input.Text;
                args.Cancel = !await MutateAsync(() => save(value));
                if (args.Cancel && !disposed)
                {
                    error.Text = Feedback.Message;
                    error.Visibility = Visibility.Visible;
                }
            }
            finally
            {
                if (!disposed)
                {
                    input.IsEnabled = true;
                    dialog.IsPrimaryButtonEnabled = true;
                }
                deferral.Complete();
            }
        };
        activeDialog = dialog;
        dialogInput = input;
        UpdateCommands();
        try { await dialog.ShowAsync(); }
        catch (Exception failure) { if (!disposed) ShowError(failure.Message); }
        finally
        {
            activeDialog = null;
            dialogInput = null;
            if (!disposed) UpdateCommands();
        }
    }

    private async Task DeleteSelectedAsync()
    {
        if (disposed || activeDialog is not null || mutating || loading || filterPending || externalPending
            || revision is not { } selectedRevision) return;
        long[] ids = HistoryList.SelectedItems.Cast<HistoryRow>().Select(row => row.Id).ToArray();
        if (ids.Length == 0) return;
        if (ids.Length > 1000) { ShowError("一次最多删除 1000 条，请缩小选择范围。"); return; }
        var dialog = new ContentDialog { Title = ids.Length == 1 ? "删除这条记录？" : $"删除所选 {ids.Length} 条记录？",
            Content = "删除后无法撤销。置顶或带标签的所选记录也会删除。",
            PrimaryButtonText = "删除", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close,
            XamlRoot = XamlRoot };
        activeDialog = dialog;
        UpdateCommands();
        try
        {
            var result = await dialog.ShowAsync();
            activeDialog = null;
            if (!disposed && result == ContentDialogResult.Primary)
                await MutateAsync(() => core!.DeleteEntriesAsync(ids, selectedRevision));
        }
        catch (Exception error) { if (!disposed) ShowError(error.Message); }
        finally { activeDialog = null; if (!disposed) UpdateCommands(); }
    }

    private async Task ShowClearDialogAsync()
    {
        if (disposed || activeDialog is not null || mutating || loading || filterPending || externalPending) return;
        mutating = true; UpdateCommands();
        try
        {
            var preview = await core!.GetClearHistoryPreviewAsync(lifetime.Token);
            if (disposed) return;
            if (preview.EligibleCount == 0)
            {
                Feedback.Title = "没有可清空的普通历史";
                Feedback.Message = "置顶和带标签的记录已保留。";
                Feedback.Severity = InfoBarSeverity.Informational; Feedback.IsOpen = true;
                return;
            }
            var dialog = new ContentDialog
            {
                Title = "清空普通历史？",
                Content = $"将删除 {preview.StoredCount} 条本地记录和 {preview.SessionCount} 条会话记录。\n"
                    + "保留所有置顶和带标签的记录。此操作作用于整个历史，不受搜索或筛选影响，删除后无法撤销。",
                PrimaryButtonText = "清空", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close,
                XamlRoot = XamlRoot
            };
            activeDialog = dialog; mutating = false; UpdateCommands();
            var choice = await dialog.ShowAsync();
            activeDialog = null;
            if (disposed || choice != ContentDialogResult.Primary) return;
            mutating = true; UpdateCommands();
            var result = await core.ClearHistoryAsync(preview.Revision, lifetime.Token);
            if (disposed) return;
            await ReloadAsync(0);
            if (disposed) return;
            Feedback.Title = "已清空普通历史";
            Feedback.Message = $"已删除 {result.RemovedCount} 条记录，置顶和带标签的记录已保留。"
                + (result.CompactionError is null ? "" : " 数据库空间压缩未完成；历史清空已经生效。");
            Feedback.Severity = result.CompactionError is null ? InfoBarSeverity.Success : InfoBarSeverity.Warning;
            Feedback.IsOpen = true;
        }
        catch (OperationCanceledException) when (disposed) { }
        catch (Exception error) { if (!disposed) ShowError(error is CoreException { Code: "history_conflict" }
            ? "历史记录已变化，本次清空未执行。请刷新后重新查看清空范围。" : error.Message); }
        finally { activeDialog = null; mutating = false; if (!disposed) UpdateCommands(); }
    }

    private static IEnumerable<T> Descendants<T>(DependencyObject root) where T : DependencyObject
    {
        for (int index = 0; index < VisualTreeHelper.GetChildrenCount(root); index++)
        {
            var child = VisualTreeHelper.GetChild(root, index);
            if (child is T match) yield return match;
            foreach (var nested in Descendants<T>(child)) yield return nested;
        }
    }

    private void AttachScroll()
    {
        if (listScroll is not null) listScroll.ViewChanged -= OnScroll;
        listScroll = Descendants<ScrollViewer>(HistoryList).FirstOrDefault();
        if (listScroll is not null) listScroll.ViewChanged += OnScroll;
    }

    private void OnScroll(object? sender, ScrollViewerViewChangedEventArgs args)
    {
        if (!disposed && !args.IsIntermediate && listScroll is { } scroll && scroll.ScrollableHeight > 0 &&
            scroll.VerticalOffset >= scroll.ScrollableHeight - 160) _ = LoadAsync(false);
    }

    public void Dispose()
    {
        if (disposed) return;
        disposed = true;
        generation++;
        detailGeneration++;
        lifetime.Cancel();
        loadCancellation?.Cancel();
        detailCancellation?.Cancel();
        if (debounce is not null) { debounce.Stop(); debounce.Tick -= OnDebounce; }
        if (poll is not null) { poll.Stop(); poll.Tick -= OnPoll; }
        if (listScroll is not null) listScroll.ViewChanged -= OnScroll;
        activeDialog?.Hide();
        selectedEntry = null;
        ImagePreview.Clear();
        DetailText.Text = "";
        loadCancellation?.Dispose();
        detailCancellation?.Dispose();
        lifetime.Dispose();
    }

    // Smoke helpers operate the real controls; they do not bypass production click handlers.
    internal bool IsBusy => loading || mutating || filterPending || captureBusy || targetBusy;
    internal bool CaptureRunning => captureStatus?.Running == true;
    internal ulong CaptureStarts => captureStatus?.Starts ?? 0;
    internal IReadOnlyList<HistoryRow> Rows => rows;
    internal string Detail => DetailText.Text;
    internal bool HasImagePreview => ImagePreview.HasImage;
    internal long? PreviewedImageId => ImagePreview.DisplayedId;
    internal int ImagePixelWidth => ImagePreview.PixelWidth;
    internal bool CanCopy => CopyButton.IsEnabled;
    internal bool CanPaste => PasteButton.IsEnabled;
    internal bool TargetTrackingRunning => targetStatus?.Running == true;
    internal bool CanReveal => RevealButton.Visibility == Visibility.Visible;
    internal bool ExternalPending => externalPending;
    internal bool HasDialog => activeDialog is not null;
    internal string DialogText => dialogInput?.Text ?? "";
    internal bool HasError => Feedback.IsOpen && Feedback.Severity == InfoBarSeverity.Error;
    internal int RealizedCount => Enumerable.Range(0, rows.Count).Count(index => HistoryList.ContainerFromIndex(index) is not null);
    internal void Search(string text) => SearchText.Text = text;
    internal void Filter(int typeIndex, string tag, bool pinned)
    {
        TypeFilter.SelectedIndex = typeIndex;
        TagFilter.Text = tag;
        PinnedOnly.IsChecked = pinned;
    }
    internal void Select(long id) => SelectMany(id);
    internal void SelectMany(params long[] ids)
    {
        HistoryList.SelectedItems.Clear();
        foreach (long id in ids) HistoryList.SelectedItems.Add(rows.First(row => row.Id == id));
    }
    internal bool CanDelete => DeleteButton.IsEnabled;
    internal void ScrollToEnd() => listScroll?.ChangeView(null, listScroll.ScrollableHeight, null, true);
    internal void Invoke(string name)
    {
        var button = name switch
        {
            "capture" => (ButtonBase)CaptureButton, "add" => AddButton,
            "paste" => PasteButton, "paste-plain" => PastePlainButton, "target-start" => StartTargetButton,
            "copy" => CopyButton, "copy-plain" => CopyPlainButton, "edit" => EditButton, "pin" => PinButton,
            "tags" => TagsButton, "delete" => DeleteButton, "clear" => ClearButton, "more" => MoreButton,
            "reveal" => RevealButton, _ => throw new ArgumentOutOfRangeException(nameof(name))
        };
        if (!button.IsEnabled) throw new InvalidOperationException($"{name} is disabled.");
        var peer = FrameworkElementAutomationPeer.CreatePeerForElement(button);
        ((IInvokeProvider)peer.GetPattern(PatternInterface.Invoke)).Invoke();
    }
    internal void SetDialogText(string text) => (dialogInput ?? throw new InvalidOperationException("No text dialog.")).Text = text;
    internal void CancelDialog()
    {
        var button = Descendants<Button>(activeDialog ?? throw new InvalidOperationException("No dialog."))
            .First(button => button.Name == "CloseButton");
        ((IInvokeProvider)FrameworkElementAutomationPeer.CreatePeerForElement(button).GetPattern(PatternInterface.Invoke)).Invoke();
    }
    internal void ConfirmDialog()
    {
        var button = Descendants<Button>(activeDialog ?? throw new InvalidOperationException("No dialog."))
            .First(button => button.Name == "PrimaryButton");
        ((IInvokeProvider)FrameworkElementAutomationPeer.CreatePeerForElement(button).GetPattern(PatternInterface.Invoke)).Invoke();
    }
}
