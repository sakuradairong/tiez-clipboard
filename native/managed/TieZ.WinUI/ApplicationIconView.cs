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
    private enum IconState { Idle, Loading, Failed, Shown }
    private readonly Image image = new() { Stretch = Stretch.Uniform };
    private readonly SymbolIcon fallback = new(Symbol.AllApps);
    private SoftwareBitmapSource? source;
    private IconState state = IconState.Idle;
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
        state = IconState.Idle;
        ApplyLanguage();
    }
    internal void SetLoading() { Clear(); state = IconState.Loading; ApplyLanguage(); }
    internal void Failed()
    {
        Clear(); Unavailable = true; state = IconState.Failed; ApplyLanguage();
    }
    internal void Show(ApplicationIconRaster raster, SoftwareBitmapSource bitmap)
    {
        Clear(); source = bitmap; image.Source = bitmap; image.Visibility = Visibility.Visible;
        fallback.Visibility = Visibility.Collapsed;
        DisplayedIdentity = raster.Identity; DisplayedCatalog = raster.CatalogJobId;
        state = IconState.Shown; ApplyLanguage();
    }
    // Only the automation name and tooltip are chrome; the decoded bitmap is never touched by a language change.
    internal void ApplyLanguage()
    {
        var text = AppPickerStrings.Current;
        string? tooltip = null;
        string name = text.IconName;
        if (state == IconState.Loading) name = text.IconLoading;
        else if (state == IconState.Failed) tooltip = name = text.IconUnavailable;
        AutomationProperties.SetName(this, name);
        ToolTipService.SetToolTip(this, tooltip);
    }
}
