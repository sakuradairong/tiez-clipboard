using TieZ.Core;

namespace TieZ.Native;

public sealed class HistoryRow(HistorySummary summary, bool isStale = false)
{
    public HistorySummary Summary { get; } = summary;
    public long Id => Summary.Id;
    public bool IsStale { get; } = isStale;
    public string Preview => IsStale ? "记录已更新 · 请刷新" : Summary.IsSensitive ? "敏感内容 · 选择后可显示" : Summary.Preview;
    public string Metadata => string.Join(" · ", new[] {
        Summary.IsPinned ? "置顶" : null,
        Summary.ContentType,
        Summary.SourceApp,
        $"使用 {Summary.UseCount} 次",
        Summary.Id < 0 ? "仅本次会话" : null,
        Summary.Tags.Count > 0 ? string.Join(" / ", Summary.Tags) : null
    }.Where(value => !string.IsNullOrEmpty(value)));
}
