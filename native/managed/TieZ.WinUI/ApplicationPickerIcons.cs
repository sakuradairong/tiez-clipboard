using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using TieZ.Core;

namespace TieZ.Native;

public sealed partial class ApplicationPickerView
{
    private sealed class IconRequest(ApplicationChoice choice, ulong catalog, uint edge, int generation,
        CancellationTokenSource cancellation)
    {
        internal readonly ApplicationChoice Choice = choice;
        internal readonly ulong Catalog = catalog;
        internal readonly uint Edge = edge;
        internal readonly int Generation = generation;
        internal CancellationTokenSource? Cancellation = cancellation;
    }
    private readonly Dictionary<ApplicationIconView, IconRequest> iconTargets = [];
    private int iconRequests, iconPeakRequests;
    private bool iconRefreshQueued;
    private XamlRoot? iconRoot;
    internal Func<ulong, InstalledApplication, uint, CancellationToken, Task<ApplicationIconRaster>>? IconReader { get; set; }
    internal int IconRequests => iconRequests;
    internal int IconPeakRequests => iconPeakRequests;
    internal int IconResources => iconTargets.Keys.Count(view => view.HasImage);
    internal bool HasIconState => IconReader is not null || iconTargets.Count > 0 || iconRequests > 0;
    internal ApplicationIconView[] IconViews => IconDescendants(Choices).ToArray();
    private bool CanReadIcons => !disposed && !stale && !loading && accepted is not null && IconReader is not null
        && lifetime is { IsCancellationRequested: false } && IsLoaded && Visibility == Visibility.Visible
        && XamlRoot?.IsHostVisible == true;
    private uint IconEdge => (uint)Math.Clamp(Math.Ceiling(24 * (XamlRoot?.RasterizationScale ?? 1)), 1, 128);

    private static IEnumerable<ApplicationIconView> IconDescendants(DependencyObject parent)
    {
        if (parent is ApplicationIconView icon) yield return icon;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
            foreach (var child in IconDescendants(VisualTreeHelper.GetChild(parent, i))) yield return child;
    }
    private void InitializeIcons()
    {
        Choices.LayoutUpdated += OnIconLayoutUpdated;
        Loaded += OnIconsLoaded; Unloaded += OnIconsUnloaded;
    }
    private void OnIconLayoutUpdated(object? sender, object args) => QueueIconRefresh();
    private void OnIconsLoaded(object sender, RoutedEventArgs args)
    {
        DetachIconRoot(); iconRoot = XamlRoot;
        if (iconRoot is not null) iconRoot.Changed += OnIconRootChanged;
        QueueIconRefresh();
    }
    private void OnIconsUnloaded(object sender, RoutedEventArgs args) { DetachIconRoot(); ClearIcons(); }
    private void OnIconRootChanged(XamlRoot sender, XamlRootChangedEventArgs args) => QueueIconRefresh();
    private void DetachIconRoot()
    {
        if (iconRoot is not null) iconRoot.Changed -= OnIconRootChanged;
        iconRoot = null;
    }
    private bool IsIconVisible(ApplicationIconView view)
    {
        if (!CanReadIcons || !view.IsLoaded || view.Choice is not { CanSelect: true, Application: not null } choice
            || !visible.Contains(choice) || view.ActualHeight <= 0 || view.ActualWidth <= 0) return false;
        var point = view.TransformToVisual(Choices).TransformPoint(default);
        return point.Y + view.ActualHeight > 0 && point.Y < Choices.ActualHeight
            && point.X + view.ActualWidth > 0 && point.X < Choices.ActualWidth;
    }
    private void QueueIconRefresh()
    {
        if (disposed || iconRefreshQueued) return;
        iconRefreshQueued = true;
        if (!DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () =>
        {
            iconRefreshQueued = false;
            RefreshIcons();
        })) iconRefreshQueued = false;
    }
    private void ClearIcons()
    {
        foreach (var pair in iconTargets) { pair.Value.Cancellation?.Cancel(); pair.Key.Clear(); }
        iconTargets.Clear();
        // Cancelled reads/decodes continue to occupy a slot until their real finally completes.
    }
    private void RefreshIcons()
    {
        if (!CanReadIcons) { ClearIcons(); return; }
        var views = IconViews.Where(IsIconVisible).ToHashSet();
        foreach (var pair in iconTargets.ToArray())
            if (!views.Contains(pair.Key) || !ReferenceEquals(pair.Key.Choice, pair.Value.Choice)
                || pair.Value.Catalog != accepted!.JobId || pair.Value.Generation != generation || pair.Value.Edge != IconEdge
                || pair.Value.Cancellation is null && !pair.Key.HasImage && !pair.Key.Unavailable)
            {
                pair.Value.Cancellation?.Cancel(); pair.Key.Clear(); iconTargets.Remove(pair.Key);
            }
        foreach (var view in views)
        {
            if (iconRequests >= 2) break;
            if (iconTargets.ContainsKey(view)) continue;
            var request = new IconRequest(view.Choice!, accepted!.JobId, IconEdge, generation,
                CancellationTokenSource.CreateLinkedTokenSource(lifetime!.Token));
            iconTargets.Add(view, request); view.SetLoading();
            iconRequests++; iconPeakRequests = Math.Max(iconPeakRequests, iconRequests);
            _ = LoadIconAsync(view, request);
        }
    }
    private bool IsCurrentIcon(ApplicationIconView view, IconRequest request) =>
        iconTargets.TryGetValue(view, out var current) && ReferenceEquals(current, request)
        && ReferenceEquals(view.Choice, request.Choice) && request.Catalog == accepted?.JobId
        && request.Generation == generation && request.Edge == IconEdge
        && request.Cancellation?.IsCancellationRequested == false && IsIconVisible(view);

    private async Task LoadIconAsync(ApplicationIconView view, IconRequest request)
    {
        var cancellation = request.Cancellation!;
        SoftwareBitmapSource? source = null;
        try
        {
            var app = request.Choice.Application!;
            var raster = await IconReader!(request.Catalog, app, request.Edge, cancellation.Token);
            if (!IsCurrentIcon(view, request)) return;
            var identity = new ApplicationIconIdentity(app.SourceId, app.Handler, app.Kind, app.Unavailable);
            if (raster.JobId == 0 || raster.CatalogJobId != request.Catalog || raster.Identity != identity
                || raster.RequestedEdge != request.Edge)
                throw new InvalidDataException("Application icon identity or generation changed.");
            source = await NativeIconBitmap.DecodeAsync(raster, cancellation.Token);
            if (IsCurrentIcon(view, request)) { view.Show(raster, source); source = null; }
        }
        catch (OperationCanceledException) { }
        catch (Exception) { if (IsCurrentIcon(view, request)) view.Failed(); }
        finally
        {
            source?.Dispose(); request.Cancellation = null; cancellation.Dispose(); iconRequests--;
            QueueIconRefresh();
        }
    }
    private void ReleaseIcons()
    {
        Choices.LayoutUpdated -= OnIconLayoutUpdated;
        Loaded -= OnIconsLoaded; Unloaded -= OnIconsUnloaded;
        DetachIconRoot(); ClearIcons(); IconReader = null;
    }
}
