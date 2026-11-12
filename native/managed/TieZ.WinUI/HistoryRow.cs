using System.ComponentModel;
using TieZ.Core;

namespace TieZ.Native;

public sealed class HistoryRow(HistorySummary summary, bool isStale = false) : INotifyPropertyChanged
{
    public HistorySummary Summary { get; } = summary;
    public long Id => Summary.Id;
    public bool IsStale { get; } = isStale;
    public event PropertyChangedEventHandler? PropertyChanged;
    public string TypeLabel => TypeLabelFor(Summary.ContentType);
    public string SourceLabel => Summary.SourceApp == "Manual" ? HistoryStrings.Current.SourceManual : Summary.SourceApp;
    public string Preview => IsStale ? HistoryStrings.Current.PreviewStale
        : Summary.IsSensitive ? HistoryStrings.Current.PreviewSensitive
        : Summary.ContentType == "image" && Summary.Preview.StartsWith("data:image/", StringComparison.OrdinalIgnoreCase)
            ? HistoryStrings.Current.PreviewImageHint : Summary.Preview;
    public string Metadata => string.Join(HistoryStrings.Current.MetadataSeparator, new[] {
        Summary.IsPinned ? HistoryStrings.Current.MetadataPinned : null,
        TypeLabel,
        SourceLabel,
        string.Format(HistoryStrings.Current.MetadataUsageFormat, Summary.UseCount),
        Summary.Id < 0 ? HistoryStrings.Current.MetadataSessionOnly : null,
        Summary.Tags.Count > 0 ? string.Join(" / ", Summary.Tags) : null
    }.Where(value => !string.IsNullOrEmpty(value)));

    /// <summary>The localized label of a core content type; unknown codes stay verbatim.</summary>
    public static string TypeLabelFor(string contentType) => contentType switch {
        "text" => HistoryStrings.Current.TypeText, "code" => HistoryStrings.Current.TypeCode,
        "url" or "link" => HistoryStrings.Current.TypeLink, "rich_text" => HistoryStrings.Current.TypeRichText,
        "image" => HistoryStrings.Current.TypeImage, "file" => HistoryStrings.Current.TypeFile,
        "video" => HistoryStrings.Current.TypeVideo, _ => contentType
    };

    /// <summary>Re-derives the localized labels in place so a live language switch keeps every container.</summary>
    internal void RefreshText()
    {
        PropertyChanged?.Invoke(this, new(nameof(TypeLabel)));
        PropertyChanged?.Invoke(this, new(nameof(SourceLabel)));
        PropertyChanged?.Invoke(this, new(nameof(Preview)));
        PropertyChanged?.Invoke(this, new(nameof(Metadata)));
    }
}
