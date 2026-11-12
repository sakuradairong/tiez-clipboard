using Microsoft.UI.Xaml.Controls;
using TieZ.Core;

namespace TieZ.Native;

public sealed partial class HistoryView
{
    private PasteQueueView? pasteQueueView;
    internal PasteQueueView? PasteQueue => pasteQueueView;
    internal ContentDialog? QueueDialog => pasteQueueView is null ? null : activeDialog;

    internal async Task ShowPasteQueueAsync(bool replaceSelection = false)
    {
        if (disposed || core is null || loading || mutating || filterPending || activeDialog is not null) return;
        var selected = ActiveList.SelectedItems.Cast<HistoryRow>().ToHashSet();
        if (replaceSelection && (externalPending || revision is null || selected.Count == 0 || selected.Any(row => row.IsStale))) return;
        long[] ids = rows.Where(selected.Contains).Select(row => row.Id).ToArray();
        ulong? selectedRevision = revision;
        mutating = true; UpdateCommands();
        try
        {
            if (replaceSelection)
            {
                var queue = await core.GetPasteQueueAsync(lifetime.Token);
                await core.ReplacePasteQueueAsync(ids, queue.Revision, selectedRevision!.Value, lifetime.Token);
            }
            if (disposed) return;
            var view = new PasteQueueView(core);
            pasteQueueView = view;
            var dialog = new ContentDialog { XamlRoot = XamlRoot, RequestedTheme = RequestedTheme,
                Content = view, DefaultButton = ContentDialogButton.Close };
            dialogChrome = () => { dialog.Title = QueueStrings.Title; dialog.CloseButtonText = CommonStrings.Current.Close; };
            dialogChrome(); activeDialog = dialog;
            void Theme(SettingsSnapshot value) { dialog.RequestedTheme = ViewTheme.From(value); }
            core.Changed += Theme;
            dialog.Closing += (_, _) => view.Dispose();
            mutating = false; UpdateCommands();
            try { await dialog.ShowAsync(); }
            finally { core.Changed -= Theme; view.Dispose(); dialog.Content = null; pasteQueueView = null; activeDialog = null; dialogChrome = null; }
        }
        catch (OperationCanceledException) { }
        catch (Exception error) { if (!disposed) ShowError(error.Message); }
        finally { mutating = false; if (!disposed) UpdateCommands(); }
    }
}
