using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;

namespace TieZ.Native;

internal sealed record ContentOpenRequest(long Id, ulong Revision, bool AllowSensitive);

public sealed partial class HistoryView
{
    private CancellationTokenSource? contentOpenCancellation;
    private Button? contentOpenAnchor;
    private long? contentOpenId;
    // Fixtures may hold an otherwise real request/result to exercise queue and dispatch races.
    internal Func<ContentOpenRequest, CancellationToken, Task<OpenContentResult>>? ContentOpener { get; set; }
    internal bool ContentOpenPending => contentOpenCancellation is not null;
    internal bool CanOpen => OpenButton.IsEnabled;
    internal OpenContentResult? LastOpenResult { get; private set; }
    internal string OpenFeedback => Feedback.Message;
    internal string OpenFeedbackTitle => Feedback.Title;
    private bool CanOpenNow => !disposed && core is not null && thumbnailActive && IsLoaded && Visibility == Visibility.Visible
        && !loading && !mutating && !filterPending && !externalPending && !targetBusy && activeDialog is null
        && revision is not null && revision == observedRevision;
    private bool CanOpenSelected => CanOpenNow && ActiveList.SelectedItems.Count == 1
        && ActiveList.SelectedItem is HistoryRow { IsStale: false } row && selectedEntry?.Id == row.Id;
    private static bool IsCardOpenButton(Button button) => button.Tag is "card-open";
    private void InitializeContentOpens()
    {
        HistoryCards.LayoutUpdated += OnOpenLayoutUpdated;
        OpenButton.Click += OnSelectedOpenClick;
    }
    private void OnOpenLayoutUpdated(object? sender, object args) => UpdateContentOpenControls();
    private void UpdateContentOpenControls()
    {
        OpenButton.IsEnabled = CanOpenSelected;
        // Card copy and open share the same revision/viewport/single-sensitive-consent guard.
        foreach (var button in Descendants<Button>(HistoryCards).Where(IsCardOpenButton)) button.IsEnabled = CanCopyCard(button);
        if (contentOpenAnchor is { } source && (!source.IsLoaded || source.DataContext is not HistoryRow row
            || row.Id != contentOpenId || row.IsStale || !rows.Contains(row) || !IsHistoryItemVisible(row.Id))) CancelContentOpen();
    }
    private void OnCardOpenLoaded(object sender, RoutedEventArgs args)
    {
        if (sender is Button button) button.IsEnabled = CanCopyCard(button);
    }
    private void OnCardOpenContextChanged(FrameworkElement sender, DataContextChangedEventArgs args)
    {
        if (ReferenceEquals(sender, contentOpenAnchor) && (sender.DataContext is not HistoryRow row || row.Id != contentOpenId)) CancelContentOpen();
        if (sender is Button button) button.IsEnabled = CanCopyCard(button);
    }
    private void OnCardOpenUnloaded(object sender, RoutedEventArgs args)
    {
        if (ReferenceEquals(sender, contentOpenAnchor)) CancelContentOpen();
        if (sender is Button button) button.IsEnabled = false;
    }
    private async void OnSelectedOpenClick(object sender, RoutedEventArgs args)
    {
        if (!CanOpenSelected || ActiveList.SelectedItem is not HistoryRow row) return;
        await OpenHistoryAsync(new(row.Id, revision!.Value, row.Summary.IsSensitive));
    }
    private async void OnCardOpenClick(object sender, RoutedEventArgs args)
    {
        if (sender is not Button button || !IsCardOpenButton(button) || !CanCopyCard(button)
            || button.DataContext is not HistoryRow row) return;
        await OpenHistoryAsync(new(row.Id, revision!.Value, row.Summary.IsSensitive), button);
    }
    private async Task OpenHistoryAsync(ContentOpenRequest request, Button? source = null)
    {
        var owner = core!;
        var cancellation = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
        contentOpenCancellation = cancellation; contentOpenAnchor = source; contentOpenId = request.Id;
        LastOpenResult = null; mutating = true; UpdateCommands();
        try
        {
            var result = await (ContentOpener is { } opener ? opener(request, cancellation.Token)
                : owner.OpenContentAsync(request.Id, request.Revision, request.AllowSensitive, cancellation.Token));
            if (result.Id != request.Id || result.HistoryRevision != request.Revision)
                throw new InvalidDataException("Open result identity or revision changed.");
            // A submitted native outcome stays authoritative after cancellation or card recycling.
            if (!disposed) { LastOpenResult = result; ShowOpenSuccess(result); }
        }
        catch (OperationCanceledException) { if (!disposed) ShowError("打开请求已取消。"); }
        catch (ObjectDisposedException) when (disposed) { }
        catch (InvalidDataException) { if (!disposed) ShowError("无法核实打开结果，请先检查外部应用，避免重复打开。"); }
        catch (Exception error)
        {
            if (!disposed) ShowError(error is CoreException { Code: "history_conflict" }
                ? "记录已变化，本次未打开。请刷新后重试。"
                : error is CoreException { Code: "sensitive_confirmation" }
                    ? "请先单独选中敏感记录并显示详情，再打开。"
                    : "无法打开记录，请检查默认应用或应用设置后重试。");
        }
        finally
        {
            contentOpenCancellation = null; contentOpenAnchor = null; contentOpenId = null;
            cancellation.Dispose(); mutating = false;
            if (!disposed) UpdateCommands();
        }
    }
    private void ShowOpenSuccess(OpenContentResult result)
    {
        Feedback.Title = result.UsageError is null ? "已打开" : "已打开，使用次数未更新";
        Feedback.Message = result.Exported
            ? !result.EditWritebackAvailable
                ? "临时副本已打开，修改不会回写历史。" : "临时副本已打开。"
            : "已交给系统或所选应用打开。";
        if (result.UsageError is not null) Feedback.Message += " 使用次数未能更新，请勿为重试计数而重复打开。";
        Feedback.Severity = result.UsageError is null ? InfoBarSeverity.Success : InfoBarSeverity.Warning;
        Feedback.IsOpen = true;
    }
    private void CancelContentOpen() => contentOpenCancellation?.Cancel();
    private void ReleaseContentOpens()
    {
        HistoryCards.LayoutUpdated -= OnOpenLayoutUpdated; OpenButton.Click -= OnSelectedOpenClick;
        CancelContentOpen(); ContentOpener = null; contentOpenAnchor = null; LastOpenResult = null;
        OpenButton.IsEnabled = false;
        foreach (var button in Descendants<Button>(HistoryCards).Where(IsCardOpenButton)) button.IsEnabled = false;
    }
    private Button? CardOpenButton(long id) => Descendants<Button>(HistoryCards).FirstOrDefault(button => IsCardOpenButton(button)
        && button.DataContext is HistoryRow row && row.Id == id && button.IsLoaded);
    internal bool CardCanOpen(long id) => CardOpenButton(id)?.IsEnabled == true;
    internal void InvokeCardOpen(long id)
    {
        var button = CardOpenButton(id) ?? throw new InvalidOperationException("Card open control is not realized.");
        if (!button.IsEnabled) throw new InvalidOperationException("Card open is disabled.");
        ((IInvokeProvider)FrameworkElementAutomationPeer.CreatePeerForElement(button).GetPattern(PatternInterface.Invoke)).Invoke();
    }
}
