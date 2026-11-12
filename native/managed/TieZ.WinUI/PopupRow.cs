using System.ComponentModel;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media.Imaging;
using TieZ.Core;

namespace TieZ.Native;

public sealed class PopupRow(HistorySummary summary, ulong revision) : INotifyPropertyChanged
{
    public HistorySummary Summary { get; } = summary;
    public ulong Revision { get; } = revision;
    public long Id => Summary.Id;
    public string Preview => new HistoryRow(Summary).Preview;
    public string PreviewLabel => PopupStrings.Preview;
    public string MoreLabel => PopupStrings.More;
    public string ActionName => $"{CommonStrings.Current.Paste}: {Preview}";
    public override string ToString() => Preview;
    public string Metadata => string.Join(" · ", new[] {
        Summary.IsPinned ? PopupStrings.Pinned : null,
        HistoryRow.TypeLabelFor(Summary.ContentType),
        new HistoryRow(Summary).SourceLabel,
        DateTimeOffset.FromUnixTimeMilliseconds(Summary.Timestamp).ToLocalTime().ToString("MM-dd HH:mm")
    }.Where(part => !string.IsNullOrEmpty(part)));
    public BitmapImage? Thumbnail { get; private set; }
    public Visibility ThumbnailVisibility => Thumbnail is null ? Visibility.Collapsed : Visibility.Visible;
    internal bool ThumbnailRequested { get; set; }
    internal CancellationTokenSource? ThumbnailRequest { get; set; }
    public event PropertyChangedEventHandler? PropertyChanged;
    internal void SetThumbnail(BitmapImage? value)
    {
        if (value is null)
        {
            ThumbnailRequest?.Cancel(); ThumbnailRequest?.Dispose(); ThumbnailRequest = null;
            ThumbnailRequested = false;
        }
        Thumbnail = value;
        PropertyChanged?.Invoke(this, new(nameof(Thumbnail)));
        PropertyChanged?.Invoke(this, new(nameof(ThumbnailVisibility)));
    }
}
