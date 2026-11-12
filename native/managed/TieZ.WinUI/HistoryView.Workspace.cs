using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace TieZ.Native;

public sealed partial class HistoryView
{
    private void ApplyWorkspaceLanguage()
    {
        FiltersTitle.Text = WorkspaceStrings.Current.Filters;
        AutomationProperties.SetName(FiltersButton, WorkspaceStrings.Current.Filters);
        ToolTipService.SetToolTip(FiltersButton, WorkspaceStrings.Current.Filters);
        OrderChoice.Header = HistoryStrings.Current.SortName;
        LayoutChoice.Header = HistoryStrings.Current.LayoutName;
    }

    // Presentation only: selection, consent and command availability remain owned by UpdateCommands.
    private void UpdateWorkspaceLayout()
    {
        if (disposed) return;
        bool selected = ActiveList.SelectedItems.Count == 1;
        bool wide = WorkspaceRoot.ActualWidth >= 820;
        DetailPanel.Visibility = selected ? Visibility.Visible : Visibility.Collapsed;
        DetailColumn.Width = new GridLength(selected && wide ? 300 : 0);
        DetailRow.Height = new GridLength(selected && !wide ? Math.Min(230, Math.Max(150, HistoryBody.ActualHeight * .4)) : 0);
        Grid.SetColumn(DetailPanel, wide ? 1 : 0);
        Grid.SetRow(DetailPanel, wide ? 0 : 1);
        HistoryBody.ColumnSpacing = selected && wide ? 12 : 0;
        HistoryBody.RowSpacing = selected && !wide ? 10 : 0;

        bool filtered = SearchText.Text.Length > 0 || TagFilter.Text.Length > 0 || exactTagFilter is not null
            || TypeFilter.SelectedIndex > 0 || PinnedOnly.IsChecked == true || TagOnly.IsChecked == true;
        ActiveFilterDot.Visibility = filtered ? Visibility.Visible : Visibility.Collapsed;
        EmptyHistory.Visibility = historyLoaded && !loading && !filterPending && rows.Count == 0
            ? Visibility.Visible : Visibility.Collapsed;
        EmptyTitle.Text = filtered ? WorkspaceStrings.Current.NoResults : WorkspaceStrings.Current.EmptyTitle;
        EmptyDescription.Text = filtered ? WorkspaceStrings.Current.NoResultsHint : WorkspaceStrings.Current.EmptyHint;
    }
}
