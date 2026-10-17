using System.Collections.ObjectModel;
using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TieZ.Core;

namespace TieZ.Native;

public sealed record PickedTagName(string Name)
{
    public string Label => JsonSerializer.Serialize(Name, new JsonSerializerOptions { Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping });
}
public sealed class PickerTagRow(TagInfo info)
{
    internal TagInfo Info => info;
    public string Label => new PickedTagName(info.Name).Label;
    public string Details => new TagRow(info).Details;
    public SolidColorBrush Swatch => new TagRow(info).Swatch;
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
            { args.Cancel = true; Message("标签名称请保持在一行。"); }
        };
        UpdateCommands();
    }
    internal async Task InitializeAsync(NativeCore sharedCore, long id)
    {
        if (core is not null) throw new InvalidOperationException("Tag picker already initialized.");
        core = sharedCore; entryId = id; await RereadAsync();
    }
    internal void ObserveRevision(ulong current)
    {
        if (!disposed && revision is { } accepted && current > accepted && !busy)
        { stale = true; Message("历史或标签目录已更新。选择已保留，请重新读取后再保存。"); UpdateCommands(); }
    }
    private void Message(string message)
    {
        Feedback.Text = message; Feedback.Visibility = Visibility.Visible;
    }
    private void Failure(Exception error)
    {
        if (error is CoreException { Code: "history_conflict" or "not_found" }) stale = true;
        Message(error.Message + (stale ? " 选择已保留，请重新读取。" : " 选择已保留，可重试。"));
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
        { Message("请输入单行标签名称。"); return; }
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
        PageStatus.Text = $"已载入 {rows.Count} 个标签";
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
            AcceptPage(page, true); Feedback.Text = ""; Feedback.Visibility = Visibility.Collapsed;
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
        { Message("请先将新名称加入选择，或清空输入后保存。"); return false; }
        var names = Selection; busy = true; UpdateCommands();
        bool requestFailed = false;
        try
        {
            bool saved = await mutate(async () =>
            {
                try { return await core.SetExactTagsAsync(entryId, names, expected, lifetime.Token); }
                catch (Exception error) { requestFailed = true; if (!disposed) Failure(error); throw; }
            });
            if (!saved && !disposed && !requestFailed) Message("保存未完成，选择已保留。请稍后重试或重新读取。");
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
        if (disposed) return; disposed = true; lifetime.Cancel(); core = null; revision = null;
        after = null; search = ""; entryId = 0; hasMore = false;
        binding = true; rows.Clear(); selected.Clear(); SearchText.Text = NewName.Text = Feedback.Text = PageStatus.Text = "";
        StateChanged = null; lifetime.Dispose();
    }
}
