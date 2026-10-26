using Microsoft.UI.Xaml.Media.Imaging;
using TieZ.Core;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace TieZ.Native;

internal static class NativeIconBitmap
{
    internal static byte[] ToPremultipliedBgra(ReadOnlySpan<byte> rgba, int width, int height, uint edge)
    {
        if (edge is < 1 or > 128 || width < 1 || height < 1 || width > edge || height > edge
            || (long)width * height * 4 > 65536 || rgba.Length != (long)width * height * 4)
            throw new InvalidDataException("Application icon raster exceeds display bounds.");
        var bgra = new byte[rgba.Length];
        for (int i = 0; i < rgba.Length; i += 4)
        {
            int alpha = rgba[i + 3];
            bgra[i] = (byte)((rgba[i + 2] * alpha + 127) / 255);
            bgra[i + 1] = (byte)((rgba[i + 1] * alpha + 127) / 255);
            bgra[i + 2] = (byte)((rgba[i] * alpha + 127) / 255);
            bgra[i + 3] = (byte)alpha;
        }
        return bgra;
    }

    internal static async Task<SoftwareBitmapSource> DecodeAsync(ApplicationIconRaster raster, CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        byte[] bytes = ToPremultipliedBgra(raster.Pixels.Span, raster.Width, raster.Height, raster.RequestedEdge);
        using var writer = new DataWriter();
        writer.WriteBytes(bytes);
        using var bitmap = SoftwareBitmap.CreateCopyFromBuffer(writer.DetachBuffer(), BitmapPixelFormat.Bgra8,
            raster.Width, raster.Height, BitmapAlphaMode.Premultiplied);
        var source = new SoftwareBitmapSource();
        try
        {
            await source.SetBitmapAsync(bitmap);
            token.ThrowIfCancellationRequested();
            return source;
        }
        catch { source.Dispose(); throw; }
    }
}
