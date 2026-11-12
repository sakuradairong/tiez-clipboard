using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using TieZ.Core;

namespace TieZ.Native;

// Only a validated, bounded PNG from Rust reaches the native decoder. No paths or URLs.
public sealed class ImagePreviewView : Grid
{
    private readonly Image image = new() { Stretch = Stretch.Uniform };
    private readonly TextBlock caption = new() { TextWrapping = TextWrapping.Wrap, Opacity = 0.7 };
    private readonly ScrollViewer viewport = new() { ZoomMode = ZoomMode.Enabled, MinZoomFactor = 1,
        MaxZoomFactor = 8, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    private int generation;
    private ImagePreviewData? shown;
    internal long? DisplayedId { get; private set; }
    internal bool HasImage => image.Source is BitmapImage { PixelWidth: > 0 } && Visibility == Visibility.Visible;
    internal int PixelWidth => (image.Source as BitmapImage)?.PixelWidth ?? 0;

    public ImagePreviewView()
    {
        RowSpacing = 8;
        RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        viewport.Content = image;
        Children.Add(viewport); Children.Add(caption); SetRow(caption, 1);
        Visibility = Visibility.Collapsed;
        RenderChrome();
    }
    internal void Clear()
    {
        generation++;
        image.Source = null; shown = null; caption.Text = ""; DisplayedId = null;
        Visibility = Visibility.Collapsed;
        viewport.ChangeView(null, null, 1, true);
    }
    internal async Task ShowAsync(ImagePreviewData preview, CancellationToken token)
    {
        Clear();
        int current = generation;
        var bitmap = await NativeImageBitmap.DecodeAsync(preview, token);
        if (current != generation) return;
        image.Source = bitmap;
        shown = preview;
        DisplayedId = preview.Id; Visibility = Visibility.Visible;
        RenderChrome();
    }
    internal void ApplyLanguage() => RenderChrome();
    // The decoded bitmap is kept; only the caption and automation name follow the active language.
    private void RenderChrome()
    {
        var text = PreviewStrings.Current;
        AutomationProperties.SetName(image, text.ImageName);
        if (shown is not { } preview) { caption.Text = ""; return; }
        caption.Text = string.Format(text.ImageCaptionFormat, preview.Width, preview.Height,
            preview.OriginalWidth, preview.OriginalHeight)
            + (preview.AnimationSource ? "\n" + text.ImageAnimationNote : "");
    }
}
