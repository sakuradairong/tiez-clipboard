namespace TieZ.Native;

internal static class PopupStrings
{
    internal static string Text(string zh, string tw, string en) => LanguageService.Current switch
    { Language.English => en, Language.TraditionalChinese => tw, _ => zh };
    internal static string Keep => Text("保持窗口打开", "保持視窗開啟", "Keep window open");
    internal static string More => Text("更多操作", "更多操作", "More actions");
    internal static string Manage => Text("历史管理", "歷史管理", "Manage history");
    internal static string Hide => Text("收起 · Esc", "收起 · Esc", "Hide · Esc");
    internal static string Search => Text("搜索剪贴板…", "搜尋剪貼簿…", "Search clipboard…");
    internal static string All => Text("全部", "全部", "All");
    internal static string Pinned => Text("置顶", "置頂", "Pinned");
    internal static string Types => Text("所有类型", "所有類型", "All types");
    internal static string Preview => Text("预览", "預覽", "Preview");
    internal static string Original => Text("保留格式粘贴 · 右键", "保留格式貼上 · 右鍵", "Paste original · Right-click");
    internal static string Hint => Text("↑↓ 选择  ·  Enter 粘贴  ·  Esc 收起", "↑↓ 選擇  ·  Enter 貼上  ·  Esc 收起", "↑↓ Select · Enter Paste · Esc Hide");
    internal static string NewItems => Text("刷新记录", "重新整理記錄", "Refresh items");
    internal static string Sensitive => Text("此条目包含敏感内容。是否允许本次操作？", "此項目包含敏感內容。是否允許本次操作？", "This item contains sensitive content. Allow this action?");
    internal static string NoTarget => Text("没有可用的粘贴目标。请回到目标应用，再用快捷键唤出。也可从条目菜单选择仅复制。", "沒有可用的貼上目標。請回到目標應用程式，再用快速鍵叫出。也可從項目選單選擇僅複製。", "No paste target. Return to the target app and use the hotkey, or choose Copy from the item menu.");
    internal static string Changed => Text("记录已变化，请刷新后重试。", "記錄已變更，請重新整理後再試。", "History changed. Refresh and try again.");
    internal static string Failed => Text("粘贴未完成。请回到目标应用后重新唤出。", "貼上未完成。請回到目標應用程式後重新叫出。", "Paste did not complete. Return to the target app and reopen.");
    internal static string NotPinned => Text("这个位置没有置顶条目。", "這個位置沒有置頂項目。", "That position has no pinned item.");
    internal static string Paused => Text("采集已暂停", "擷取已暫停", "Capture paused");
    internal static string Sent => Text("已提交粘贴请求", "已提交貼上請求", "Paste request sent");
}
