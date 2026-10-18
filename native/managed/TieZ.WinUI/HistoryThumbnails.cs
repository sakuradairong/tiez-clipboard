using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using TieZ.Core;

namespace TieZ.Native;

public sealed partial class HistoryView
{
    private sealed class ThumbnailRequest(HistoryRow row, ulong revision, CancellationTokenSource cancellation)
    {
        internal readonly HistoryRow Row = row;
        internal readonly ulong Revision = revision;
        internal CancellationTokenSource? Cancellation = cancellation;
    }
    private readonly Dictionary<HistoryThumbnailView, ThumbnailRequest> thumbnails = [];
    private bool thumbnailActive = true, thumbnailRefreshQueued;
    private int thumbnailRequests, thumbnailPeakRequests;
    // The smoke fixture can delay an otherwise real Rust response, modelling non-abortable FFI work.
    internal Func<long, ulong, CancellationToken, Task<ImagePreviewData>>? ThumbnailReader { get; set; }
    internal int ThumbnailRequests => thumbnailRequests;
    internal int ThumbnailPeakRequests => thumbnailPeakRequests;
    internal int ThumbnailResources => thumbnails.Count;
    internal long[] ThumbnailIds => thumbnails.Keys.Where(view => view.HasImage).Select(view => view.DisplayedId!.Value).ToArray();
    internal long[] UnavailableThumbnailIds => thumbnails.Keys.Where(view => view.Unavailable).Select(view => view.Row!.Id).ToArray();
    internal long[] VisibleThumbnailIds => Descendants<HistoryThumbnailView>(HistoryCards).Where(IsThumbnailVisible)
        .Select(view => view.Row!.Id).ToArray();

    private void InitializeThumbnails() => HistoryCards.LayoutUpdated += OnThumbnailLayoutUpdated;
    private void OnThumbnailLayoutUpdated(object? sender, object args) => QueueThumbnailRefresh();
    internal void SetWorkspaceActive(bool active)
    {
        thumbnailActive = active;
        if (!active) { CancelCardCopy(); CancelContentOpen(); InvalidateThumbnails(); } else QueueThumbnailRefresh();
        UpdateCardCopyButtons();
        UpdateContentOpenControls();
    }
    private bool CanReadThumbnails => !disposed && thumbnailActive && cards && IsLoaded && Visibility == Visibility.Visible
        && core is not null && !loading && !mutating && !filterPending && !externalPending && revision is not null && revision == observedRevision;
    private bool IsThumbnailVisible(HistoryThumbnailView view)
    {
        if (!CanReadThumbnails || !view.IsLoaded || view.Row is not { IsStale: false } row
            || row.Summary.IsSensitive || row.Summary.ContentType != "image" || !rows.Contains(row)
            || view.ActualWidth <= 0 || view.ActualHeight <= 0) return false;
        var point = view.TransformToVisual(HistoryCards).TransformPoint(default);
        return point.Y + view.ActualHeight > 0 && point.Y < HistoryCards.ActualHeight
            && point.X + view.ActualWidth > 0 && point.X < HistoryCards.ActualWidth;
    }
    private void QueueThumbnailRefresh()
    {
        if (disposed || thumbnailRefreshQueued) return;
        thumbnailRefreshQueued = true;
        if (!DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () =>
        {
            thumbnailRefreshQueued = false;
            RefreshThumbnails();
        })) thumbnailRefreshQueued = false;
    }
    private void InvalidateThumbnails()
    {
        foreach (var pair in thumbnails) { pair.Value.Cancellation?.Cancel(); pair.Key.Clear(); }
        thumbnails.Clear();
    }
    private void RefreshThumbnails()
    {
        if (!CanReadThumbnails) { InvalidateThumbnails(); return; }
        var visible = Descendants<HistoryThumbnailView>(HistoryCards).Where(IsThumbnailVisible).ToHashSet();
        foreach (var pair in thumbnails.ToArray())
            if (!visible.Contains(pair.Key) || !ReferenceEquals(pair.Key.Row, pair.Value.Row) || pair.Value.Revision != revision
                || (pair.Value.Cancellation is null && !pair.Key.HasImage && !pair.Key.Unavailable))
            {
                pair.Value.Cancellation?.Cancel(); pair.Key.Clear(); thumbnails.Remove(pair.Key);
            }
        // Cancelled native calls still occupy a slot until their real completion. No queued per-row tasks or cache.
        foreach (var view in visible)
        {
            if (thumbnailRequests >= 2) break;
            if (thumbnails.ContainsKey(view)) continue;
            var request = new ThumbnailRequest(view.Row!, revision!.Value, CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token));
            thumbnails.Add(view, request); view.SetLoading();
            thumbnailRequests++; thumbnailPeakRequests = Math.Max(thumbnailPeakRequests, thumbnailRequests);
            _ = LoadThumbnailAsync(view, request);
        }
    }
    private bool IsCurrentThumbnail(HistoryThumbnailView view, ThumbnailRequest request) =>
        thumbnails.TryGetValue(view, out var current) && ReferenceEquals(current, request)
        && ReferenceEquals(view.Row, request.Row) && revision == request.Revision
        && request.Cancellation?.IsCancellationRequested == false && IsThumbnailVisible(view);
    private async Task LoadThumbnailAsync(HistoryThumbnailView view, ThumbnailRequest request)
    {
        var cancellation = request.Cancellation!;
        try
        {
            var preview = await (ThumbnailReader is { } read ? read(request.Row.Id, request.Revision, cancellation.Token)
                : core!.GetImagePreviewAsync(request.Row.Id, request.Revision, maxEdge: 256, cancellationToken: cancellation.Token));
            if (!IsCurrentThumbnail(view, request)) return;
            if (preview.Id != request.Row.Id || preview.Revision != request.Revision)
                throw new InvalidDataException("Thumbnail identity or revision changed.");
            var bitmap = await NativeImageBitmap.DecodeAsync(preview, cancellation.Token, maxEdge: 256);
            if (IsCurrentThumbnail(view, request)) view.Show(preview.Id, bitmap, preview.AnimationSource);
        }
        catch (OperationCanceledException) { }
        catch (Exception) { if (IsCurrentThumbnail(view, request)) view.Failed(); }
        finally
        {
            request.Cancellation = null; cancellation.Dispose(); thumbnailRequests--;
            QueueThumbnailRefresh();
        }
    }
    private void ReleaseThumbnails()
    {
        HistoryCards.LayoutUpdated -= OnThumbnailLayoutUpdated;
        InvalidateThumbnails(); ThumbnailReader = null;
    }
}
