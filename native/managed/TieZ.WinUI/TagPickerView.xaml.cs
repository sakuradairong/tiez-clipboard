using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TieZ.Core;

namespace TieZ.Native;

public sealed record PickedTagName(string Name)
{
    public string Label => JsonSerializer.Serialize(Name, new JsonSerializerOptions { Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping });
}
public sealed class PickerTagRow(TagInfo info) : INotifyPropertyChanged
{
    private readonly TagRow row = new(info);
    internal TagInfo Info => info;
    public event PropertyChangedEventHandler? PropertyChanged;
    public string Label => new PickedTagName(info.Name).Label;
    public string Details => row.Details;
    public SolidColorBrush Swatch => row.Swatch;

    /// <summary>Re-derives the localized row caption in place so a live language switch keeps the container.</summary>
    internal void RefreshText() => PropertyChanged?.Invoke(this, new(nameof(Details)));
}

// Borrows one core. Search, selection and new-name drafts do not write to it.
public sealed partial class TagPickerView : UserControl, IDisposable
{
    private readonly ObservableCollection<PickerTagRow> rows = [];
    private readonly ObservableCollection<PickedTagName> selected = [];
    private readonly CancellationTokenSource lifetime = new();
    private NativeCore? core;
    private long entryId;
    private ulong? revision;
    private string? after;
    private string search = "";
    private bool hasMore, busy, binding, stale, disposed;
    private Func<TagStrings, string>? feedbackRender;
    internal event Action? StateChanged;
    internal bool CanSave => !disposed && !busy && !stale && revision is not null;
    internal bool IsBusy => busy;
    internal bool IsStale => stale;
    internal bool HasError => Feedback.Visibility == Visibility.Visible;
    internal IReadOnlyList<string> Selection => selected.Select(value => value.Name).ToArray();
    internal IReadOnlyList<TagInfo> Catalog => rows.Select(value => value.Info).ToArray();
    internal bool HasMore => hasMore;
    internal ulong? Revision => revision;
    internal string SearchDraft { get => SearchText.Text; set => SearchText.Text = value; }
    internal string NameDraft { get => NewName.Text; set => NewName.Text = value; }
    internal bool HasRetainedData => core is not null || rows.Count != 0 || selected.Count != 0 || revision is not null
        || SearchText.Text.Length > 0 || NewName.Text.Length > 0 || Feedback.Text.Length > 0 || after is not null || search.Length > 0;
    internal int RealizedCount => Enumerable.Range(0, rows.Count).Count(i => Choices.ContainerFromIndex(i) is not null);

    public TagPickerView()
    {
        InitializeComponent(); Choices.ItemsSource = rows; SelectedNames.ItemsSource = selected;
        Choices.SelectionChanged += (_, args) =>
        {
            if (binding || disposed || busy) return;
            foreach (PickerTagRow row in args.RemovedItems) Remove(row.Info.Name);
            foreach (PickerTagRow row in args.AddedItems) Add(row.Info.Name);
            UpdateCommands();
        };
        SelectedNames.SelectionChanged += (_, _) => UpdateCommands();
        SearchButton.Click += async (_, _) => await LoadPageAsync(reset: true);
        MoreButton.Click += async (_, _) => await LoadPageAsync(reset: false);
        ReloadButton.Click += async (_, _) => await RereadAsync();
        AddButton.Click += (_, _) => AddNewName();
        RemoveButton.Click += (_, _) =>
        {
            if (SelectedNames.SelectedItem is PickedTagName name) { Remove(name.Name); BindChoices(); UpdateCommands(); }
        };
        NewName.BeforeTextChanging += (_, args) =>
        {
            if (!disposed && args.NewText.IndexOfAny(['\r', '\n']) >= 0)
            { args.Cancel = true; Message(text => text.SingleLineName); }
        };
        UpdateCommands();
        LanguageService.Changed += ApplyLanguage; ApplyLanguage();
    }
    /// <summary>
    /// Re-derives the localized chrome of the picker from the shared language. It never talks to the core:
    /// the search text, the new-name draft, the selection and the busy state stay exactly where they were.
    /// </summary>
    internal void ApplyLanguage()
    {
        if (disposed) return;
        var text = TagStrings.Current;
        var common = CommonStrings.Current;
        IntroText.Text = text.PickerIntro;
        SearchText.PlaceholderText = text.SearchPlaceholder;
        AutomationProperties.SetName(SearchText, text.PickerSearchName);
        SearchButton.Content = common.Search;
        AutomationProperties.SetName(Choices, text.PickerChoicesName);
        MoreButton.Content = text.PickerMore;
        if (rows.Count > 0 || PageStatus.Text.Length > 0) PageStatus.Text = string.Format(text.StatusLoaded, rows.Count);
        SelectedNames.Header = text.PickerSelectedHeader;
        AutomationProperties.SetName(SelectedNames, text.PickerSelectedName);
        RemoveButton.Content = text.PickerRemove;
        NewName.PlaceholderText = text.PickerNewNamePlaceholder;
        AutomationProperties.SetName(NewName, text.PickerNewNamePlaceholder);
        AddButton.Content = text.PickerAdd;
        SensitiveHint.Text = text.PickerSensitiveHint;
        ReloadButton.Content = text.PickerReload;
        // Row containers stay; only the captions derived from the table are re-raised.
        foreach (var row in rows) row.RefreshText();
        if (Feedback.Visibility == Visibility.Visible && feedbackRender is not null) Feedback.Text = feedbackRender(text);
    }
    internal async Task InitializeAsync(NativeCore sharedCore, long id)
    {
        if (core is not null) throw new InvalidOperationException("Tag picker already initialized.");
        core = sharedCore; entryId = id; await RereadAsync();
    }
    internal void ObserveRevision(ulong current)
    {
        if (!disposed && revision is { } accepted && current > accepted && !busy)
        { stale = true; Message(text => text.PickerStale); UpdateCommands(); }
    }
    private void Message(Func<TagStrings, string> message)
    {
        feedbackRender = message; Feedback.Text = message(TagStrings.Current); Feedback.Visibility = Visibility.Visible;
    }
    private void Failure(Exception error)
    {
        if (error is CoreException { Code: "history_conflict" or "not_found" }) stale = true;
        Message(text => error.Message + (stale ? text.PickerStaleSuffix : text.PickerRetrySuffix));
    }
    private void Add(string name)
    {
        if (!selected.Any(value => value.Name == name)) selected.Add(new(name));
    }
    private void Remove(string name)
    {
        var item = selected.FirstOrDefault(value => value.Name == name);
        if (item is not null) selected.Remove(item);
    }
    private void AddNewName()
    {
        if (disposed || busy || revision is null) return;
        string name = NewName.Text.Trim();
        if (name.Length == 0 || name.IndexOfAny(['\r', '\n']) >= 0)
        { Message(text => text.PickerNeedName); return; }
        Add(name); NewName.Text = ""; BindChoices(); UpdateCommands();
    }
    private void BindChoices()
    {
        binding = true;
        try
        {
            Choices.SelectedItems.Clear();
            foreach (var row in rows) if (selected.Any(value => value.Name == row.Info.Name)) Choices.SelectedItems.Add(row);
        }
        finally { binding = false; }
    }
    private void AcceptPage(TagPage page, bool reset)
    {
        binding = true;
        try
        {
            if (reset) rows.Clear();
            foreach (var tag in page.Items) rows.Add(new(tag));
        }
        finally { binding = false; }
        after = page.NextAfter; hasMore = page.HasMore; BindChoices();
        PageStatus.Text = string.Format(TagStrings.Current.StatusLoaded, rows.Count);
    }
    private async Task RereadAsync()
    {
        if (disposed || busy || core is null) return;
        busy = true; UpdateCommands();
        try
        {
            var snapshot = await core.GetTagSelectionAsync(entryId, lifetime.Token);
            if (disposed) return;
            string nextSearch = SearchText.Text;
            var page = await core.GetTagsAsync(new(ExpectedRevision: snapshot.Revision, Sort: "count", Search: nextSearch), lifetime.Token);
            if (disposed) return;
            // Only a complete, same-revision read may discard the draft.
            selected.Clear(); foreach (string name in snapshot.Tags) Add(name);
            revision = snapshot.Revision; stale = false; search = nextSearch; NewName.Text = "";
            AcceptPage(page, true); feedbackRender = null; Feedback.Text = ""; Feedback.Visibility = Visibility.Collapsed;
        }
        catch (OperationCanceledException) when (disposed) { }
        catch (Exception error) { if (!disposed) Failure(error); }
        finally { busy = false; if (!disposed) UpdateCommands(); }
    }
    private async Task LoadPageAsync(bool reset)
    {
        if (!CanSave || core is null || (!reset && !hasMore)) return;
        busy = true; UpdateCommands();
        try
        {
            string nextSearch = reset ? SearchText.Text : search;
            var page = await core.GetTagsAsync(new(After: reset ? null : after, ExpectedRevision: revision, Sort: "count", Search: nextSearch), lifetime.Token);
            if (disposed) return;
            search = nextSearch; AcceptPage(page, reset);
        }
        catch (OperationCanceledException) when (disposed) { }
        catch (Exception error) { if (!disposed) Failure(error); }
        finally { busy = false; if (!disposed) UpdateCommands(); }
    }
    internal async Task<bool> SaveAsync(Func<Func<Task<HistoryMutation>>, Task<bool>> mutate)
    {
        if (!CanSave || core is null || revision is not { } expected) return false;
        if (NewName.Text.Length > 0)
        { Message(text => text.PickerAddBeforeSave); return false; }
        var names = Selection; busy = true; UpdateCommands();
        bool requestFailed = false;
        try
        {
            bool saved = await mutate(async () =>
            {
                try { return await core.SetExactTagsAsync(entryId, names, expected, lifetime.Token); }
                catch (Exception error) { requestFailed = true; if (!disposed) Failure(error); throw; }
            });
            if (!saved && !disposed && !requestFailed) Message(text => text.PickerSaveIncomplete);
            return saved;
        }
        finally { busy = false; if (!disposed) UpdateCommands(); }
    }
    private void UpdateCommands()
    {
        if (disposed) return;
        bool ready = !busy && revision is not null;
        SearchButton.IsEnabled = CanSave; MoreButton.IsEnabled = CanSave && hasMore;
        ReloadButton.IsEnabled = !busy; SearchText.IsEnabled = Choices.IsEnabled = !busy;
        NewName.IsEnabled = AddButton.IsEnabled = ready;
        RemoveButton.IsEnabled = ready && SelectedNames.SelectedItem is not null;
        StateChanged?.Invoke();
    }
    internal void Invoke(string command)
    {
        var button = command switch { "search" => SearchButton, "more" => MoreButton, "reload" => ReloadButton,
            "add" => AddButton, "remove" => RemoveButton, _ => throw new ArgumentException(command) };
        var peer = new Microsoft.UI.Xaml.Automation.Peers.ButtonAutomationPeer(button);
        ((Microsoft.UI.Xaml.Automation.Provider.IInvokeProvider)peer.GetPattern(Microsoft.UI.Xaml.Automation.Peers.PatternInterface.Invoke)).Invoke();
    }
    internal void Choose(string name, bool value)
    {
        var row = rows.Single(row => row.Info.Name == name);
        if (value) Choices.SelectedItems.Add(row); else Choices.SelectedItems.Remove(row);
    }
    internal void SelectForRemoval(string name) => SelectedNames.SelectedItem = selected.Single(value => value.Name == name);
    public void Dispose()
    {
        if (disposed) return; disposed = true; lifetime.Cancel(); LanguageService.Changed -= ApplyLanguage; feedbackRender = null; core = null; revision = null;
        after = null; search = ""; entryId = 0; hasMore = false;
        binding = true; rows.Clear(); selected.Clear(); SearchText.Text = NewName.Text = Feedback.Text = PageStatus.Text = "";
        StateChanged = null; lifetime.Dispose();
    }
}
