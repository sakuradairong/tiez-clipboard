using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics.Imaging;
using Windows.Storage;
using Windows.Storage.Streams;

namespace TieZ.Native;

internal static class NativeSmokeCapture
{
    // XAML-tree evidence only, not a full compositor or physical-display screenshot.
    internal static async Task<string> CaptureAsync(FrameworkElement element, string report)
    {
        double scale = element.XamlRoot?.RasterizationScale ?? 1;
        double width = element.ActualWidth * scale, height = element.ActualHeight * scale;
        if (width <= 0 || height <= 0) throw new InvalidOperationException("Native screenshot target has no layout.");
        double ratio = Math.Min(1, 2048 / Math.Max(width, height));
        var bitmap = new RenderTargetBitmap();
        await bitmap.RenderAsync(element, Math.Max(1, (int)(width * ratio)), Math.Max(1, (int)(height * ratio)));
        if (bitmap.PixelWidth <= 0 || bitmap.PixelHeight <= 0) throw new InvalidOperationException("Native screenshot was empty.");
        var buffer = await bitmap.GetPixelsAsync();
        using var reader = DataReader.FromBuffer(buffer);
        byte[] bytes = new byte[checked((int)buffer.Length)]; reader.ReadBytes(bytes);
        var folder = await StorageFolder.GetFolderFromPathAsync(Path.GetDirectoryName(Path.GetFullPath(report))!);
        var file = await folder.CreateFileAsync(Path.GetFileName(report) + ".png", CreationCollisionOption.GenerateUniqueName);
        using var stream = await file.OpenAsync(FileAccessMode.ReadWrite);
        var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, stream);
        encoder.SetPixelData(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied, (uint)bitmap.PixelWidth,
            (uint)bitmap.PixelHeight, 96 * scale, 96 * scale, bytes);
        await encoder.FlushAsync(); return file.Path;
    }
}
