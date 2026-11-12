namespace TieZ.Native;

internal sealed record WorkspaceStrings(string Filters, string EmptyTitle, string EmptyHint, string NoResults,
    string NoResultsHint, string Appearance, string Capture, string Storage, string SettingsHint)
{
    internal static WorkspaceStrings Current => LanguageService.Current switch
    {
        Language.English => En,
        Language.TraditionalChinese => Tw,
        _ => Zh
    };
    internal static WorkspaceStrings Zh { get; } = new("筛选与视图", "复制的内容，都在这里", "复制文字、图片或文件，即可开始积累历史。也可以点击新增，保存常用内容。",
        "没有匹配的记录", "试试其他关键词，或调整类型和标签筛选。", "外观与操作", "剪贴板采集", "历史与存储", "按你的习惯调整，保存后立即生效。");
    internal static WorkspaceStrings Tw { get; } = new("篩選與檢視", "複製的內容，都在這裡", "複製文字、圖片或檔案，即可開始累積歷史。也可以點選新增，儲存常用內容。",
        "沒有符合的記錄", "試試其他關鍵字，或調整類型與標籤篩選。", "外觀與操作", "剪貼簿擷取", "歷史與儲存", "依照你的習慣調整，儲存後立即生效。");
    internal static WorkspaceStrings En { get; } = new("Filters and view", "Everything you copy, in one place", "Copy text, images or files to start your history. Use Add to keep something handy.",
        "No matching items", "Try another search or adjust the type and tag filters.", "Appearance and controls", "Clipboard capture", "History and storage", "Make it yours. Changes take effect when saved.");
}
