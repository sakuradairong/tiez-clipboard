using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;

namespace TieZ.Native;

// A recycled template owns only its current native bitmap, never a core or a reader.
public sealed class HistoryThumbnailView : Grid
{
    private readonly Image image = new() { Stretch = Stretch.Uniform };
    private readonly TextBlock fallback = new() { TextWrapping = TextWrapping.Wrap, MaxLines = 4,
        TextTrimming = TextTrimming.CharacterEllipsis };
    internal HistoryRow? Row => DataContext as HistoryRow;
    internal long? DisplayedId { get; private set; }
    internal bool HasImage => image.Source is BitmapImage { PixelWidth: > 0 } && image.Visibility == Visibility.Visible;
    internal bool Unavailable { get; private set; }
    public HistoryThumbnailView()
    {
        Children.Add(image); Children.Add(fallback);
        DataContextChanged += (_, _) => Clear();
        Unloaded += (_, _) => Clear();
        Clear();
    }
    internal void Clear()
    {
        image.Source = null; image.Visibility = Visibility.Collapsed;
        DisplayedId = null; Unavailable = false;
        fallback.Text = Row is { IsStale: false } row && !row.Summary.IsSensitive && row.Summary.ContentType == "image"
            ? "图片 · 选择后查看详情" : Row?.Preview ?? "";
        fallback.Visibility = Visibility.Visible;
        AutomationProperties.SetName(image, "图片缩略图");
        ToolTipService.SetToolTip(this, null);
    }
    internal void SetLoading() { Clear(); fallback.Text = "正在加载图片预览"; }
    internal void Failed() { Clear(); Unavailable = true; fallback.Text = "图片预览不可用 · 选择后查看详情"; }
    internal void Show(long id, BitmapImage bitmap, bool animation)
    {
        image.Source = bitmap; image.Visibility = Visibility.Visible;
        fallback.Visibility = Visibility.Collapsed; DisplayedId = id; Unavailable = false;
        string label = animation ? "图片缩略图 · 动画显示首帧，原始动画保留" : "图片缩略图 · 原始图片保留";
        AutomationProperties.SetName(image, label); ToolTipService.SetToolTip(this, label);
    }
}
