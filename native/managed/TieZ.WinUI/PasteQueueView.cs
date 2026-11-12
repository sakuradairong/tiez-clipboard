using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;

namespace TieZ.Native;

internal sealed class PasteQueueView : StackPanel, IDisposable
{
    private readonly NativeCore core;
    private readonly CancellationTokenSource lifetime = new();
    private readonly DispatcherQueueTimer poll;
    private readonly TextBlock hint = new() { TextWrapping = TextWrapping.Wrap };
    private readonly TextBlock count = new();
    private readonly ListView list = new() { Height = 240, SelectionMode = ListViewSelectionMode.Single };
    private readonly Button up = new(), down = new(), remove = new(), clear = new(), refresh = new(), next = new();
    private readonly CheckBox consent = new() { Visibility = Visibility.Collapsed };
    private readonly InfoBar feedback = new() { IsOpen = false };
    private PasteQueueSnapshot? snapshot;
    private Func<string>? feedbackText;
    private bool disposed, busy, reading;
    internal bool IsBusy => busy || reading;
    internal IReadOnlyList<long> Ids => snapshot?.Ids ?? [];
    internal bool HasError => feedback.IsOpen && feedback.Severity == InfoBarSeverity.Error;
    internal bool Disposed => disposed;
    internal string Diagnostic => $"disposed={disposed}, busy={busy}, reading={reading}, loaded={IsLoaded}, count={Ids.Count}, error={feedback.Message}";
    internal ListView QueueList => list;
    internal Button UpButton => up;
    internal Button RemoveButton => remove;
    internal Button ClearButton => clear;
    internal Button NextButton => next;

    internal PasteQueueView(NativeCore core)
    {
        this.core = core; Spacing = 12; MinWidth = 300; MaxWidth = 540;
        Children.Add(hint); Children.Add(count); Children.Add(list);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        foreach (var button in new[] { up, down, remove, clear }) actions.Children.Add(button);
        Children.Add(actions); Children.Add(consent);
        var footer = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        footer.Children.Add(refresh); footer.Children.Add(next); Children.Add(footer); Children.Add(feedback);
        list.SelectionChanged += (_, _) => Commands();
        up.Click += async (_, _) => await ChangeAsync(-1, false);
        down.Click += async (_, _) => await ChangeAsync(1, false);
        remove.Click += async (_, _) => await ChangeAsync(0, true);
        clear.Click += async (_, _) => await ReplaceAsync([]);
        refresh.Click += async (_, _) => await ReloadAsync();
        next.Click += async (_, _) => await PasteAsync();
        poll = DispatcherQueue.CreateTimer(); poll.Interval = TimeSpan.FromMilliseconds(750);
        poll.Tick += Poll;
        Loaded += async (_, _) => { if (!disposed) { poll.Start(); await ReloadAsync(); } };
        // ContentDialog can detach and reattach its content while applying templates.
        // The owning dialog closes this borrowed view; Unloaded is not a close signal.
        LanguageService.Changed += ApplyLanguage;
        ApplyLanguage(); Commands();
    }
    private void ApplyLanguage()
    {
        if (disposed) return;
        hint.Text = QueueStrings.Hint; up.Content = QueueStrings.Up; down.Content = QueueStrings.Down;
        remove.Content = QueueStrings.Remove; clear.Content = QueueStrings.Clear;
        refresh.Content = CommonStrings.Current.Refresh; next.Content = QueueStrings.Next;
        consent.Content = QueueStrings.Sensitive;
        if (feedbackText is not null) feedback.Message = feedbackText();
        AutomationProperties.SetName(list, QueueStrings.Title);
        Render();
    }
    private void Render()
    {
        if (disposed) return;
        int index = list.SelectedIndex;
        list.ItemsSource = (snapshot?.Ids ?? []).Select((id, i) => QueueStrings.Row(i + 1, id)).ToArray();
        list.SelectedIndex = index < list.Items.Count ? index : -1;
        count.Text = QueueStrings.Count(snapshot?.Ids.Count ?? 0);
        Commands();
    }
    private void Commands()
    {
        if (disposed) return;
        bool ready = snapshot is not null && !busy && !reading;
        bool usable = ready && snapshot!.RequiresReset == false;
        up.IsEnabled = usable && list.SelectedIndex > 0;
        down.IsEnabled = usable && list.SelectedIndex >= 0 && list.SelectedIndex < list.Items.Count - 1;
        remove.IsEnabled = usable && list.SelectedIndex >= 0;
        clear.IsEnabled = ready && (snapshot!.Ids.Count > 0 || snapshot.RequiresReset);
        next.IsEnabled = usable && snapshot!.Ids.Count > 0;
        refresh.IsEnabled = !busy && !reading;
        list.IsEnabled = !busy;
    }
    private async void Poll(DispatcherQueueTimer sender, object args) => await ReloadAsync();
    internal async Task ReloadAsync()
    {
        if (disposed || busy || reading) return;
        reading = true; Commands();
        try
        {
            var value = await core.GetPasteQueueAsync(lifetime.Token);
            if (disposed) return;
            bool changed = snapshot is null || value.Revision != snapshot.Revision || value.HistoryRevision != snapshot.HistoryRevision;
            snapshot = value;
            if (changed) { consent.IsChecked = false; consent.Visibility = Visibility.Collapsed; Render(); }
            if (value.RequiresReset) Error(() => QueueStrings.ResetRequired);
            else if (value.LastError == "queue_full") Error(() => QueueStrings.Full);
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { if (!disposed) Error(error.Message); }
        finally { reading = false; Commands(); }
    }
    private async Task ChangeAsync(int direction, bool deleting)
    {
        if (snapshot is null || busy || reading || snapshot.RequiresReset || list.SelectedIndex < 0) return;
        int index = list.SelectedIndex;
        var ids = snapshot.Ids.ToList();
        if (deleting) ids.RemoveAt(index);
        else { int target = index + direction; if (target < 0 || target >= ids.Count) return; (ids[index], ids[target]) = (ids[target], ids[index]); }
        await ReplaceAsync(ids);
        if (!disposed) list.SelectedIndex = Math.Min(Math.Max(0, index + (deleting ? 0 : direction)), list.Items.Count - 1);
    }
    private async Task ReplaceAsync(IReadOnlyList<long> ids)
    {
        if (disposed || busy || reading || snapshot is null) return;
        busy = true; Commands();
        try
        {
            var accepted = await core.ReplacePasteQueueAsync(ids, snapshot.Revision, snapshot.HistoryRevision, lifetime.Token);
            if (disposed) return;
            snapshot = accepted; consent.IsChecked = false; consent.Visibility = Visibility.Collapsed;
            feedback.IsOpen = false; Render();
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { if (!disposed) Error(error.Message); }
        finally { busy = false; Commands(); }
    }
    private async Task PasteAsync()
    {
        if (disposed || busy || reading || snapshot is null || snapshot.RequiresReset || snapshot.Ids.Count == 0) return;
        var accepted = snapshot; bool allowed = consent.IsChecked == true;
        consent.IsChecked = false; busy = true; Commands();
        try
        {
            var target = await core.GetTargetStatusAsync(lifetime.Token);
            if (!target.Running || !target.HasTarget) { Error(() => PopupStrings.NoTarget); return; }
            await NativePasteActivation.WaitAsync(lifetime.Token);
            var result = await core.PasteNextAsync(accepted, target.Generation, allowed, lifetime.Token);
            if (disposed) return;
            feedback.Severity = result.PostPasteError is null ? InfoBarSeverity.Informational : InfoBarSeverity.Warning;
            feedbackText = () => result.PostPasteError is not null ? QueueStrings.Sent + " " + PopupStrings.PostPaste
                : result.Pasted ? QueueStrings.Sent : QueueStrings.Empty;
            feedback.Message = feedbackText();
            feedback.IsOpen = true;
        }
        catch (OperationCanceledException) { }
        catch (CoreException error) when (error.Code == "sensitive_confirmation")
        {
            if (!disposed) { consent.Visibility = Visibility.Visible; Error(() => QueueStrings.Sensitive); }
        }
        catch (Exception error) { if (!disposed) Error(error.Message); }
        finally { busy = false; Commands(); await ReloadAsync(); }
    }
    private void Error(string message) => Error(() => message);
    private void Error(Func<string> message)
    { feedbackText = message; feedback.Message = message(); feedback.Severity = InfoBarSeverity.Error; feedback.IsOpen = true; }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; poll.Stop(); poll.Tick -= Poll;
        LanguageService.Changed -= ApplyLanguage; lifetime.Cancel(); lifetime.Dispose();
        list.ItemsSource = null; snapshot = null; consent.IsChecked = false; feedback.IsOpen = false; feedbackText = null;
    }
}
