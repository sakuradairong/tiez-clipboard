using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using TieZ.Core;

namespace TieZ.Native;

// The realized container, not the catalog row, owns the native bitmap.
public sealed class ApplicationIconView : Grid
{
    private readonly Image image = new() { Stretch = Stretch.Uniform };
    private readonly SymbolIcon fallback = new(Symbol.AllApps);
    private SoftwareBitmapSource? source;
    internal ApplicationChoice? Choice => DataContext as ApplicationChoice;
    internal ApplicationIconIdentity? DisplayedIdentity { get; private set; }
    internal ulong? DisplayedCatalog { get; private set; }
    internal bool HasImage => image.Source is SoftwareBitmapSource && image.Visibility == Visibility.Visible;
    internal bool Unavailable { get; private set; }
    internal Image RenderedImage => image;

    public ApplicationIconView()
    {
        Width = Height = 24;
        Children.Add(image); Children.Add(fallback);
        DataContextChanged += (_, _) => Clear();
        Unloaded += (_, _) => Clear();
        Clear();
    }
    internal void Clear()
    {
        image.Source = null; image.Visibility = Visibility.Collapsed;
        source?.Dispose(); source = null;
        DisplayedIdentity = null; DisplayedCatalog = null; Unavailable = false;
        fallback.Visibility = Visibility.Visible;
        ToolTipService.SetToolTip(this, null);
        AutomationProperties.SetName(this, "应用图标");
    }
    internal void SetLoading() { Clear(); AutomationProperties.SetName(this, "正在加载应用图标"); }
    internal void Failed()
    {
        Clear(); Unavailable = true;
        AutomationProperties.SetName(this, "图标不可用，不影响选择应用");
        ToolTipService.SetToolTip(this, "图标不可用，不影响选择应用");
    }
    internal void Show(ApplicationIconRaster raster, SoftwareBitmapSource bitmap)
    {
        Clear(); source = bitmap; image.Source = bitmap; image.Visibility = Visibility.Visible;
        fallback.Visibility = Visibility.Collapsed;
        DisplayedIdentity = raster.Identity; DisplayedCatalog = raster.CatalogJobId;
        AutomationProperties.SetName(this, "应用图标");
    }
}
