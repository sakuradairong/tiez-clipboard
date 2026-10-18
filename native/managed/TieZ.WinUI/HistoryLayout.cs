using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;

namespace TieZ.Native;

public sealed partial class HistoryView
{
    private bool cards, bindingLayout, switchingLayout, tagLayoutScope;
    private string ordinaryLayout = "list", tagLayout = "grid";
    private ListViewBase ActiveList => cards ? HistoryCards : HistoryList;
    private string LayoutKey => tagLayoutScope ? "app.tag_history_view_mode" : "app.history_view_mode";
    internal string ViewLayout => cards ? "grid" : "list";
    internal bool CanChangeLayout => LayoutChoice.IsEnabled;
    internal bool InactiveLayoutReleased => (cards ? (ListViewBase)HistoryList : HistoryCards).ItemsSource is null;
    internal long[] SelectedHistoryIds => ActiveList.SelectedItems.Cast<HistoryRow>().Select(row => row.Id).ToArray();
    internal double CardWidth => (HistoryCards.ItemsPanelRoot as ItemsWrapGrid)?.ItemWidth ?? 0;
    internal int CardColumns => CardWidth <= 0 ? 0 : Math.Max(1, (int)Math.Floor((HistoryCards.ActualWidth - 20) / CardWidth));
    internal long? ViewportAnchorId => ViewportAnchor()?.Id;
    internal bool HasLayoutResources => HistoryList.ItemsSource is not null || HistoryCards.ItemsSource is not null || rows.Count > 0;
    internal int RealizedColumns => rows.Select(row => ActiveList.ContainerFromItem(row)).OfType<FrameworkElement>()
        .Where(item => item.ActualHeight > 0).Select(item => Math.Round(item.TransformToVisual(ActiveList).TransformPoint(default).X)).Distinct().Count();
    internal void ScrollIntoHistory(long id) => ActiveList.ScrollIntoView(rows.Single(row => row.Id == id), ScrollIntoViewAlignment.Leading);
    internal bool IsHistoryItemVisible(long id)
    {
        if (ActiveList.ContainerFromItem(rows.FirstOrDefault(row => row.Id == id)) is not FrameworkElement item) return false;
        var origin = item.TransformToVisual(ActiveList).TransformPoint(default);
        return origin.Y + item.ActualHeight > 0 && origin.Y < ActiveList.ActualHeight;
    }

    private void InitializeHistoryLayouts()
    {
        HistoryList.ItemsSource = rows;
        foreach (var view in new ListViewBase[] { HistoryList, HistoryCards })
        {
            view.SelectionChanged += OnHistorySelectionChanged;
            view.Loaded += OnHistoryLayoutLoaded;
        }
        HistoryCards.SizeChanged += OnCardSizeChanged;
        LayoutChoice.SelectionChanged += OnLayoutChoiceChanged;
        ReloadLayoutButton.Click += OnReloadLayout;
    }
    private void OnHistorySelectionChanged(object sender, SelectionChangedEventArgs args)
    {
        if (!disposed && !switchingLayout && ReferenceEquals(sender, ActiveList)) _ = LoadDetailAsync(false);
    }
    private void OnHistoryLayoutLoaded(object sender, RoutedEventArgs args)
    {
        if (!disposed && ReferenceEquals(sender, ActiveList)) { AttachScroll(); SizeCards(); }
    }
    private void OnCardSizeChanged(object sender, SizeChangedEventArgs args) => SizeCards();
    private void SizeCards()
    {
        if (disposed || !cards || HistoryCards.ItemsPanelRoot is not ItemsWrapGrid panel) return;
        double width = Math.Max(1, HistoryCards.ActualWidth - 20);
        int columns = Math.Max(1, (int)Math.Floor(width / 240));
        panel.ItemWidth = Math.Max(1, Math.Floor(width / columns));
    }
    internal void ApplyViewPreferences(SettingsSnapshot snapshot)
    {
        if (disposed) return;
        ordinaryLayout = snapshot.Values["app.history_view_mode"];
        tagLayout = snapshot.Values["app.tag_history_view_mode"];
        ApplyAcceptedLayout();
    }
    private void ApplyAcceptedLayout()
    {
        bool useCards = (tagLayoutScope ? tagLayout : ordinaryLayout) == "grid";
        bindingLayout = true;
        try { LayoutChoice.SelectedIndex = useCards ? 1 : 0; }
        finally { bindingLayout = false; }
        if (cards == useCards) return;
        var old = ActiveList;
        var selection = SelectedHistoryIds;
        var anchor = ViewportAnchor();
        bool hadFocus = old.FocusState != FocusState.Unfocused || Descendants<Control>(old).Any(control => control.FocusState != FocusState.Unfocused);
        PreviewInvalidated?.Invoke(); hoveredAnchor = null;
        switchingLayout = true;
        try
        {
            if (listScroll is not null) listScroll.ViewChanged -= OnScroll;
            listScroll = null;
            old.ItemsSource = null; old.Visibility = Visibility.Collapsed;
            cards = useCards;
            ActiveList.ItemsSource = rows; ActiveList.Visibility = Visibility.Visible;
            ActiveList.UpdateLayout(); SizeCards();
            foreach (long id in selection)
                if (rows.FirstOrDefault(row => row.Id == id) is { } row) ActiveList.SelectedItems.Add(row);
            if (anchor is not null && rows.Contains(anchor)) ActiveList.ScrollIntoView(anchor, ScrollIntoViewAlignment.Leading);
            AttachScroll();
            if (hadFocus) ActiveList.Focus(FocusState.Programmatic);
        }
        finally { switchingLayout = false; }
        // New containers must not retain a previously revealed sensitive body/preview.
        _ = LoadDetailAsync(false);
        UpdateCommands();
    }
    private HistoryRow? ViewportAnchor()
    {
        if (!ActiveList.IsLoaded) return null;
        foreach (var row in rows)
        {
            if (ActiveList.ContainerFromItem(row) is not FrameworkElement container || container.ActualHeight <= 0) continue;
            var origin = container.TransformToVisual(ActiveList).TransformPoint(default);
            if (origin.Y + container.ActualHeight > 0 && origin.Y < ActiveList.ActualHeight) return row;
        }
        return null;
    }
    private void OnLayoutChoiceChanged(object sender, SelectionChangedEventArgs args)
    {
        if (bindingLayout || disposed || core is null) return;
        if (!LayoutChoice.IsEnabled) { ApplyAcceptedLayout(); return; }
        string requested = LayoutChoice.SelectedIndex == 1 ? "grid" : "list";
        try { core.Update(new Dictionary<string,string> { [LayoutKey] = requested }, core.Snapshot.Revision); ApplyAcceptedLayout(); }
        catch (Exception error) { ApplyAcceptedLayout(); ShowError($"布局未保存，已保留原布局。{error.Message} 可重读布局后重试。"); }
    }
    private void OnReloadLayout(object sender, RoutedEventArgs args)
    {
        if (disposed || core is null) return;
        try { ApplyViewPreferences(core.Refresh()); }
        catch (Exception error) { ShowError(error.Message); }
    }
    internal void ChooseLayout(string layout)
    {
        if (!LayoutChoice.IsEnabled) throw new InvalidOperationException("Layout selection is disabled.");
        LayoutChoice.SelectedIndex = layout switch { "list" => 0, "grid" => 1, _ => throw new ArgumentException(layout) };
    }
    private void ReleaseHistoryLayouts()
    {
        switchingLayout = true;
        foreach (var view in new ListViewBase[] { HistoryList, HistoryCards })
        {
            view.SelectionChanged -= OnHistorySelectionChanged; view.Loaded -= OnHistoryLayoutLoaded; view.ItemsSource = null;
        }
        HistoryCards.SizeChanged -= OnCardSizeChanged; LayoutChoice.SelectionChanged -= OnLayoutChoiceChanged;
        ReloadLayoutButton.Click -= OnReloadLayout; rows.Clear();
    }
}
