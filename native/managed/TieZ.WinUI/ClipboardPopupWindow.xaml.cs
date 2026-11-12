using System.Collections.ObjectModel;
using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using TieZ.Core;
using Windows.Graphics;
using VirtualKey = Windows.System.VirtualKey;

namespace TieZ.Native;

// This surface borrows the application's core. It owns no capture worker, tray or database.
public sealed partial class ClipboardPopupWindow : Window, IDisposable
{
    private readonly MainWindow owner;
    private readonly NativeCore core;
    private readonly ObservableCollection<PopupRow> rows = [];
    private readonly SemaphoreSlim thumbnailSlots = new(2, 2);
    private readonly CancellationTokenSource lifetime = new();
    private readonly EdgeDockController edgeDock;
    private readonly DispatcherQueueTimer debounce, poll;
    private CancellationTokenSource read = new();
    private ContentDialog? dialog;
    private MenuFlyout? menu;
    private Action? clearPreview;
    private TargetTrackingStatus? target;
    private ulong revision;
    private uint offset;
    private int generation;
    private bool disposed, busy, loading, polling, pending, hasMore, composing, relabeling, active;
    private long compositionEnded;
    private string? shellMessage;
    private static readonly string?[] Types = [null, "text", "code", "url", "rich_text", "image", "file"];
    internal bool IsBusy => busy || loading || pending;
    internal IReadOnlyList<PopupRow> Rows => rows;
    internal long? SelectedId => (Items.SelectedItem as PopupRow)?.Id;
    internal bool HasDialog => dialog is not null;
    internal bool HasPendingChanges => pending;
    internal string Message => Feedback.Message;
    internal bool IsComposing { get => composing; set => composing = value; }

    internal ClipboardPopupWindow(MainWindow owner, NativeCore core)
    {
        this.owner = owner; this.core = core;
        InitializeComponent();
        Title = "TieZ";
        ExtendsContentIntoTitleBar = true; SetTitleBar(DragRegion);
        AppWindow.IsShownInSwitchers = false;
        if (AppWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.SetBorderAndTitleBar(true, false);
            presenter.IsMaximizable = false; presenter.IsMinimizable = false;
        }
        Items.ItemsSource = rows;
        debounce = DispatcherQueue.CreateTimer(); debounce.IsRepeating = false;
        debounce.Interval = TimeSpan.FromMilliseconds(180);
        debounce.Tick += (_, _) => _ = ReloadAsync();
        poll = DispatcherQueue.CreateTimer(); poll.Interval = TimeSpan.FromMilliseconds(650);
        poll.Tick += Poll;
        SearchBox.TextChanging += (_, _) => ScheduleSearch();
        SearchBox.TextCompositionStarted += (_, _) => { composing = true; Invalidate(); debounce.Stop(); };
        SearchBox.TextCompositionEnded += (_, _) => { composing = false; compositionEnded = Environment.TickCount64; ScheduleSearch(); };
        TypeFilter.SelectionChanged += (_, _) => { if (!relabeling) ScheduleSearch(); };
        AllFilter.Click += (_, _) => { AllFilter.IsChecked = true; PinnedFilter.IsChecked = false; ScheduleSearch(); };
        PinnedFilter.Click += (_, _) => { PinnedFilter.IsChecked = true; AllFilter.IsChecked = false; ScheduleSearch(); };
        Hide.Click += (_, _) => HidePopup();
        More.Click += (_, _) => ShowMainMenu();
        Refresh.Click += (_, _) => _ = ReloadAsync();
        LoadMore.Click += (_, _) => _ = ReloadAsync(false);
        KeepOpen.Checked += (_, _) => ApplyKeepOpen(); KeepOpen.Unchecked += (_, _) => ApplyKeepOpen();
        Items.ContainerContentChanging += ContainerChanged;
        Root.KeyDown += OnKeyDown;
        Activated += (_, args) => active = args.WindowActivationState != WindowActivationState.Deactivated;
        AppWindow.Closing += (_, args) => { if (!disposed) { args.Cancel = true; HidePopup(); } };
        owner.ShutdownStarted += Dispose;
        core.Changed += ApplySettings;
        LanguageService.Changed += ApplyLanguage;
        edgeDock = new EdgeDockController(this, core);
        ApplySettings(core.Snapshot); ApplyLanguage();
    }

    private void ApplyKeepOpen()
    {
        if (AppWindow.Presenter is OverlappedPresenter presenter) presenter.IsAlwaysOnTop = KeepOpen.IsChecked == true;
    }
    private void ApplySettings(SettingsSnapshot snapshot)
    {
        Root.RequestedTheme = ViewTheme.From(snapshot);
        // Consent and decrypted previews never survive a settings/privacy transition.
        CloseTransient();
    }
    private void ApplyLanguage()
    {
        CloseTransient();
        Label(KeepOpen, PopupStrings.Keep); Label(More, PopupStrings.More); Label(Hide, PopupStrings.Hide);
        SearchBox.PlaceholderText = PopupStrings.Search; AutomationProperties.SetName(SearchBox, PopupStrings.Search);
        AllFilter.Content = PopupStrings.All; PinnedFilter.Content = PopupStrings.Pinned;
        Refresh.Content = CommonStrings.Current.Refresh; LoadMore.Content = CommonStrings.Current.LoadMore;
        AutomationProperties.SetName(Items, CommonStrings.Current.Clipboard);
        AutomationProperties.SetName(TypeFilter, PopupStrings.Types);
        int index = Math.Max(0, TypeFilter.SelectedIndex);
        relabeling = true;
        TypeFilter.Items.Clear();
        foreach (string? type in Types) TypeFilter.Items.Add(type is null ? PopupStrings.Types : HistoryRow.TypeLabelFor(type));
        TypeFilter.SelectedIndex = index;
        relabeling = false;
        Hint.Text = PopupStrings.Hint;
        UpdateEmpty();
        if (AppWindow.IsVisible) ScheduleSearch();
    }
    private static void Label(DependencyObject element, string text)
    { AutomationProperties.SetName(element, text); ToolTipService.SetToolTip(element, text); }

    internal async Task ShowAsync()
    {
        if (disposed || owner.ShutdownPending) return;
        // Snapshot before activation. Never silently replace this target during a visible session.
        try { target = await core.GetTargetStatusAsync(lifetime.Token); }
        catch (Exception) when (!disposed) { target = null; }
        if (disposed) return;
        Place(); AppWindow.Show(); Activate();
        poll.Start();
        await ReloadAsync();
        if (!disposed && AppWindow.IsVisible) SearchBox.Focus(FocusState.Programmatic);
    }
    internal Task ToggleAsync()
    {
        if (AppWindow.IsVisible && active) { HidePopup(); return Task.CompletedTask; }
        return ShowAsync();
    }
    private void Place()
    {
        GetCursorPos(out var point);
        var area = DisplayArea.GetFromPoint(new PointInt32(point.X, point.Y), DisplayAreaFallback.Nearest).WorkArea;
        double scale = 1;
        nint monitor = MonitorFromPoint(point, 2);
        if (GetDpiForMonitor(monitor, 0, out uint dpi, out _) == 0) scale = dpi / 96d;
        int width = Math.Min(area.Width, (int)Math.Round(400 * scale));
        int height = Math.Min(area.Height, (int)Math.Round(560 * scale));
        int x = Math.Clamp(point.X - width / 2, area.X, area.X + area.Width - width);
        int y = Math.Clamp(point.Y - 80, area.Y, area.Y + area.Height - height);
        AppWindow.MoveAndResize(new RectInt32(x, y, width, height));
    }
    internal void HidePopup()
    {
        if (disposed) return;
        CloseTransient(); debounce.Stop(); poll.Stop(); read.Cancel(); generation++;
        foreach (var row in rows) row.SetThumbnail(null);
        loading = false; AppWindow.Hide();
        // A failed tray must never leave the app unreachable.
        if (!owner.ResidentMode && !owner.ShutdownPending) owner.ShowMain();
    }
    private void CloseTransient()
    {
        menu?.Hide(); menu = null;
        clearPreview?.Invoke(); clearPreview = null;
        dialog?.Hide();
    }
    internal void SetMessage(string message)
    { if (disposed) return; Feedback.Message = message; Feedback.IsOpen = true; }
    internal void SetShellMessage(string? message)
    {
        if (disposed || message == shellMessage) return;
        if (message is not null) SetMessage(message);
        else if (Feedback.Message == shellMessage) Feedback.IsOpen = false;
        shellMessage = message;
    }
    private void Invalidate()
    {
        generation++; read.Cancel(); pending = true; loading = false;
        CloseTransient();
        foreach (var row in rows) row.SetThumbnail(null);
        Refresh.Content = PopupStrings.NewItems;
    }
    private void ScheduleSearch()
    {
        if (disposed) return;
        Invalidate(); debounce.Stop();
        if (!composing && AppWindow.IsVisible) debounce.Start();
    }
    internal void Search(string text) => SearchBox.Text = text;
    internal void Select(long id) => Items.SelectedItem = rows.FirstOrDefault(row => row.Id == id);

    internal async Task ReloadAsync(bool reset = true)
    {
        if (disposed || busy || composing || (!reset && (loading || pending || !hasMore))) return;
        long? selected = SelectedId;
        if (reset)
        {
            debounce.Stop(); Invalidate(); read.Dispose();
            read = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
        }
        int current = generation; var token = read.Token;
        loading = true;
        try
        {
            var query = new HistoryQuery(Limit: 50, Offset: reset ? 0 : offset,
                ExpectedRevision: reset ? null : revision, Search: SearchBox.Text,
                ContentType: Types[Math.Max(0, TypeFilter.SelectedIndex)], PinnedOnly: PinnedFilter.IsChecked == true);
            HistoryPage page;
            do
            {
                page = await core.GetHistoryAsync(query, token);
                if (disposed || current != generation || token.IsCancellationRequested) return;
                if (page.HasMore && page.NextOffset <= query.Offset) throw new InvalidOperationException("History paging did not advance.");
                query = query with { Offset = page.NextOffset, ExpectedRevision = page.Revision };
            } while (page.Items.Count == 0 && page.HasMore);
            if (reset) rows.Clear();
            revision = page.Revision; offset = page.NextOffset; hasMore = page.HasMore; pending = false;
            foreach (var item in page.Items) if (!rows.Any(row => row.Id == item.Id)) rows.Add(new(item, revision));
            Items.SelectedItem = rows.FirstOrDefault(row => row.Id == selected) ?? rows.FirstOrDefault();
            Refresh.Content = CommonStrings.Current.Refresh;
            LoadMore.Visibility = hasMore ? Visibility.Visible : Visibility.Collapsed;
            UpdateEmpty();
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (CoreException error) when (error.Code == "history_conflict")
        { if (!disposed && current == generation) { pending = true; SetMessage(PopupStrings.Changed); } }
        catch (Exception error) { if (!disposed && current == generation) SetMessage(error.Message); }
        finally { if (current == generation) loading = false; }
    }
    private void UpdateEmpty()
    {
        bool filtered = SearchBox.Text.Length > 0 || TypeFilter.SelectedIndex > 0 || PinnedFilter.IsChecked == true;
        Empty.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        EmptyTitle.Text = filtered ? WorkspaceStrings.Current.NoResults : WorkspaceStrings.Current.EmptyTitle;
        EmptyHint.Text = filtered ? WorkspaceStrings.Current.NoResultsHint
            : PopupStrings.Text("复制文字、图片或文件，然后在这里直接粘贴。", "複製文字、圖片或檔案，然後在這裡直接貼上。", "Copy text, images or files, then paste them from here.");
    }
    private async void Poll(DispatcherQueueTimer sender, object args)
    {
        if (disposed || polling || loading || !AppWindow.IsVisible) return;
        polling = true;
        int current = generation;
        try
        {
            var changes = await core.GetHistoryChangesAsync(revision, lifetime.Token);
            if (disposed || current != generation) return;
            if (changes.Revision != revision && !pending)
            {
                // Do not move the highlighted row underneath Enter. Explicit refresh adopts new records.
                Invalidate();
            }
            var status = await core.GetCaptureStatusAsync(lifetime.Token);
            if (!disposed) Hint.Text = status.Running ? PopupStrings.Hint : PopupStrings.Paused + " · " + PopupStrings.Hint;
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (Exception error) { if (!disposed) SetMessage(error.Message); }
        finally { polling = false; }
    }
    private new bool Current(PopupRow row) => !disposed && AppWindow.IsVisible && !pending && !loading
        && row.Revision == revision && rows.Contains(row);
    private async void ContainerChanged(ListViewBase sender, ContainerContentChangingEventArgs args)
    {
        var previous = args.ItemContainer.Tag as PopupRow;
        if (args.InRecycleQueue || disposed)
        {
            previous?.SetThumbnail(null); args.ItemContainer.Tag = null; return;
        }
        if (args.Item is not PopupRow row) return;
        if (previous is not null && !ReferenceEquals(previous, row)) previous.SetThumbnail(null);
        args.ItemContainer.Tag = row;
        AutomationProperties.SetName(args.ItemContainer, row.ActionName);
        if (row.ThumbnailRequested
            || row.Summary.IsSensitive || row.Summary.ContentType != "image") return;
        row.ThumbnailRequested = true;
        var request = CancellationTokenSource.CreateLinkedTokenSource(read.Token);
        row.ThumbnailRequest = request;
        var token = request.Token;
        bool entered = false;
        try
        {
            await thumbnailSlots.WaitAsync(token); entered = true;
            var preview = await core.GetImagePreviewAsync(row.Id, row.Revision, false, 96, token);
            var bitmap = await NativeImageBitmap.DecodeAsync(preview, token);
            if (Current(row) && !token.IsCancellationRequested && ReferenceEquals(row.ThumbnailRequest, request)) row.SetThumbnail(bitmap);
        }
        catch (Exception) { /* A thumbnail is optional; the original record and explicit preview stay available. */ }
        finally
        {
            if (entered) thumbnailSlots.Release();
            if (ReferenceEquals(row.ThumbnailRequest, request)) { row.ThumbnailRequest = null; request.Dispose(); }
        }
    }
    private async Task<bool> ConsentAsync(PopupRow row)
    {
        if (!row.Summary.IsSensitive) return true;
        var confirm = new ContentDialog { XamlRoot = Root.XamlRoot, RequestedTheme = Root.RequestedTheme,
            Title = HistoryStrings.Current.PreviewSensitive, Content = PopupStrings.Sensitive,
            PrimaryButtonText = CommonStrings.Current.Confirm, CloseButtonText = CommonStrings.Current.Cancel,
            DefaultButton = ContentDialogButton.Close };
        dialog = confirm;
        try { return await confirm.ShowAsync() == ContentDialogResult.Primary && Current(row); }
        finally { if (ReferenceEquals(dialog, confirm)) dialog = null; }
    }
    internal async Task UseAsync(PopupRow row, bool plainText = false, bool copyOnly = false)
    {
        if (pending && !busy && !composing) { SetMessage(PopupStrings.Changed); await ReloadAsync(); return; }
        if (busy || composing || dialog is not null || !Current(row)) return;
        Items.SelectedItem = row; busy = true;
        int current = generation;
        bool accepted = false;
        try
        {
            if (!await ConsentAsync(row) || !Current(row) || current != generation) return;
            if (copyOnly)
            {
                await core.CopyEntryAsync(row.Id, plainText, row.Summary.IsSensitive, lifetime.Token, row.Revision);
                accepted = true;
                SetMessage(plainText ? HistoryStrings.Current.CopySuccessPlain : HistoryStrings.Current.CopySuccessOriginal);
            }
            else
            {
                if (target is not { Running: true, HasTarget: true } destination) { SetMessage(PopupStrings.NoTarget); return; }
                // Wait for the initiating key/button release before transferring focus. Held Enter must not type in the target.
                long until = Environment.TickCount64 + 1500;
                while (ActivationKeysHeld() && Environment.TickCount64 < until) await Task.Delay(15, lifetime.Token);
                if (ActivationKeysHeld()) { SetMessage(PopupStrings.Failed); return; }
                if (!Current(row) || current != generation) return;
                var pasted = await core.PasteEntryAsync(row.Id, destination.Generation, plainText, row.Summary.IsSensitive,
                    lifetime.Token, expectedRevision: row.Revision);
                accepted = true;
                if (!disposed)
                {
                    if (pasted.PostPasteError is not null) SetMessage(PopupStrings.PostPaste);
                    if (KeepOpen.IsChecked != true) HidePopup();
                    else if (pasted.PostPasteError is null) SetMessage(PopupStrings.Sent);
                }
            }
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (disposed) { }
        catch (CoreException error)
        {
            if (error.Code is "history_conflict" or "sensitive_confirmation" or "not_found")
            { Invalidate(); SetMessage(PopupStrings.Changed); }
            else SetMessage(error.Message.StartsWith("Clipboard was written;", StringComparison.Ordinal)
                ? PopupStrings.Text("内容已复制，但自动粘贴未完成，可回到目标应用手动粘贴。", "內容已複製，但自動貼上未完成，可回到目標應用程式手動貼上。", "Copied, but automatic paste did not complete. Paste manually in the target app.")
                : copyOnly ? error.Message : PopupStrings.Failed);
        }
        catch (Exception error) { SetMessage(error.Message); }
        finally
        {
            busy = false;
            if (accepted && !disposed && AppWindow.IsVisible) await ReloadAsync();
        }
    }
    private static bool ActivationKeysHeld() => new[] { 1, 2, 13, 16, 17, 18, 0x5b, 0x5c, 0x56 }.Any(key => (GetAsyncKeyState(key) & 0x8000) != 0);
    private async void PasteClick(object sender, RoutedEventArgs args)
    { if ((sender as FrameworkElement)?.DataContext is PopupRow row) await UseAsync(row, DefaultPlainText(row)); }
    // Match the legacy left-click/plain-text and right-click/original-format distinction.
    private static bool DefaultPlainText(PopupRow row) => row.Summary.ContentType is "text" or "code" or "url" or "rich_text";
    private async void OriginalRightTapped(object sender, RightTappedRoutedEventArgs args)
    { args.Handled = true; if ((sender as FrameworkElement)?.DataContext is PopupRow row) await UseAsync(row); }
    private async void PreviewClick(object sender, RoutedEventArgs args)
    { if ((sender as FrameworkElement)?.DataContext is PopupRow row) await PreviewAsync(row); }
    private void RowMenuClick(object sender, RoutedEventArgs args)
    { if (sender is FrameworkElement { DataContext: PopupRow row } anchor) ShowRowMenu(row, anchor); }

    internal async Task PreviewAsync(PopupRow row)
    {
        if (busy || dialog is not null || !Current(row)) return;
        busy = true; Items.SelectedItem = row;
        var token = read.Token;
        TextBox? text = null; ImagePreviewView? image = null; DocumentPreviewView? document = null;
        try
        {
            if (!await ConsentAsync(row) || !Current(row)) return;
            var entry = await core.GetPreviewEntryAsync(row.Id, row.Revision, row.Summary.IsSensitive, token);
            if (!Current(row)) return;
            UIElement content;
            if (entry.ContentType == "image")
            {
                image = new ImagePreviewView { Height = 280 };
                await image.ShowAsync(await core.GetImagePreviewAsync(row.Id, row.Revision, row.Summary.IsSensitive, 1024, token), token);
                content = image;
            }
            else if (entry.ContentType == "rich_text")
            {
                document = new DocumentPreviewView { Height = 280 };
                document.Show(await core.GetDocumentPreviewAsync(row.Id, row.Revision, row.Summary.IsSensitive, token), entry.Content);
                content = document;
            }
            else content = text = new TextBox { Text = entry.Content, IsReadOnly = true, AcceptsReturn = true,
                TextWrapping = TextWrapping.Wrap, MaxHeight = 280, MinWidth = 220 };
            if (!Current(row)) return;
            var preview = new ContentDialog { XamlRoot = Root.XamlRoot, RequestedTheme = Root.RequestedTheme,
                Title = HistoryRow.TypeLabelFor(entry.ContentType), Content = content, CloseButtonText = CommonStrings.Current.Close };
            clearPreview = () => { if (text is not null) text.Text = ""; image?.Clear(); document?.Clear(); preview.Content = null; };
            dialog = preview;
            await preview.ShowAsync();
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { if (!disposed) SetMessage(error is CoreException { Code: "history_conflict" } ? PopupStrings.Changed : error.Message); }
        finally
        {
            if (text is not null) text.Text = ""; image?.Clear(); document?.Clear();
            clearPreview?.Invoke(); clearPreview = null; dialog = null; busy = false;
        }
    }
    private MenuFlyoutItem Command(string text, Action action)
    { var item = new MenuFlyoutItem { Text = text }; item.Click += (_, _) => action(); return item; }
    private void ShowMainMenu()
    {
        menu = new MenuFlyout { Placement = FlyoutPlacementMode.BottomEdgeAlignedRight };
        menu.Items.Add(Command(PopupStrings.Manage, () => { HidePopup(); owner.ShowMain(); }));
        menu.Items.Add(Command(CommonStrings.Current.Settings, () => { HidePopup(); owner.OpenSettings(); }));
        menu.Items.Add(new MenuFlyoutSeparator());
        menu.Items.Add(Command(ShellStrings.Current.Quit, () => _ = owner.Quit()));
        menu.ShowAt(More);
    }
    private void ShowRowMenu(PopupRow row, FrameworkElement anchor)
    {
        if (!Current(row) || busy) return;
        Items.SelectedItem = row;
        menu = new MenuFlyout();
        menu.Items.Add(Command(PopupStrings.Original, () => _ = UseAsync(row)));
        if (row.Summary.ContentType is "text" or "code" or "url" or "rich_text")
            menu.Items.Add(Command(CommonStrings.Current.PastePlainText, () => _ = UseAsync(row, true)));
        menu.Items.Add(Command(CommonStrings.Current.Copy, () => _ = UseAsync(row, copyOnly: true)));
        menu.Items.Add(Command(PopupStrings.Preview, () => _ = PreviewAsync(row)));
        menu.Items.Add(new MenuFlyoutSeparator());
        menu.Items.Add(Command(row.Summary.IsPinned ? HistoryStrings.Current.UnpinAction : HistoryStrings.Current.PinAction, () => _ = PinAsync(row)));
        menu.Items.Add(Command(PopupStrings.Manage, () => { HidePopup(); owner.ShowMain(); }));
        menu.ShowAt(anchor);
    }
    private async Task PinAsync(PopupRow row)
    {
        if (!Current(row) || busy) return;
        busy = true;
        try
        {
            var result = await core.SetPinnedAsync(row.Id, !row.Summary.IsPinned, lifetime.Token);
            busy = false; await ReloadAsync(); Select(result.Id);
        }
        catch (Exception error) { SetMessage(error.Message); }
        finally { busy = false; }
    }
    private void OnKeyDown(object sender, KeyRoutedEventArgs args)
    {
        if (args.Handled || composing || Environment.TickCount64 - compositionEnded < 150) return;
        if (dialog is not null) return;
        if (args.Key == VirtualKey.Escape)
        {
            args.Handled = true;
            if (VisualTreeHelper.GetOpenPopupsForXamlRoot(Root.XamlRoot).Any(p => p.IsOpen)) { menu?.Hide(); return; }
            if (SearchBox.Text.Length > 0) SearchBox.Text = ""; else HidePopup();
            return;
        }
        if (busy || loading || pending) return;
        var focus = FocusManager.GetFocusedElement(Root.XamlRoot);
        bool navigation = ReferenceEquals(focus, SearchBox) || focus is ListViewItem || ReferenceEquals(focus, Items);
        bool rowAction = focus is FrameworkElement { DataContext: PopupRow };
        if (args.Key is VirtualKey.Up or VirtualKey.Down && (navigation || rowAction))
        {
            args.Handled = true;
            if (rows.Count == 0) return;
            Items.SelectedIndex = Math.Clamp(Items.SelectedIndex + (args.Key == VirtualKey.Down ? 1 : -1), 0, rows.Count - 1);
            Items.ScrollIntoView(Items.SelectedItem);
            SearchBox.Focus(FocusState.Programmatic);
        }
        else if (args.Key == VirtualKey.Enter && navigation && !args.KeyStatus.WasKeyDown && Items.SelectedItem is PopupRow row)
        { args.Handled = true; _ = UseAsync(row, DefaultPlainText(row)); }
        else if (args.Key == VirtualKey.F2 && Items.SelectedItem is PopupRow previewRow)
        { args.Handled = true; _ = PreviewAsync(previewRow); }
        else if (args.Key == VirtualKey.Application && Items.SelectedItem is PopupRow menuRow)
        { args.Handled = true; ShowRowMenu(menuRow, Items); }
        else if (args.Key == VirtualKey.F && (GetAsyncKeyState(17) & 0x8000) != 0)
        { args.Handled = true; SearchBox.Focus(FocusState.Keyboard); SearchBox.SelectAll(); }
    }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; edgeDock.Dispose(); CloseTransient(); poll.Stop(); debounce.Stop();
        owner.ShutdownStarted -= Dispose; core.Changed -= ApplySettings; LanguageService.Changed -= ApplyLanguage;
        read.Cancel(); read.Dispose(); lifetime.Cancel(); lifetime.Dispose();
        foreach (var row in rows) row.SetThumbnail(null); rows.Clear(); Close();
    }
    [StructLayout(LayoutKind.Sequential)] private struct Point { public int X, Y; }
    [DllImport("user32.dll")] private static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] private static extern nint MonitorFromPoint(Point point, uint flags);
    [DllImport("shcore.dll")] private static extern int GetDpiForMonitor(nint monitor, int type, out uint x, out uint y);
    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
}
