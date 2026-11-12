using System.Collections.ObjectModel;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Input;
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
    internal bool ManageCaptureStartup { get; set; } = true;
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
    private bool started, disposed, loading, mutating, polling, filterPending, hasMore, externalPending, relabeling;
    private ClipboardEntry? selectedEntry;
    private string? exactTagFilter;
    private FrameworkElement? hoveredAnchor;
    private bool compactMode;
    internal event Action<CompactPreviewRequest>? PreviewRequested;
    internal event Action? PreviewInvalidated;
    internal Func<bool>? IsExplicitPreviewActive { get; set; }
    internal bool CompactMode
    {
        get => compactMode;
        set
        {
            if (compactMode == value) return;
            CancelCardCopy(); CancelContentOpen(); InvalidateThumbnails();
            compactMode = value;
            PreviewInvalidated?.Invoke(); hoveredAnchor = null;
            HistoryList.ItemTemplate = (DataTemplate)Resources[value ? "CompactHistoryTemplate" : "StandardHistoryTemplate"];
            HistoryCards.ItemTemplate = (DataTemplate)Resources[value ? "CompactCardHistoryTemplate" : "CardHistoryTemplate"];
            UpdateCommands();
        }
    }
    internal void SetPreviewError(string message) => ShowError(message);

    private ContentDialog? activeDialog;
    private TextBox? dialogInput;
    private TagPickerView? tagPicker;
    private Action? feedbackRender;
    private Action? dialogChrome;
    private long? storedCount;
    private int? sessionCount;
    private string? captureFailureMessage, targetErrorMessage;
    private bool detailTextFallback, historyLoaded;

    /// <summary>
    /// Re-derives every localized string of this surface from the shared language. It never talks to the core:
    /// selection, loaded detail, scroll offset, filters, drafts and busy state stay exactly where they were.
    /// </summary>
    // A combo box caches the string it shows for the current selection, so relabelled items only reach the
    // collapsed box after the selection is re-asserted; the handlers ignore that re-assert.
    private static void Reassert(ComboBox combo)
    {
        int index = combo.SelectedIndex;
        if (index < 0) return;
        combo.SelectedIndex = -1;
        combo.SelectedIndex = index;
    }

    internal void ApplyLanguage()    {
        if (disposed) return;
        var text = HistoryStrings.Current;
        var common = CommonStrings.Current;
        PageTitle.Text = text.Title;
        ApplyWorkspaceLanguage();
        DatabaseNote.Text = text.DatabaseNote;
        ToolTipService.SetToolTip(DatabaseNote, text.DatabaseNoteToolTip);
        ReturnTargetButton.Label = text.ReturnTargetLabel;
        ToolTipService.SetToolTip(ReturnTargetButton, text.ReturnTargetToolTip);
        StartTargetButton.Label = text.StartTargetLabel;
        ToolTipService.SetToolTip(StartTargetButton, text.StartTargetToolTip);
        SearchText.PlaceholderText = text.SearchPlaceholder;
        AutomationProperties.SetName(SearchText, text.SearchName);
        AutomationProperties.SetName(TypeFilter, text.TypeFilterName);
        TypeAllItem.Content = text.TypeAll; TypeTextItem.Content = text.TypeText;
        TypeCodeItem.Content = text.TypeCode; TypeLinkItem.Content = text.TypeLink;
        TypeRichTextItem.Content = text.TypeRichText; TypeImageItem.Content = text.TypeImage;
        TypeFileItem.Content = text.TypeFile; TypeVideoItem.Content = text.TypeVideo;
        TagFilter.PlaceholderText = text.TagFilterPlaceholder;
        AutomationProperties.SetName(TagFilter, text.TagFilterName);
        PinnedOnly.Content = text.PinnedOnly;
        TagOnly.Content = text.TagOnly;
        AutomationProperties.SetName(OrderChoice, text.SortName);
        ToolTipService.SetToolTip(OrderChoice, text.SortToolTip);
        SortPinnedItem.Content = text.SortPinned; SortTimeItem.Content = text.SortTime;
        SortUsageItem.Content = text.SortUsage;
        AutomationProperties.SetName(LayoutChoice, text.LayoutName);
        ToolTipService.SetToolTip(LayoutChoice, text.LayoutToolTip);
        LayoutListItem.Content = text.LayoutList; LayoutCardsItem.Content = text.LayoutCards;
        relabeling = true;
        try { Reassert(TypeFilter); Reassert(OrderChoice); Reassert(LayoutChoice); }
        finally { relabeling = false; }        AutomationProperties.SetName(ReloadLayoutButton, text.ReloadLayoutName);
        ToolTipService.SetToolTip(ReloadLayoutButton, text.ReloadLayoutToolTip);
        AddButton.Label = common.Add;
        EditButton.Label = common.Edit;
        PasteButton.Label = common.Paste;
        ToolTipService.SetToolTip(PasteButton, text.PasteToolTip);
        CopyButton.Label = common.Copy;
        ToolTipService.SetToolTip(CopyButton, text.CopyToolTip);
        OpenButton.Label = common.Open;
        ToolTipService.SetToolTip(OpenButton, text.OpenToolTip);
        PastePlainButton.Label = common.PastePlainText;
        TagsButton.Label = common.Tags;
        CopyPlainButton.Label = common.CopyPlainText;
        PreviewButton.Label = text.PreviewLabel;
        ClearButton.Label = text.ClearHistory;
        QueueSelectionButton.Label = QueueStrings.Replace; QueueButton.Label = QueueStrings.Manage;
        ToolTipService.SetToolTip(QueueSelectionButton, QueueStrings.Replace);
        ToolTipService.SetToolTip(QueueButton, QueueStrings.Manage);
        CaptureInfo.Title = text.CaptureStatusTitle;
        AutomationProperties.SetName(HistoryList, text.ListName);
        AutomationProperties.SetName(HistoryCards, text.CardsName);
        AutomationProperties.SetName(DetailText, text.DetailTextName);
        RevealButton.Content = text.RevealSensitive;
        MoreButton.Content = text.LoadMore;
        UpdateRefreshLabel();
        // AppBarButton derives a tooltip from Label and keeps that copy when only the label changes; keep them in step
        // so a language switch cannot leave the previous language in a hover hint.
        foreach (var bar in new[] { AddButton, EditButton, PastePlainButton, TagsButton, CopyPlainButton, PreviewButton, ClearButton } )
            ToolTipService.SetToolTip(bar, bar.Label);
        // Rows and their containers stay; only the labels derived from the table are re-raised.
        foreach (var row in rows) row.RefreshText();
        UpdateListStatus();
        UpdateCommands();
        ApplyDetailHeading();
        if (ActiveList.SelectedItems.Count == 1 && ActiveList.SelectedItem is HistoryRow single && !single.IsStale)
            DetailMetadata.Text = single.Metadata;
        if (captureFailureMessage is not null) RenderCaptureFailure();
        else if (captureStatus is not null) ApplyCapture(captureStatus);
        else
        {
            CaptureStatus.Text = text.CapturePreparing;
            CaptureButton.Label = text.CaptureStart;
        }
        if (targetErrorMessage is not null) SetTargetError(targetErrorMessage);
        else if (targetStatus is not null) ApplyTarget(targetStatus);
        else TargetInfo.Text = text.TargetReading;
        if (detailTextFallback) DetailText.Text = text.ImageUnavailable;
        ImagePreview.ApplyLanguage();
        DocumentPreview.ApplyLanguage();
        foreach (var view in Descendants<HistoryThumbnailView>(HistoryCards)) view.ApplyLanguage();
        foreach (var button in Descendants<Button>(HistoryCards)) ApplyCardButtonText(button);
        if (Feedback.IsOpen) feedbackRender?.Invoke();
        dialogChrome?.Invoke();
    }
    internal TagPickerView? TagPicker => tagPicker;

    public HistoryView()
    {
        InitializeComponent();
        WorkspaceRoot.SizeChanged += (_, _) => UpdateWorkspaceLayout();
        InitializeHistoryLayouts();
        InitializeThumbnails();
        InitializeCardCopies();
        InitializeContentOpens();
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
        TagFilter.TextChanging += (_, _) => exactTagFilter = null;
        // A relabel of the item contents re-asserts the selection, which must not debounce a new query.
        TypeFilter.SelectionChanged += (_, _) => { if (!relabeling) ScheduleSearch(); };
        OrderChoice.SelectionChanged += (_, _) => { if (!relabeling) ScheduleSearch(); };
        PinnedOnly.Checked += (_, _) => ScheduleSearch();
        PinnedOnly.Unchecked += (_, _) => ScheduleSearch();
        TagOnly.Checked += (_, _) => ScheduleSearch();
        TagOnly.Unchecked += (_, _) => ScheduleSearch();
        Loaded += (_, _) =>
        {
            if (started || disposed) return;
            started = true;
            poll!.Start();
            _ = ReloadAsync();
            if (ManageCaptureStartup) _ = SetCaptureAsync(startCapture);
        };
        CaptureButton.Click += async (_, _) => await SetCaptureAsync(captureStatus?.Running != true);
        AddButton.Click += async (_, _) => await ShowTextDialogAsync(text => text.AddTitle, "", (value, token) => core!.AddManualAsync(value, cancellationToken: token));
        ReturnTargetButton.Click += async (_, _) => await ReturnTargetAsync();
        StartTargetButton.Click += async (_, _) => await StartTargetAsync();
        PasteButton.Click += async (_, _) => await PasteSelectedAsync(false);
        PastePlainButton.Click += async (_, _) => await PasteSelectedAsync(true);
        PreviewButton.Click += (_, _) =>
        {
            if (ActiveList.SelectedItem is HistoryRow row) PreviewRow(row.Id);
        };
        CopyButton.Click += async (_, _) => await CopySelectedAsync(false);
        CopyPlainButton.Click += async (_, _) => await CopySelectedAsync(true);
        EditButton.Click += async (_, _) =>
        {
            if (selectedEntry is { } entry)
                await ShowTextDialogAsync(text => text.EditTitle, entry.Content, (value, token) => core!.EditEntryAsync(entry.Id, value, cancellationToken: token));
        };
        TagsButton.Click += async (_, _) =>
        {
            if (ActiveList.SelectedItem is HistoryRow row)
                await ShowTagPickerAsync(row.Id);
        };
        PinButton.Click += async (_, _) =>
        {
            if (ActiveList.SelectedItem is HistoryRow row)
                await MutateAsync(() => core!.SetPinnedAsync(row.Id, !row.Summary.IsPinned));
        };
        DeleteButton.Click += async (_, _) => await DeleteSelectedAsync();
        ClearButton.Click += async (_, _) => await ShowClearDialogAsync();
        QueueSelectionButton.Click += async (_, _) => await ShowPasteQueueAsync(true);
        QueueButton.Click += async (_, _) => await ShowPasteQueueAsync();
        RefreshButton.Click += async (_, _) => await ReloadAsync();
        MoreButton.Click += async (_, _) => await LoadAsync(false);
        RevealButton.Click += async (_, _) => await LoadDetailAsync(true);
        UpdateCommands();
        LanguageService.Changed += ApplyLanguage;
        ApplyLanguage();
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

    internal void SetTargetError(string message)
    {
        targetErrorMessage = message;
        TargetInfo.Text = string.Format(HistoryStrings.Current.TargetErrorFormat, message);
    }

    private void ApplyTarget(TargetTrackingStatus status)
    {
        if (targetStatus is not null && status.Generation < targetStatus.Generation) return;
        targetStatus = status; targetErrorMessage = null;
        var text = HistoryStrings.Current;
        TargetInfo.Text = status.HasTarget ? string.Format(text.TargetFormat, status.Application)
            : status.Running ? text.TargetNone : text.TargetStopped;
        if (status.LastError is not null) TargetInfo.Text += string.Format(text.TargetErrorSuffix, status.LastError);
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
            if (!disposed)
                PresentFeedback(InfoBarSeverity.Success, () =>
                {
                    var text = HistoryStrings.Current;
                    Feedback.Title = text.ReturnedTitle; Feedback.Message = text.ReturnedMessage;
                });
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { if (!disposed) ShowError(error.Message); }
        finally { targetBusy = false; if (!disposed) UpdateCommands(); }
    }

    private void ApplyCapture(CaptureStatus status)
    {
        var text = HistoryStrings.Current;
        captureStatus = status; captureFailureMessage = null;
        CaptureButton.Label = status.Running ? text.CapturePause : text.CaptureStart;
        CaptureButton.Icon = new SymbolIcon(status.Running ? Symbol.Pause : Symbol.Play);
        string state = status.Running ? text.CaptureListening : text.CapturePaused;
        CaptureStatus.Text = string.Format(text.CaptureStatusFormat, state, status.Accepted);
        ToolTipService.SetToolTip(CaptureStatus, string.Format(text.CaptureStatusToolTip, status.Accepted, status.Ignored, status.Failed));
        CaptureInfo.Severity = InfoBarSeverity.Warning;
        CaptureInfo.Message = status.LastError is null ? "" : string.Format(text.CaptureErrorFormat, status.LastError);
        CaptureInfo.IsOpen = status.LastError is not null;
    }
    private void RenderCaptureFailure()
    {
        var text = HistoryStrings.Current;
        CaptureStatus.Text = text.CaptureUnknown;
        CaptureInfo.IsOpen = true;
        CaptureInfo.Severity = InfoBarSeverity.Error;
        CaptureInfo.Message = string.Format(text.CaptureUnknownFormat, captureFailureMessage);
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
                captureFailureMessage = error.Message;
                RenderCaptureFailure();
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
        CancelCardCopy(); CancelContentOpen(); InvalidateThumbnails();
        PreviewInvalidated?.Invoke(); hoveredAnchor = null;
        if (!started || disposed) return;
        generation++;
        loadCancellation?.Cancel();
        ActiveList.SelectedItems.Clear();
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
            CancelCardCopy(); CancelContentOpen(); InvalidateThumbnails();
            PreviewInvalidated?.Invoke(); hoveredAnchor = null;
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
            Tag: exactTagFilter ?? (string.IsNullOrWhiteSpace(TagFilter.Text) ? null : TagFilter.Text.Trim()),
            TagOnly: TagOnly.IsChecked == true, PinnedOnly: PinnedOnly.IsChecked == true, Sort: SortOrder);
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
                selectId ??= (ActiveList.SelectedItem as HistoryRow)?.Id;
                rows.Clear();
                externalPending = false; historyLoaded = true;
                UpdateRefreshLabel();
        // AppBarButton derives a tooltip from Label and keeps that copy when only the label changes; keep them in step
        // so a language switch cannot leave the previous language in a hover hint.
        foreach (var bar in new[] { AddButton, EditButton, PastePlainButton, TagsButton, CopyPlainButton, PreviewButton, ClearButton } )
            ToolTipService.SetToolTip(bar, bar.Label);
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
            storedCount = page.StoredCount; sessionCount = page.SessionCount;
            UpdateListStatus();
            if (selectId is { } selected)
                ActiveList.SelectedItem = rows.FirstOrDefault(row => row.Id == selected);
            if (reset) { tagLayoutScope = query.Tag is not null; ApplyAcceptedLayout(); }
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
            if (!disposed) tagPicker?.ObserveRevision(changes.Revision);
            if (disposed || current != generation || loading || mutating || changes.Revision == observedRevision) return;
            PreviewInvalidated?.Invoke(); hoveredAnchor = null;
            InvalidateThumbnails();
            CancelCardCopy(); CancelContentOpen();
            observedRevision = changes.Revision;
            if (externalPending || activeDialog is not null || ActiveList.SelectedItem is not null || (listScroll?.VerticalOffset ?? 0) > 32)
            {
                externalPending = true;
                UpdateRefreshLabel();
        // AppBarButton derives a tooltip from Label and keeps that copy when only the label changes; keep them in step
        // so a language switch cannot leave the previous language in a hover hint.
        foreach (var bar in new[] { AddButton, EditButton, PastePlainButton, TagsButton, CopyPlainButton, PreviewButton, ClearButton } )
            ToolTipService.SetToolTip(bar, bar.Label);
                var changedIds = changes.Changes.SelectMany(change => change.RemovedIds.Append(change.Id)
                    .Concat(change.ReplacedId is { } replaced ? new[] { replaced } : Array.Empty<long>())).ToHashSet();
                for (int index = 0; index < rows.Count; index++)
                    if (changes.ResyncRequired || changedIds.Contains(rows[index].Id))
                        rows[index] = new HistoryRow(rows[index].Summary, isStale: true);
                if ((ActiveList.SelectedItem as HistoryRow)?.IsStale == true) ActiveList.SelectedItem = null;
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
        CancelContentOpen();
        PreviewInvalidated?.Invoke();
        detailGeneration++;
        var current = detailGeneration;
        detailCancellation?.Cancel();
        detailCancellation?.Dispose();
        detailCancellation = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
        var token = detailCancellation.Token;
        selectedEntry = null;
        ImagePreview.Clear();
        DocumentPreview.Clear();
        DetailText.Visibility = Visibility.Visible;
        detailTextFallback = false;
        DetailText.Text = "";
        RevealButton.Visibility = Visibility.Collapsed;
        if (disposed) return;
        if (ActiveList.SelectedItems.Count != 1 || ActiveList.SelectedItem is not HistoryRow row)
        {
            ApplyDetailHeading();
            DetailMetadata.Text = "";
            UpdateCommands();
            return;
        }
        if (row.IsStale) { UpdateCommands(); return; }
        ApplyDetailHeading();
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
                (ActiveList.SelectedItem as HistoryRow)?.Id != row.Id) return;
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
                    detailTextFallback = true;
                    DetailText.Text = HistoryStrings.Current.ImageUnavailable;
                    ShowError(error.Message);
                }
            }
            else if (entry.ContentType == "rich_text")
            {
                try
                {
                    var preview = await core.GetDocumentPreviewAsync(entry.Id, revision!.Value, reveal, token);
                    if (disposed || token.IsCancellationRequested || current != detailGeneration) return;
                    detailTextFallback = false;
                    DetailText.Text = entry.Content;
                    DocumentPreview.Show(preview, entry.Content);
                    DetailText.Visibility = Visibility.Collapsed;
                }
                catch (CoreException error) when (error.Code is not "history_conflict" and not "sensitive_confirmation")
                {
                    if (disposed || token.IsCancellationRequested || current != detailGeneration) return;
                    detailTextFallback = false;
                    DetailText.Text = entry.Content;
                    ShowError(error.Message);
                }
            }
            else { detailTextFallback = false; DetailText.Text = entry.Content; }
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
        UpdateCardCopyButtons();
        UpdateContentOpenControls();
        if (!CanReadThumbnails) InvalidateThumbnails(); else QueueThumbnailRefresh();
        bool busy = loading || mutating || filterPending || targetBusy;
        var selected = ActiveList.SelectedItems.Count == 1 ? ActiveList.SelectedItem as HistoryRow : null;
        int selectedCount = ActiveList.SelectedItems.Count;
        var text = HistoryStrings.Current;
        SelectionInfo.Text = selectedCount == 0 ? text.SelectionHint : string.Format(text.SelectionCountFormat, selectedCount);
        bool showDetail = selected is { IsStale: false } && selectedEntry?.Id == selected.Id;
        DetailBody.Visibility = showDetail ? Visibility.Visible : Visibility.Collapsed;
        DetailHint.Visibility = showDetail ? Visibility.Collapsed : Visibility.Visible;
        DetailHintText.Text = selectedCount > 1 ? text.DetailMultipleHint : selected is { IsStale: true } ? text.DetailStaleHint
            : selected?.Summary.IsSensitive == true ? text.DetailSensitiveHint : selected is null ? text.DetailEmptyHint : text.DetailLoading;
        bool available = !busy && activeDialog is null;
        OrderChoice.IsEnabled = LayoutChoice.IsEnabled = ReloadLayoutButton.IsEnabled = available;
        if (busy || activeDialog is not null || externalPending) PreviewInvalidated?.Invoke();
        PreviewButton.IsEnabled = available && !externalPending && compactMode && selected is { IsStale: false }
            && selected.Summary.ContentType != "file" && (!selected.Summary.IsSensitive || selectedEntry is not null);
        ReturnTargetButton.IsEnabled = available && targetStatus?.HasTarget == true;
        StartTargetButton.IsEnabled = available;
        AddButton.IsEnabled = available && !externalPending;
        EditButton.IsEnabled = available && !externalPending && selectedEntry is not null;
        CopyButton.IsEnabled = CopyPlainButton.IsEnabled = available && !externalPending && selectedEntry is not null && selected is { IsStale: false };
        PasteButton.IsEnabled = PastePlainButton.IsEnabled = CopyButton.IsEnabled && targetStatus is { Running: true, HasTarget: true };
        PinButton.IsEnabled = TagsButton.IsEnabled = available && !externalPending && selected is { IsStale: false };
        DeleteButton.IsEnabled = available && !externalPending && ActiveList.SelectedItems.Count > 0
            && ActiveList.SelectedItems.Cast<HistoryRow>().All(row => !row.IsStale);
        DeleteButton.Label = ActiveList.SelectedItems.Count > 1
            ? string.Format(text.DeleteSelectedFormat, ActiveList.SelectedItems.Count) : CommonStrings.Current.Delete;
        ToolTipService.SetToolTip(DeleteButton, DeleteButton.Label);
        ToolTipService.SetToolTip(PinButton, PinButton.Label);
        PinButton.Label = selected?.Summary.IsPinned == true ? text.UnpinAction : text.PinAction;
        ClearButton.IsEnabled = available && !externalPending;
        QueueSelectionButton.IsEnabled = core is not null && DeleteButton.IsEnabled && selectedCount <= 1000;
        QueueButton.IsEnabled = core is not null && available;
        RefreshButton.IsEnabled = available;
        MoreButton.IsEnabled = available && hasMore;
        MoreButton.Visibility = hasMore ? Visibility.Visible : Visibility.Collapsed;
        RevealButton.IsEnabled = available && !externalPending;
        LoadingIndicator.IsActive = busy;
        UpdateWorkspaceLayout();
    }

    private async Task CopySelectedAsync(bool plainText)
    {
        if (disposed || mutating || loading || filterPending || externalPending || activeDialog is not null
            || selectedEntry is not { } entry || ActiveList.SelectedItem is not HistoryRow { IsStale: false } row) return;
        mutating = true;
        UpdateCommands();
        try
        {
            await core!.CopyEntryAsync(entry.Id, plainText, allowSensitive: row.Summary.IsSensitive, lifetime.Token, expectedRevision: revision!.Value);
            if (!disposed) ShowCopySuccess(plainText);
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (Exception error) { if (!disposed) ShowCopyError(error); }
        finally { mutating = false; if (!disposed) UpdateCommands(); }
    }

    private async Task PasteSelectedAsync(bool plainText)
    {
        if (disposed || mutating || loading || filterPending || externalPending || targetBusy || activeDialog is not null
            || targetStatus is not { Running: true, HasTarget: true } target || selectedEntry is not { } entry
            || ActiveList.SelectedItem is not HistoryRow { IsStale: false } row) return;
        mutating = true;
        UpdateCommands();
        try
        {
            await core!.PasteEntryAsync(entry.Id, target.Generation, plainText,
                allowSensitive: row.Summary.IsSensitive, lifetime.Token);
            if (!disposed)
            {
                PresentFeedback(InfoBarSeverity.Success, () =>
                {
                    var text = HistoryStrings.Current;
                    Feedback.Title = text.PasteSentTitle; Feedback.Message = text.PasteSentMessage;
                });
            }
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (Exception error) { if (!disposed) ShowError(error.Message); }
        finally { mutating = false; if (!disposed) UpdateCommands(); }
    }

    private void ShowError(string message) => PresentFeedback(InfoBarSeverity.Error, () =>
    {
        Feedback.Title = HistoryStrings.Current.ErrorTitle;
        Feedback.Message = message;
    });
    private void ShowError(Func<HistoryStrings, string> message) => PresentFeedback(InfoBarSeverity.Error, () =>
    {
        var text = HistoryStrings.Current;
        Feedback.Title = text.ErrorTitle;
        Feedback.Message = message(text);
    });
    // The renderer is retained so an open bar follows a live language switch instead of freezing one language.
    private void PresentFeedback(InfoBarSeverity severity, Action render)
    {
        Feedback.Severity = severity;
        render();
        feedbackRender = render;
        Feedback.IsOpen = true;
    }
    private void CloseFeedback()
    {
        feedbackRender = null;
        Feedback.IsOpen = false;
    }
    private void UpdateRefreshLabel() => RefreshButton.Label = !historyLoaded ? HistoryStrings.Current.RefreshHistory
        : externalPending ? HistoryStrings.Current.RefreshPending : HistoryStrings.Current.RefreshLabel;
    private void UpdateListStatus()
    {
        if (storedCount is not { } stored) return;
        ListStatus.Text = string.Format(HistoryStrings.Current.ListStatusFormat, rows.Count, stored, sessionCount ?? 0);
    }
    private void ApplyDetailHeading() => DetailTitle.Text = ActiveList.SelectedItems.Count > 1
        ? string.Format(HistoryStrings.Current.DetailTitleFormat, ActiveList.SelectedItems.Count)
        : ActiveList.SelectedItem is HistoryRow row ? row.TypeLabel : HistoryStrings.Current.DetailTitleDefault;
    // Card labels live in a recycled template, so they are re-derived from the tag whenever it is realized.
    internal static void ApplyCardButtonText(Button button)
    {
        var text = HistoryStrings.Current;
        switch (button.Tag)
        {
            case "card-copy":
                AutomationProperties.SetName(button, text.CardCopyName);
                ToolTipService.SetToolTip(button, text.CardCopyToolTip);
                break;
            case "card-copy-plain":
                AutomationProperties.SetName(button, text.CardCopyPlainName);
                ToolTipService.SetToolTip(button, text.CardCopyPlainToolTip);
                break;
            case "card-open":
                AutomationProperties.SetName(button, text.CardOpenName);
                ToolTipService.SetToolTip(button, text.CardOpenToolTip);
                break;
        }
    }

    private async Task<bool> MutateAsync(Func<Task<HistoryMutation>> action, bool revisionGuarded = false)
    {
        if (disposed || mutating || loading || filterPending) return false;
        // Only callers whose core mutation atomically checks an accepted revision may bypass this UI guard.
        if (externalPending && !revisionGuarded)
        {
            ShowError(text => text.MutateBlocked);
            return false;
        }
        mutating = true;
        UpdateCommands();
        try
        {
            var result = await action();
            if (disposed) return true;
            CloseFeedback();
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

    internal void DismissTagPicker()
    {
        if (tagPicker is null) return;
        tagPicker.Dispose(); activeDialog?.Hide();
    }

    private async Task ShowTagPickerAsync(long id)
    {
        if (disposed || core is null || activeDialog is not null || mutating || loading || filterPending || externalPending) return;
        var picker = new TagPickerView();
        var dialog = new ContentDialog { Content = picker, DefaultButton = ContentDialogButton.Close,
            XamlRoot = XamlRoot, IsPrimaryButtonEnabled = false };
        dialogChrome = () =>
        {
            var text = HistoryStrings.Current;
            dialog.Title = text.TagPickerTitle;
            dialog.PrimaryButtonText = CommonStrings.Current.Save;
            dialog.CloseButtonText = CommonStrings.Current.Cancel;
        };
        dialogChrome();
        tagPicker = picker; activeDialog = dialog;
        void UpdateSave() => dialog.IsPrimaryButtonEnabled = picker.CanSave;
        picker.StateChanged += UpdateSave;
        dialog.PrimaryButtonClick += async (_, args) =>
        {
            var deferral = args.GetDeferral();
            try { args.Cancel = !await picker.SaveAsync(action => MutateAsync(action, revisionGuarded: true)); }
            finally { deferral.Complete(); }
        };
        dialog.Opened += async (_, _) => await picker.InitializeAsync(core, id);
        UpdateCommands();
        try { await dialog.ShowAsync(); }
        catch (Exception error) { if (!disposed) ShowError(error.Message); }
        finally
        {
            picker.StateChanged -= UpdateSave; picker.Dispose();
            tagPicker = null; activeDialog = null; dialogChrome = null;
            if (!disposed) UpdateCommands();
        }
    }

    private async Task ShowTextDialogAsync(Func<HistoryStrings, string> title, string initial,
        Func<string, CancellationToken, Task<HistoryMutation>> save)
    {
        if (disposed || activeDialog is not null || mutating || loading || filterPending) return;
        using var dialogCancellation = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
        bool dialogClosed = false;
        var input = new TextBox { Text = initial, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap,
            MinWidth = 360, MaxHeight = 300 };
        // TextBox normalizes line endings. Opening and accepting without edits must preserve the original payload.
        string initialInput = input.Text;
        var error = new TextBlock { TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
        var content = new StackPanel { Spacing = 12 };
        content.Children.Add(input);
        content.Children.Add(error);
        var dialog = new ContentDialog { Content = content, DefaultButton = ContentDialogButton.Primary, XamlRoot = XamlRoot };
        // The open dialog follows a live language switch; its draft text is never touched.
        dialogChrome = () =>
        {
            var text = HistoryStrings.Current;
            dialog.Title = title(text);
            dialog.PrimaryButtonText = CommonStrings.Current.Save;
            dialog.CloseButtonText = CommonStrings.Current.Cancel;
            input.PlaceholderText = text.InputPlaceholder;
            if (error.Visibility == Visibility.Visible) error.Text = Feedback.Message;
        };
        dialogChrome();
        dialog.Closing += (_, _) => dialogCancellation.Cancel();
        dialog.PrimaryButtonClick += async (_, args) =>
        {
            var deferral = args.GetDeferral();
            dialog.IsPrimaryButtonEnabled = false;
            input.IsEnabled = false;
            try
            {
                string value = input.Text == initialInput ? initial : input.Text;
                args.Cancel = !await MutateAsync(() => save(value, dialogCancellation.Token));
                if (args.Cancel && !disposed && !dialogClosed)
                {
                    error.Text = Feedback.Message;
                    error.Visibility = Visibility.Visible;
                }
            }
            finally
            {
                if (!disposed && !dialogClosed)
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
            dialogClosed = true; dialogCancellation.Cancel();
            input.Text = ""; error.Text = ""; content.Children.Clear(); dialog.Content = null;
            activeDialog = null;
            dialogInput = null; dialogChrome = null;
            if (!disposed) UpdateCommands();
        }
    }

    private async Task DeleteSelectedAsync()
    {
        if (disposed || activeDialog is not null || mutating || loading || filterPending || externalPending
            || revision is not { } selectedRevision) return;
        long[] ids = ActiveList.SelectedItems.Cast<HistoryRow>().Select(row => row.Id).ToArray();
        if (ids.Length == 0) return;
        if (ids.Length > 1000) { ShowError(text => text.DeleteTooMany); return; }
        var dialog = new ContentDialog { DefaultButton = ContentDialogButton.Close, XamlRoot = XamlRoot };
        dialogChrome = () =>
        {
            var text = HistoryStrings.Current;
            dialog.Title = ids.Length == 1 ? text.DeleteOneTitle : string.Format(text.DeleteManyTitleFormat, ids.Length);
            dialog.Content = text.DeleteBody;
            dialog.PrimaryButtonText = CommonStrings.Current.Delete;
            dialog.CloseButtonText = CommonStrings.Current.Cancel;
        };
        dialogChrome();
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
        finally { activeDialog = null; dialogChrome = null; if (!disposed) UpdateCommands(); }
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
                PresentFeedback(InfoBarSeverity.Informational, () =>
                {
                    var text = HistoryStrings.Current;
                    Feedback.Title = text.ClearNothingTitle; Feedback.Message = text.ClearNothingMessage;
                });
                return;
            }
            var dialog = new ContentDialog { DefaultButton = ContentDialogButton.Close, XamlRoot = XamlRoot };
            dialogChrome = () =>
            {
                var text = HistoryStrings.Current;
                dialog.Title = text.ClearTitle;
                dialog.Content = string.Format(text.ClearBodyFormat, preview.StoredCount, preview.SessionCount);
                dialog.PrimaryButtonText = CommonStrings.Current.Clear;
                dialog.CloseButtonText = CommonStrings.Current.Cancel;
            };
            dialogChrome();
            activeDialog = dialog; mutating = false; UpdateCommands();
            var choice = await dialog.ShowAsync();
            activeDialog = null;
            if (disposed || choice != ContentDialogResult.Primary) return;
            mutating = true; UpdateCommands();
            var result = await core.ClearHistoryAsync(preview.Revision, lifetime.Token);
            if (disposed) return;
            await ReloadAsync(0);
            if (disposed) return;
            PresentFeedback(result.CompactionError is null ? InfoBarSeverity.Success : InfoBarSeverity.Warning, () =>
            {
                var text = HistoryStrings.Current;
                Feedback.Title = text.ClearDoneTitle;
                Feedback.Message = string.Format(text.ClearDoneFormat, result.RemovedCount)
                    + (result.CompactionError is null ? "" : text.ClearCompactionWarning);
            });
        }
        catch (OperationCanceledException) when (disposed) { }
        catch (Exception error)
        {
            if (disposed) { }
            else if (error is CoreException { Code: "history_conflict" }) ShowError(text => text.ClearConflict);
            else ShowError(error.Message);
        }
        finally { activeDialog = null; dialogChrome = null; mutating = false; if (!disposed) UpdateCommands(); }
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
        listScroll = Descendants<ScrollViewer>(ActiveList).FirstOrDefault();
        if (listScroll is not null) listScroll.ViewChanged += OnScroll;
    }

    private void OnScroll(object? sender, ScrollViewerViewChangedEventArgs args)
    {
        RefreshThumbnails(); UpdateCardCopyButtons();
        PreviewInvalidated?.Invoke(); hoveredAnchor = null;
        if (!disposed && !switchingLayout && !args.IsIntermediate && listScroll is { } scroll && scroll.ScrollableHeight > 0 &&
            scroll.VerticalOffset >= scroll.ScrollableHeight - 160) _ = LoadAsync(false);
    }

    public void Dispose()
    {
        if (disposed) return;
        disposed = true; LanguageService.Changed -= ApplyLanguage; feedbackRender = null; dialogChrome = null;
        IsExplicitPreviewActive = null; exactTagFilter = null; OrderChoice.SelectedIndex = 0;
        PreviewInvalidated?.Invoke(); hoveredAnchor = null;
        generation++;
        detailGeneration++;
        lifetime.Cancel();
        loadCancellation?.Cancel();
        detailCancellation?.Cancel();
        if (debounce is not null) { debounce.Stop(); debounce.Tick -= OnDebounce; }
        if (poll is not null) { poll.Stop(); poll.Tick -= OnPoll; }
        if (listScroll is not null) listScroll.ViewChanged -= OnScroll;
        tagPicker?.Dispose();
        pasteQueueView?.Dispose();
        activeDialog?.Hide();
        ReleaseCardCopies();
        ReleaseContentOpens();
        ReleaseThumbnails();
        ReleaseHistoryLayouts();
        selectedEntry = null;
        ImagePreview.Clear();
        DocumentPreview.Clear();
        DetailText.Text = "";
        loadCancellation?.Dispose();
        detailCancellation?.Dispose();
        lifetime.Dispose();
    }

    private void OnRowPointerEntered(object sender, PointerRoutedEventArgs args)
    {
        if (IsExplicitPreviewActive?.Invoke() == true || args.Pointer.PointerDeviceType != Microsoft.UI.Input.PointerDeviceType.Mouse
            || sender is not FrameworkElement anchor || anchor.DataContext is not HistoryRow row) return;
        RequestPreview(row, anchor, requireHover: true);
    }
    private void OnRowPointerExited(object sender, PointerRoutedEventArgs args) => EndHover(sender);
    private void OnRowUnloaded(object sender, RoutedEventArgs args) => EndHover(sender);
    private void EndHover(object sender)
    {
        if (IsExplicitPreviewActive?.Invoke() == true || !ReferenceEquals(sender, hoveredAnchor)) return;
        hoveredAnchor = null; PreviewInvalidated?.Invoke();
    }
    private void RequestPreview(HistoryRow row, FrameworkElement anchor, bool requireHover)
    {
        if (Visibility != Visibility.Visible || revision is not { } expected || !compactMode || disposed || row.IsStale || externalPending
            || loading || mutating || filterPending || activeDialog is not null || row.Summary.ContentType == "file") return;
        bool allowSensitive = selectedEntry?.Id == row.Id && ActiveList.SelectedItems.Count == 1;
        if (row.Summary.IsSensitive && !allowSensitive) return;
        // Only an accepted hover owns pointer-exit cancellation. Explicit commands detach the
        // previous hover so a queued exit/unload cannot cancel their independent request.
        hoveredAnchor = requireHover ? anchor : null;
        PreviewRequested?.Invoke(new(row.Summary, expected, allowSensitive, anchor, () =>
            !disposed && Visibility == Visibility.Visible && compactMode && !externalPending && !loading && !mutating && !filterPending
            && activeDialog is null && revision == expected && rows.Contains(row) && anchor.IsLoaded
            && (ReferenceEquals(anchor, ActiveList) || (anchor.DataContext is HistoryRow current && current.Id == row.Id))
            && (!requireHover || ReferenceEquals(hoveredAnchor, anchor)), IsExplicit: !requireHover));
    }
    internal void PreviewRow(long id)
    {
        var row = rows.FirstOrDefault(row => row.Id == id);
        if (row is null) return;
        // Explicit keyboard/button requests also work before virtualization realizes the row.
        FrameworkElement? anchor = ActiveList.ContainerFromItem(row) is SelectorItem container
            ? Descendants<FrameworkElement>(container).FirstOrDefault(element => element.IsLoaded
                && element.DataContext is HistoryRow bound && bound.Id == row.Id) : null;
        RequestPreview(row, anchor ?? ActiveList, requireHover: false);
    }
    internal void CancelPreview() { hoveredAnchor = null; PreviewInvalidated?.Invoke(); }

    // Smoke helpers operate the real controls; they do not bypass production click handlers.
    internal bool IsBusy => loading || mutating || filterPending || captureBusy || targetBusy || tagPicker?.IsBusy == true;
    internal bool CaptureRunning => captureStatus?.Running == true;
    internal ulong CaptureStarts => captureStatus?.Starts ?? 0;
    internal IReadOnlyList<HistoryRow> Rows => rows;
    internal string Detail => DetailText.Text;
    internal bool HasDocumentPreview => DocumentPreview.HasDocument;
    internal long? PreviewedDocumentId => DocumentPreview.DisplayedId;
    internal int DocumentCells => DocumentPreview.CellCount;
    internal int DocumentMergedCells => DocumentPreview.MergedCellCount;
    internal int DocumentEmphasis => DocumentPreview.EmphasizedRuns;
    internal int DocumentLinks => DocumentPreview.SafeLinkCount;
    internal bool DocumentOmittedMedia => DocumentPreview.OmittedMedia;
    internal string DocumentText => DocumentPreview.RenderedText;
    internal bool HasImagePreview => ImagePreview.HasImage;
    internal long? PreviewedImageId => ImagePreview.DisplayedId;
    internal int ImagePixelWidth => ImagePreview.PixelWidth;
    internal bool CanCopy => CopyButton.IsEnabled;
    internal bool CanPaste => PasteButton.IsEnabled;
    internal bool TargetTrackingRunning => targetStatus?.Running == true;
    internal bool CanReveal => RevealButton.Visibility == Visibility.Visible;
    internal bool ExternalPending => externalPending;
    internal bool HasDialog => activeDialog is not null;
    internal bool IsDialogReady => activeDialog is { IsLoaded: true }
        && Descendants<Button>(activeDialog).Any(button => button.Name == "PrimaryButton" && button.IsLoaded)
        && Descendants<Button>(activeDialog).Any(button => button.Name == "CloseButton" && button.IsLoaded);
    internal string DialogText => dialogInput?.Text ?? "";
    internal bool HasError => Feedback.IsOpen && Feedback.Severity == InfoBarSeverity.Error;
    internal string FeedbackText => Feedback.IsOpen ? Feedback.Message : "";
    internal int RealizedCount => Enumerable.Range(0, rows.Count).Count(index => ActiveList.ContainerFromIndex(index) is not null);
    internal string ActiveTagFilter => exactTagFilter ?? TagFilter.Text.Trim();
    internal string SortOrder => OrderChoice.SelectedIndex switch { 1 => "time", 2 => "usage", _ => "default" };
    internal void ChooseSort(string sort)
    {
        if (!OrderChoice.IsEnabled) throw new InvalidOperationException("Record sort is disabled.");
        OrderChoice.SelectedIndex = sort switch { "time" => 1, "usage" => 2, _ => 0 };
    }
    internal void ShowTag(string name)
    {
        SearchText.Text = ""; TypeFilter.SelectedIndex = 0; PinnedOnly.IsChecked = TagOnly.IsChecked = false;
        TagFilter.Text = name; exactTagFilter = name;
        if (OrderChoice.SelectedIndex == 0) OrderChoice.SelectedIndex = 1;
        ScheduleSearch();
    }
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
        ActiveList.SelectedItems.Clear();
        foreach (long id in ids) ActiveList.SelectedItems.Add(rows.First(row => row.Id == id));
    }
    internal bool CanDelete => DeleteButton.IsEnabled;
    internal void ScrollToEnd() => listScroll?.ChangeView(null, listScroll.ScrollableHeight, null, true);
    internal void Invoke(string name)
    {
        var button = name switch
        {
            "capture" => (ButtonBase)CaptureButton, "add" => AddButton,
            "paste" => PasteButton, "paste-plain" => PastePlainButton, "target-start" => StartTargetButton,
            "preview" => PreviewButton, "open" => OpenButton, "copy" => CopyButton, "copy-plain" => CopyPlainButton, "edit" => EditButton, "pin" => PinButton,
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
