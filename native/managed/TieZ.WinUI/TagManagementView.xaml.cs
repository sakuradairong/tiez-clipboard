using System.Collections.ObjectModel;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;
using Microsoft.UI.Xaml.Media;
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
    private ContentDialog? activeDialog;
    private Func<TagStrings, string>? feedbackRender;
    private Action<TagStrings>? dialogChrome;
    private bool renameDirty;
    private ulong? revision;
    private string? after, acceptedColor, draftColor;
    private string sortOrder = "count", searchTerm = "";
    private Color? knownPickerColor;
    private int generation;
    private bool active, disposed, loading, saving, polling, binding, dirty, stale, hasMore, refreshOnActivate;
    internal event Action<string>? FilterRequested;
    internal bool IsBusy => loading || saving;
    internal bool IsActive => active;
    internal bool IsContentReady => IsLoaded && XamlRoot is not null;
    internal bool HasDraft => dirty || renameDirty;
    internal bool HasDialog => activeDialog is not null;
    internal string DialogText => (activeDialog?.Content as TextBlock)?.Text ?? "";
    internal string RenameDraft => RenameName.Text;
    internal bool HasConflict => stale;
    internal bool HasError => Feedback.IsOpen && Feedback.Severity == InfoBarSeverity.Error;
    internal bool CanSave => SaveColorButton.IsEnabled;
    internal bool HasInvalidColor => dirty && draftColor is not null && !NativeTagColor.TryParse(draftColor, out _);
    internal int Count => rows.Count;
    internal string SortOrder => sortOrder;
    internal string AcceptedSearch => searchTerm;
    internal string SearchDraft { get => TagSearch.Text; set => TagSearch.Text = value; }
    internal IReadOnlyList<TagInfo> DisplayedTags => rows.Select(row => row.Info).ToArray();
    internal bool HasMore => hasMore;
    internal ulong? DisplayedRevision => revision;
    internal string? SelectedTag => selected?.Info.Name;
    internal string ColorDraft => ColorText.Text;
    internal string NewNameDraft { get => NewTagName.Text; set => NewTagName.Text = value; }
    internal ElementTheme CurrentTheme => ActualTheme;
    internal int RealizedCount => Enumerable.Range(0, rows.Count).Count(index => TagList.ContainerFromIndex(index) is not null);
    internal bool HasRetainedData => core is not null || rows.Count > 0 || selected is not null || revision is not null
        || acceptedColor is not null || draftColor is not null || ColorText.Text.Length > 0 || NewTagName.Text.Length > 0
        || activeDialog is not null || RenameName.Text.Length > 0 || searchTerm.Length > 0 || TagSearch.Text.Length > 0;

    public TagManagementView()
    {
        InitializeComponent(); TagList.ItemsSource = rows;
        TagList.SelectionChanged += (_, _) =>
        {
            if (binding || disposed || ReferenceEquals(selected, TagList.SelectedItem)) return;
            if (HasDraft && !ReferenceEquals(selected, TagList.SelectedItem))
            {
                binding = true; TagList.SelectedItem = selected; binding = false;
                Message(text => text.DraftBlocked, InfoBarSeverity.Warning); return;
            }
            LoadSelected(TagList.SelectedItem as TagRow);
        };
        SortChoice.SelectionChanged += async (_, _) => await ChangeSortAsync();
        TagSearch.TextChanging += (_, _) => { if (!binding) UpdateCommands(); };
        SearchButton.Click += async (_, _) => await ApplySearchAsync();
        ClearSearchButton.Click += async (_, _) => await ApplySearchAsync("");
        TagSearch.KeyDown += async (_, args) =>
        {
            if (args.Key == Windows.System.VirtualKey.Enter && SearchButton.IsEnabled)
            { args.Handled = true; await ApplySearchAsync(); }
        };
        NewTagName.TextChanging += (_, _) => UpdateCommands();
        NewTagName.BeforeTextChanging += (_, args) =>
        {
            if (!binding && args.NewText.IndexOfAny(['\r', '\n']) >= 0)
            { args.Cancel = true; Message(text => text.SingleLineName, InfoBarSeverity.Warning); }
        };
        RenameName.BeforeTextChanging += (_, args) =>
        {
            if (!binding && args.NewText.IndexOfAny(['\r', '\n']) >= 0)
            { args.Cancel = true; Message(text => text.SingleLineName, InfoBarSeverity.Warning); }
        };
        RenameName.TextChanging += (_, _) =>
        {
            if (binding || disposed) return;
            renameDirty = selected is not null && RenameName.Text != selected.Info.Name && RenameName.Text.Trim() != selected.Info.Name;
            if (!stale) Feedback.IsOpen = false; UpdateCommands();
        };
        RenameButton.Click += async (_, _) => await RenameAsync();
        DeleteButton.Click += async (_, _) => await DeleteAsync();
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
        FilterButton.Click += (_, _) => { if (!IsBusy && !HasDraft && !stale && selected is not null) FilterRequested?.Invoke(selected.Info.Name); };
        LoadSelected(null); UpdateCommands();
        LanguageService.Changed += ApplyLanguage; ApplyLanguage();
    }
    /// <summary>
    /// Re-derives every localized string of this surface from the shared language. It never talks to the core:
    /// selection, paging, search, sort, drafts, an open confirmation and the busy state stay exactly where they were.
    /// </summary>
    internal void ApplyLanguage()
    {
        if (disposed) return;
        var text = TagStrings.Current;
        var common = CommonStrings.Current;
        TitleText.Text = text.Title;
        NewTagName.Header = text.NewTagHeader; NewTagName.PlaceholderText = text.NewTagPlaceholder;
        CreateButton.Content = text.CreateTag; ReloadButton.Content = common.Refresh;
        TagSearch.PlaceholderText = text.SearchPlaceholder;
        AutomationProperties.SetName(TagSearch, text.SearchName);
        SearchButton.Content = common.Search; ClearSearchButton.Content = text.ClearSearch;
        SortChoice.Header = text.SortHeader;
        if (SortChoice.Items.Count == 2 && SortChoice.Items[0] is ComboBoxItem countItem && SortChoice.Items[1] is ComboBoxItem nameItem)
        { countItem.Content = text.SortByCount; nameItem.Content = text.SortByName; }
        AutomationProperties.SetName(TagList, text.ListName);
        MoreButton.Content = text.MoreTags;
        SelectedName.Text = selected?.Label ?? text.NoTagSelected;
        SelectedCounts.Text = selected?.Details ?? "";
        FilterButton.Content = text.FilterByTag;
        DeleteButton.Content = text.DeleteTag; DeleteHint.Text = text.DeleteTagHint;
        RenameName.Header = text.RenameHeader; RenameButton.Content = text.RenameButton; RenameHint.Text = text.RenameHint;
        ColorText.Header = text.ColorHeader; ColorText.PlaceholderText = text.ColorPlaceholder;
        ClearColorButton.Content = text.UseDefaultColor; SaveColorButton.Content = text.SaveColor;
        EditorHint.Text = text.EditorHint;
        // Rows and their containers stay; only the labels derived from the table are re-raised.
        foreach (var row in rows) row.RefreshText();
        DescribeColor();
        UpdateCommands();
        if (Feedback.IsOpen && feedbackRender is not null) Feedback.Message = feedbackRender(text);
        dialogChrome?.Invoke(text);
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
            activeDialog?.Hide(); poll?.Stop(); generation++; readCancellation?.Cancel(); loading = false; UpdateCommands(); return;
        }
        poll?.Start(); UpdateCommands();
        if (revision is null || refreshOnActivate) _ = LoadAsync(reset: true);
    }
    private void LoadSelected(TagRow? row)
    {
        selected = row; acceptedColor = draftColor = row?.Info.Color; dirty = renameDirty = false;
        binding = true;
        try
        {
            SelectedName.Text = row?.Label ?? TagStrings.Current.NoTagSelected; SelectedCounts.Text = row?.Details ?? "";
            RenameName.Text = row?.Info.Name ?? ""; ColorText.Text = acceptedColor ?? "";
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
    private void DescribeColor()
    {
        var text = TagStrings.Current;
        ColorHint.Text = draftColor is null ? text.ColorDefaultHint
            : NativeTagColor.TryParse(draftColor, out _) ? text.ColorAlphaHint
            : dirty ? text.ColorInvalidHint : text.ColorUnsupportedHint;
    }
    private void UpdateCommands()
    {
        if (disposed) return;
        bool ready = active && !IsBusy && activeDialog is null && revision is not null;
        SortChoice.IsEnabled = TagSearch.IsEnabled = ready && !HasDraft && !stale;
        SearchButton.IsEnabled = ready && !HasDraft && !stale && TagSearch.Text != searchTerm;
        ClearSearchButton.IsEnabled = ready && !HasDraft && !stale && (TagSearch.Text.Length > 0 || searchTerm.Length > 0);
        CreateButton.IsEnabled = ready && !HasDraft && !stale && !string.IsNullOrWhiteSpace(NewTagName.Text);
        ReloadButton.IsEnabled = active && !IsBusy && activeDialog is null;
        MoreButton.IsEnabled = ready && hasMore && !HasDraft && !stale;
        MoreButton.Visibility = hasMore ? Visibility.Visible : Visibility.Collapsed;
        TagList.IsEnabled = active && !IsBusy && activeDialog is null;
        NewTagName.IsEnabled = active && !IsBusy && activeDialog is null;
        RenameName.IsEnabled = ready && selected is not null && !dirty;
        RenameButton.IsEnabled = ready && selected is not null && renameDirty && !dirty && !stale && !string.IsNullOrWhiteSpace(RenameName.Text);
        ColorText.IsEnabled = Picker.IsEnabled = ready && selected is not null && !renameDirty;
        ClearColorButton.IsEnabled = ready && selected is not null && !renameDirty && draftColor is not null;
        SaveColorButton.IsEnabled = ready && selected is not null && dirty && !renameDirty && !stale && !HasInvalidColor;
        FilterButton.IsEnabled = ready && selected is not null && !HasDraft && !stale;
        DeleteButton.IsEnabled = ready && selected is not null && !HasDraft && !stale;
        var text = TagStrings.Current;
        Status.Text = loading ? text.StatusLoading : saving ? text.StatusSaving : string.Format(text.StatusLoaded, rows.Count)
            + (hasMore ? text.Separator + text.StatusMore : "")
            + (searchTerm.Length > 0 ? text.Separator + string.Format(text.StatusSearch, searchTerm) : "");
    }
    private void Message(Func<TagStrings, string> message, InfoBarSeverity severity)
    {
        if (disposed) return;
        feedbackRender = message; Feedback.Message = message(TagStrings.Current); Feedback.Severity = severity; Feedback.IsOpen = true;
    }
    private async Task ChangeSortAsync()
    {
        if (binding || disposed) return;
        string requested = SortChoice.SelectedIndex == 1 ? "name" : "count";
        if (requested == sortOrder) return;
        bool changed = active && !HasDraft && !stale && !IsBusy && activeDialog is null
            && await LoadAsync(reset: true, order: requested);
        if (!changed && !disposed)
        {
            binding = true; SortChoice.SelectedIndex = sortOrder == "name" ? 1 : 0; binding = false;
        }
    }
    private async Task ApplySearchAsync(string? requested = null)
    {
        if (disposed || !active || core is null || HasDraft || stale || IsBusy || activeDialog is not null) return;
        string submitted = requested ?? TagSearch.Text;
        if (submitted == searchTerm) { TagSearch.Text = submitted; UpdateCommands(); return; }
        await LoadAsync(reset: true, search: submitted);
    }
    private async Task<bool> LoadAsync(bool reset, string? preferred = null, string? order = null, string? search = null)
    {
        if (disposed || !active || core is null || IsBusy || activeDialog is not null || (!reset && (!hasMore || HasDraft || stale))) return false;
        string requestedOrder = order ?? sortOrder;
        string requestedSearch = search ?? searchTerm;
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
                page = await core.GetTagsAsync(new TagQuery(PageSize, cursor, expected, requestedOrder, requestedSearch), cancellation.Token);
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
                if (search is not null) TagSearch.Text = requestedSearch;
            }
            finally { binding = false; }
            sortOrder = requestedOrder; searchTerm = requestedSearch;
            revision = page.Revision; after = page.NextAfter; hasMore = page.HasMore; stale = false; refreshOnActivate = false;
            LoadSelected(TagList.SelectedItem as TagRow); Feedback.IsOpen = false; return true;
        }
        catch (OperationCanceledException) { return false; }
        catch (Exception error)
        {
            if (!disposed && current == generation)
            {
                if (error is CoreException { Code: "history_conflict" }) stale = true;
                Message(text => string.Format(text.ReadFailed, error.Message), InfoBarSeverity.Error);
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
            if (!disposed) Message(text => reloaded ? (result.Changed ? text.TagSaved : text.TagUnchanged) : text.SaveUnrefreshed,
                reloaded ? InfoBarSeverity.Success : InfoBarSeverity.Warning);
        }
        catch (OperationCanceledException) { }
        catch (Exception error)
        {
            if (!disposed)
            {
                if (error is CoreException { Code: "history_conflict" }) stale = true;
                Message(text => string.Format(text.SaveFailed, error.Message), InfoBarSeverity.Error);
            }
        }
        finally { saving = false; if (!disposed) UpdateCommands(); }
    }
    private async Task RenameAsync()
    {
        if (disposed || core is null || !RenameButton.IsEnabled || selected is null || revision is not { } expected || activeDialog is not null) return;
        string oldName = selected.Info.Name, newName = RenameName.Text.Trim();
        long affected = selected.Info.Count;
        var body = new TextBlock { TextWrapping = TextWrapping.Wrap };
        var dialog = new ContentDialog { XamlRoot = XamlRoot, Content = body, DefaultButton = ContentDialogButton.Close };
        void Render(TagStrings text)
        {
            dialog.Title = text.RenameTitle;
            body.Text = string.Format(text.RenameBody, oldName, newName, affected);
            dialog.PrimaryButtonText = text.RenameConfirm;
            dialog.CloseButtonText = CommonStrings.Current.Cancel;
        }
        Render(TagStrings.Current); dialogChrome = Render;
        activeDialog = dialog; UpdateCommands();
        try
        {
            var choice = await dialog.ShowAsync(); activeDialog = null;
            if (disposed || !active || choice != ContentDialogResult.Primary) return;
            saving = true; UpdateCommands();
            var result = await core.RenameTagAsync(oldName, newName, expected, lifetime.Token);
            if (disposed) return;
            revision = result.Revision; renameDirty = false;
            binding = true; RenameName.Text = newName; binding = false;
            refreshOnActivate = true; saving = false;
            bool reloaded = active && await LoadAsync(reset: true, preferred: newName);
            if (!reloaded) stale = true;
            if (!disposed) Message(text => reloaded ? text.Renamed : text.RenameUnrefreshed,
                reloaded ? InfoBarSeverity.Success : InfoBarSeverity.Warning);
        }
        catch (OperationCanceledException) { }
        catch (Exception error)
        {
            if (!disposed)
            {
                if (error is CoreException { Code: "history_conflict" or "not_found" }) stale = true;
                Message(text => string.Format(text.RenameFailed, error.Message), InfoBarSeverity.Error);
            }
        }
        finally
        {
            if (ReferenceEquals(activeDialog, dialog)) activeDialog = null;
            if (activeDialog is null) dialogChrome = null;
            saving = false; if (!disposed) UpdateCommands();
        }
    }
    private async Task DeleteAsync()
    {
        if (disposed || core is null || !DeleteButton.IsEnabled || selected is null || activeDialog is not null) return;
        string name = selected.Info.Name;
        ContentDialog? dialog = null;
        saving = true; UpdateCommands();
        try
        {
            var preview = await core.GetTagDeletePreviewAsync(name, lifetime.Token);
            if (disposed || !active) return;
            var body = new TextBlock { TextWrapping = TextWrapping.Wrap };
            var created = new ContentDialog { XamlRoot = XamlRoot, Content = body, DefaultButton = ContentDialogButton.Close };
            dialog = created;
            void Render(TagStrings text)
            {
                created.Title = text.DeleteTitle;
                body.Text = string.Format(text.DeleteBody, preview.Name, preview.Count, preview.StoredCount, preview.SessionCount);
                created.PrimaryButtonText = text.DeleteConfirm;
                created.CloseButtonText = CommonStrings.Current.Cancel;
            }
            Render(TagStrings.Current); dialogChrome = Render;
            activeDialog = created; saving = false; UpdateCommands();
            var choice = await created.ShowAsync(); activeDialog = null;
            if (disposed || !active || choice != ContentDialogResult.Primary) return;
            saving = true; UpdateCommands();
            var result = await core.DeleteTagAsync(preview.Name, preview.Revision, lifetime.Token);
            if (disposed) return;
            revision = result.Revision; refreshOnActivate = true; saving = false;
            bool reloaded = active && await LoadAsync(reset: true);
            if (!reloaded) stale = true;
            if (!disposed) Message(text => string.Format(text.Deleted, result.RemovedCount, result.StoredCount, result.SessionCount)
                + (reloaded ? "" : text.DeleteUnrefreshed)
                + (result.CleanupError is null ? "" : text.CleanupIncomplete),
                reloaded && result.CleanupError is null ? InfoBarSeverity.Success : InfoBarSeverity.Warning);
        }
        catch (OperationCanceledException) { }
        catch (Exception error)
        {
            if (!disposed)
            {
                if (error is CoreException { Code: "history_conflict" or "not_found" }) stale = true;
                Message(text => string.Format(text.DeleteFailed, error.Message), InfoBarSeverity.Error);
            }
        }
        finally
        {
            if (ReferenceEquals(activeDialog, dialog)) activeDialog = null;
            if (activeDialog is null) dialogChrome = null;
            saving = false; if (!disposed) UpdateCommands();
        }
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
                stale = true; Message(text => text.StaleNotice, InfoBarSeverity.Warning); UpdateCommands();
            }
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { if (!disposed && active) Message(text => string.Format(text.PollFailed, error.Message), InfoBarSeverity.Warning); }
        finally { polling = false; }
    }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; active = false; dirty = renameDirty = stale = hasMore = loading = saving = false; generation++; lifetime.Cancel(); readCancellation?.Cancel();
        LanguageService.Changed -= ApplyLanguage; feedbackRender = null; dialogChrome = null;
        activeDialog?.Hide(); activeDialog = null;
        poll?.Stop(); if (poll is not null) poll.Tick -= OnPoll;
        readCancellation?.Dispose(); readCancellation = null; lifetime.Dispose();
        binding = true; sortOrder = "count"; searchTerm = ""; TagSearch.Text = ""; SortChoice.SelectedIndex = 0;
        TagList.SelectedItem = null; rows.Clear(); selected = null; acceptedColor = draftColor = after = null; revision = null; core = null;
        NewTagName.Text = ColorText.Text = RenameName.Text = SelectedName.Text = SelectedCounts.Text = Status.Text = "";
        Picker.Color = Colors.Transparent; knownPickerColor = null; Feedback.IsOpen = false; Feedback.Message = ""; FilterRequested = null;
    }
    internal void Select(string name) => TagList.SelectedItem = rows.FirstOrDefault(row => row.Info.Name == name);
    internal void ChooseSort(string value) => SortChoice.SelectedIndex = value == "name" ? 1 : 0;
    internal void EditName(string value) => RenameName.Text = value;
    internal void InvokeDialog(bool confirm)
    {
        if (activeDialog is null) throw new InvalidOperationException("No tag confirmation.");
        var button = Descendants<Button>(activeDialog).First(button => button.Name == (confirm ? "PrimaryButton" : "CloseButton"));
        ((IInvokeProvider)FrameworkElementAutomationPeer.CreatePeerForElement(button).GetPattern(PatternInterface.Invoke)).Invoke();
    }
    private static IEnumerable<T> Descendants<T>(DependencyObject parent) where T : DependencyObject
    {
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
        {
            var child = VisualTreeHelper.GetChild(parent, i);
            if (child is T match) yield return match;
            foreach (var descendant in Descendants<T>(child)) yield return descendant;
        }
    }
    internal void EditColor(string value) => ColorText.Text = value;
    internal void ChooseColor(Color value) => Picker.Color = value;
    internal void Invoke(string command)
    {
        var button = command switch { "search" => SearchButton, "clear-search" => ClearSearchButton, "delete" => DeleteButton, "rename" => RenameButton, "create" => CreateButton, "save" => SaveColorButton, "clear" => ClearColorButton,
            "reload" => ReloadButton, "more" => MoreButton, "filter" => FilterButton, _ => throw new ArgumentException(nameof(command)) };
        if (button.IsEnabled) ((IInvokeProvider)new ButtonAutomationPeer(button).GetPattern(PatternInterface.Invoke)).Invoke();
    }
}
