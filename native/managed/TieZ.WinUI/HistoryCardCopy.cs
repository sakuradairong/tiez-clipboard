using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;

namespace TieZ.Native;

internal sealed record CardCopyRequest(long Id, bool PlainText, bool AllowSensitive, ulong Revision);

public sealed partial class HistoryView
{
    private CancellationTokenSource? cardCopyCancellation;
    internal Func<CardCopyRequest, CancellationToken, Task<ClipboardWriteResult>>? CardCopyWriter { get; set; }
    internal bool CardCopyPending => cardCopyCancellation is not null;
    internal string CopyFeedback => Feedback.Message;
    private void InitializeCardCopies() => HistoryCards.LayoutUpdated += OnCardCopyLayoutUpdated;
    private void OnCardCopyLayoutUpdated(object? sender, object args) => UpdateCardCopyButtons();
    private static bool IsCardCopyButton(Button button) => button.Tag is "card-copy" or "card-copy-plain";
    private bool CanCopyCard(Button button)
    {
        if (disposed || core is null || !thumbnailActive || !cards || !IsLoaded || Visibility != Visibility.Visible
            || loading || mutating || filterPending || externalPending || targetBusy || activeDialog is not null
            || revision is null || revision != observedRevision || !button.IsLoaded
            || button.DataContext is not HistoryRow { IsStale: false } row || !rows.Contains(row)
            || button.ActualWidth <= 0 || button.ActualHeight <= 0) return false;
        if (row.Summary.IsSensitive && (selectedEntry?.Id != row.Id || ActiveList.SelectedItems.Count != 1
            || !ReferenceEquals(ActiveList.SelectedItem, row))) return false;
        var point = button.TransformToVisual(HistoryCards).TransformPoint(default);
        return point.Y + button.ActualHeight > 0 && point.Y < HistoryCards.ActualHeight
            && point.X + button.ActualWidth > 0 && point.X < HistoryCards.ActualWidth;
    }
    private void UpdateCardCopyButtons()
    {
        foreach (var button in Descendants<Button>(HistoryCards).Where(IsCardCopyButton)) button.IsEnabled = CanCopyCard(button);
    }
    private void OnCardCopyLoaded(object sender, RoutedEventArgs args)
    {
        if (sender is Button button) button.IsEnabled = CanCopyCard(button);
    }
    private void OnCardCopyContextChanged(FrameworkElement sender, DataContextChangedEventArgs args)
    {
        if (sender is Button button) button.IsEnabled = CanCopyCard(button);
    }
    private void OnCardCopyUnloaded(object sender, RoutedEventArgs args)
    {
        if (sender is Button button) button.IsEnabled = false;
    }
    private async void OnCardCopyClick(object sender, RoutedEventArgs args)
    {
        if (sender is not Button button || !IsCardCopyButton(button) || !CanCopyCard(button)
            || button.DataContext is not HistoryRow row) return;
        // Capture identity/consent/revision now; never consult a recycled template after awaiting.
        var request = new CardCopyRequest(row.Id, button.Tag is "card-copy-plain", row.Summary.IsSensitive, revision!.Value);
        var cancellation = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
        cardCopyCancellation = cancellation; mutating = true; UpdateCommands();
        try
        {
            var result = await (CardCopyWriter is { } writer ? writer(request, cancellation.Token)
                : core!.CopyEntryAsync(request.Id, request.PlainText, request.AllowSensitive, cancellation.Token, request.Revision));
            if (result.Id != request.Id) throw new InvalidDataException("Copy result identity changed.");
            // Once Rust accepts a write, later cancellation cannot turn its result into 'cancelled'.
            if (!disposed) ShowCopySuccess(request.PlainText);
        }
        catch (OperationCanceledException) { if (!disposed) ShowError("复制请求已取消。"); }
        catch (ObjectDisposedException) when (disposed) { }
        catch (Exception error) { if (!disposed) ShowCopyError(error); }
        finally
        {
            cardCopyCancellation = null; cancellation.Dispose(); mutating = false;
            if (!disposed) UpdateCommands();
        }
    }
    private void CancelCardCopy() => cardCopyCancellation?.Cancel();
    private void ShowCopySuccess(bool plainText)
    {
        Feedback.Title = "已复制";
        Feedback.Message = plainText ? "已复制为纯文本。" : "已按原始格式复制到剪贴板。";
        Feedback.Severity = InfoBarSeverity.Success; Feedback.IsOpen = true;
    }
    private void ShowCopyError(Exception error) => ShowError(error is CoreException { Code: "history_conflict" }
        ? "记录已变化，本次未复制。请刷新后重试。" : error.Message);
    private void ReleaseCardCopies()
    {
        HistoryCards.LayoutUpdated -= OnCardCopyLayoutUpdated; CancelCardCopy(); CardCopyWriter = null;
        foreach (var button in Descendants<Button>(HistoryCards).Where(IsCardCopyButton)) button.IsEnabled = false;
    }
    private Button? CardCopyButton(long id, bool plainText) => Descendants<Button>(HistoryCards)
        .FirstOrDefault(button => button.Tag is string tag && tag == (plainText ? "card-copy-plain" : "card-copy")
            && button.DataContext is HistoryRow row && row.Id == id && button.IsLoaded);
    internal bool CardCanCopy(long id, bool plainText = false) => CardCopyButton(id, plainText)?.IsEnabled == true;
    internal void InvokeCardCopy(long id, bool plainText = false)
    {
        var button = CardCopyButton(id, plainText) ?? throw new InvalidOperationException("Card copy control is not realized.");
        if (!button.IsEnabled) throw new InvalidOperationException("Card copy is disabled.");
        ((IInvokeProvider)FrameworkElementAutomationPeer.CreatePeerForElement(button).GetPattern(PatternInterface.Invoke)).Invoke();
    }
}
