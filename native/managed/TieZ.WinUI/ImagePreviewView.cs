using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using TieZ.Core;
using Windows.Storage.Streams;

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
    internal long? DisplayedId { get; private set; }
    internal bool HasImage => image.Source is BitmapImage { PixelWidth: > 0 } && Visibility == Visibility.Visible;
    internal int PixelWidth => (image.Source as BitmapImage)?.PixelWidth ?? 0;

    public ImagePreviewView()
    {
        RowSpacing = 8;
        RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        AutomationProperties.SetName(image, "所选图片的静态预览");
        viewport.Content = image;
        Children.Add(viewport); Children.Add(caption); SetRow(caption, 1);
        Visibility = Visibility.Collapsed;
    }
    internal void Clear()
    {
        generation++;
        image.Source = null; caption.Text = ""; DisplayedId = null;
        Visibility = Visibility.Collapsed;
        viewport.ChangeView(null, null, 1, true);
    }
    internal async Task ShowAsync(ImagePreviewData preview, CancellationToken token)
    {
        Clear();
        int current = generation;
        token.ThrowIfCancellationRequested();
        if (preview.Width is < 1 or > 1024 || preview.Height is < 1 or > 1024
            || preview.PngBase64.Length > 7 * 1024 * 1024)
            throw new InvalidDataException("Native image preview exceeds its display bounds.");
        byte[] bytes = Convert.FromBase64String(preview.PngBase64);
        using var stream = new InMemoryRandomAccessStream();
        using (var writer = new DataWriter(stream))
        {
            writer.WriteBytes(bytes);
            await writer.StoreAsync();
            writer.DetachStream();
        }
        token.ThrowIfCancellationRequested();
        stream.Seek(0);
        var bitmap = new BitmapImage();
        await bitmap.SetSourceAsync(stream);
        token.ThrowIfCancellationRequested();
        if (current != generation) return;
        if (bitmap.PixelWidth != preview.Width || bitmap.PixelHeight != preview.Height)
            throw new InvalidDataException("Native image dimensions do not match the preview.");
        image.Source = bitmap;
        caption.Text = $"静态预览 {preview.Width} × {preview.Height} · 原图 {preview.OriginalWidth} × {preview.OriginalHeight}"
            + (preview.AnimationSource ? "\n动画显示首帧，原始动画保留用于复制和粘贴。" : "");
        DisplayedId = preview.Id; Visibility = Visibility.Visible;
    }
}
