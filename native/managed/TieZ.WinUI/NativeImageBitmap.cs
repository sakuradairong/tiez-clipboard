using Microsoft.UI.Xaml.Media.Imaging;
using TieZ.Core;
using Windows.Storage.Streams;

namespace TieZ.Native;

// The only decoder input is a bounded Rust raster; never open a path/URL in XAML.
internal static class NativeImageBitmap
{
    internal static async Task<BitmapImage> DecodeAsync(ImagePreviewData preview, CancellationToken token, int maxEdge = 1024)
    {
        token.ThrowIfCancellationRequested();
        if (preview.Width < 1 || preview.Width > maxEdge || preview.Height < 1 || preview.Height > maxEdge
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
        if (bitmap.PixelWidth != preview.Width || bitmap.PixelHeight != preview.Height)
            throw new InvalidDataException("Native image dimensions do not match the preview.");
        return bitmap;
    }
}
