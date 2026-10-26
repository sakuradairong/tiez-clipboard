using TieZ.Core;

namespace TieZ.Native;

public sealed class HistoryRow(HistorySummary summary, bool isStale = false)
{
    public HistorySummary Summary { get; } = summary;
    public long Id => Summary.Id;
    public bool IsStale { get; } = isStale;
    public string TypeLabel => Summary.ContentType switch {
        "text" => "文本", "code" => "代码", "url" or "link" => "链接", "rich_text" => "富文本",
        "image" => "图片", "file" => "文件", "video" => "视频", _ => Summary.ContentType
    };
    public string SourceLabel => Summary.SourceApp == "Manual" ? "手动添加" : Summary.SourceApp;
    public string Preview => IsStale ? "记录已更新 · 请刷新" : Summary.IsSensitive ? "敏感内容 · 选择后可显示"
        : Summary.ContentType == "image" && Summary.Preview.StartsWith("data:image/", StringComparison.OrdinalIgnoreCase)
            ? "图片 · 选择后查看预览" : Summary.Preview;
    public string Metadata => string.Join(" · ", new[] {
        Summary.IsPinned ? "置顶" : null,
        TypeLabel,
        SourceLabel,
        $"已使用 {Summary.UseCount} 次",
        Summary.Id < 0 ? "仅本次会话" : null,
        Summary.Tags.Count > 0 ? string.Join(" / ", Summary.Tags) : null
    }.Where(value => !string.IsNullOrEmpty(value)));
}
