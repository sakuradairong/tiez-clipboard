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
    private int state;
    internal HistoryRow? Row
    {
        get
        {
            // A recycled card that is being torn down can fail to hand back its DataContext; a thumbnail only
            // falls back to its text then, it never crashes the window.
            try { return DataContext as HistoryRow; }
            catch (Exception) { return null; }
        }
    }    internal long? DisplayedId { get; private set; }
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
        DisplayedId = null; Unavailable = false; state = 0;
        Render();
    }
    internal void SetLoading() { Clear(); state = 1; Render(); }
    internal void Failed() { Clear(); Unavailable = true; state = 2; Render(); }
    internal void Show(long id, BitmapImage bitmap, bool animation)
    {
        image.Source = bitmap; image.Visibility = Visibility.Visible;
        DisplayedId = id; Unavailable = false; state = animation ? 4 : 3;
        Render();
    }
    internal void ApplyLanguage() => Render();
    // Labels are re-derived from the retained state; a decoded bitmap is never dropped by a language switch.
    private void Render()
    {
        var text = PreviewStrings.Current;
        if (state is 3 or 4)
        {
            string label = state == 4 ? text.ThumbnailAnimationLabel : text.ThumbnailStaticLabel;
            AutomationProperties.SetName(image, label); ToolTipService.SetToolTip(this, label);
            fallback.Visibility = Visibility.Collapsed;
            return;
        }
        fallback.Text = state switch
        {
            1 => text.ThumbnailLoading,
            2 => text.ThumbnailFailed,
            _ => Row is { IsStale: false } row && !row.Summary.IsSensitive && row.Summary.ContentType == "image"
                ? text.ThumbnailHint : Row?.Preview ?? ""
        };
        fallback.Visibility = Visibility.Visible;
        AutomationProperties.SetName(image, text.ThumbnailName);
        ToolTipService.SetToolTip(this, null);
    }
}
