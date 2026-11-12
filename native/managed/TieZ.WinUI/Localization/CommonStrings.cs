namespace TieZ.Native;

/// <summary>
/// Strings shared by more than one native surface. Surface-specific text lives in its own table next to the
/// view that renders it; add a property here only when two surfaces genuinely need the same sentence.
/// </summary>
internal sealed class CommonStrings
{
    internal required string Save { get; init; }
    internal required string Cancel { get; init; }
    internal required string Close { get; init; }
    internal required string Delete { get; init; }
    internal required string Clear { get; init; }
    internal required string Add { get; init; }
    internal required string Edit { get; init; }
    internal required string Copy { get; init; }
    internal required string CopyPlainText { get; init; }
    internal required string Open { get; init; }
    internal required string Pin { get; init; }
    internal required string Tags { get; init; }
    internal required string Paste { get; init; }
    internal required string PastePlainText { get; init; }
    internal required string Refresh { get; init; }
    internal required string Reload { get; init; }
    internal required string Retry { get; init; }
    internal required string Search { get; init; }
    internal required string Confirm { get; init; }
    internal required string Discard { get; init; }
    internal required string Loading { get; init; }
    internal required string LoadMore { get; init; }
    internal required string Error { get; init; }
    internal required string Back { get; init; }
    internal required string Browse { get; init; }
    internal required string Unavailable { get; init; }
    internal required string Default { get; init; }
    internal required string Recommended { get; init; }
    internal required string Settings { get; init; }
    internal required string Clipboard { get; init; }
    internal required string On { get; init; }
    internal required string Off { get; init; }

    internal static CommonStrings Zh { get; } = new()
    {
        Save = "保存",
        Cancel = "取消",
        Close = "关闭",
        Delete = "删除",
        Clear = "清空",
        Add = "新增",
        Edit = "编辑",
        Copy = "复制",
        CopyPlainText = "复制为纯文本",
        Open = "打开",
        Pin = "置顶",
        Tags = "标签",
        Paste = "粘贴",
        PastePlainText = "粘贴纯文本",
        Refresh = "刷新",
        Reload = "重新加载",
        Retry = "重试",
        Search = "搜索",
        Confirm = "确认",
        Discard = "放弃",
        Loading = "正在加载…",
        LoadMore = "加载更多",
        Error = "操作未完成",
        Back = "返回",
        Browse = "浏览…",
        Unavailable = "不可用",
        Default = "默认",
        Recommended = "推荐",
        Settings = "设置",
        Clipboard = "剪贴板",
        On = "开",
        Off = "关",
    };

    internal static CommonStrings Tw { get; } = new()
    {
        Save = "儲存",
        Cancel = "取消",
        Close = "關閉",
        Delete = "刪除",
        Clear = "清空",
        Add = "新增",
        Edit = "編輯",
        Copy = "複製",
        CopyPlainText = "複製為純文字",
        Open = "開啟",
        Pin = "置頂",
        Tags = "標籤",
        Paste = "貼上",
        PastePlainText = "貼上純文字",
        Refresh = "重新整理",
        Reload = "重新載入",
        Retry = "重試",
        Search = "搜尋",
        Confirm = "確認",
        Discard = "放棄",
        Loading = "正在載入…",
        LoadMore = "載入更多",
        Error = "操作未完成",
        Back = "返回",
        Browse = "瀏覽…",
        Unavailable = "無法使用",
        Default = "預設",
        Recommended = "建議",
        Settings = "設定",
        Clipboard = "剪貼簿",
        On = "開",
        Off = "關",
    };

    internal static CommonStrings En { get; } = new()
    {
        Save = "Save",
        Cancel = "Cancel",
        Close = "Close",
        Delete = "Delete",
        Clear = "Clear",
        Add = "Add",
        Edit = "Edit",
        Copy = "Copy",
        CopyPlainText = "Copy as plain text",
        Open = "Open",
        Pin = "Pin",
        Tags = "Tags",
        Paste = "Paste",
        PastePlainText = "Paste as plain text",
        Refresh = "Refresh",
        Reload = "Reload",
        Retry = "Retry",
        Search = "Search",
        Confirm = "Confirm",
        Discard = "Discard",
        Loading = "Loading...",
        LoadMore = "Load more",
        Error = "The operation did not complete",
        Back = "Back",
        Browse = "Browse...",
        Unavailable = "Unavailable",
        Default = "Default",
        Recommended = "Recommended",
        Settings = "Settings",
        Clipboard = "Clipboard",
        On = "On",
        Off = "Off",
    };

    internal static CommonStrings Current => LanguageService.Current switch
    {
        Language.TraditionalChinese => Tw,
        Language.English => En,
        _ => Zh
    };
}