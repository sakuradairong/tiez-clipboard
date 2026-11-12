namespace TieZ.Native;

internal static class QueueStrings
{
    private static string T(string zh, string tw, string en) => PopupStrings.Text(zh, tw, en);
    internal static string Title => T("顺序粘贴队列", "循序貼上佇列", "Sequential paste queue");
    internal static string Replace => T("按列表顺序设为队列", "依清單順序設為佇列", "Replace queue with selection");
    internal static string Manage => T("管理粘贴队列", "管理貼上佇列", "Manage paste queue");
    internal static string Hint => T("按显示顺序逐条粘贴；可调整顺序、移除或清空。编号是历史记录 ID，不显示敏感正文。", "依顯示順序逐筆貼上；可調整順序、移除或清空。編號為歷史記錄 ID，不顯示敏感內文。", "Paste in displayed order; reorder, remove or clear items. Numbers are history IDs; sensitive content stays hidden.");
    internal static string Up => T("上移", "上移", "Move up");
    internal static string Down => T("下移", "下移", "Move down");
    internal static string Remove => T("移除", "移除", "Remove");
    internal static string Clear => T("清空队列", "清空佇列", "Clear queue");
    internal static string Rebuild => T("确认重新建立队列", "確認重新建立佇列", "Rebuild queue explicitly");
    internal static string Next => T("粘贴下一条", "貼上下一筆", "Paste next");
    internal static string Mode => T("顺序采集并启用队列快捷键", "循序擷取並啟用佇列快速鍵", "Collect sequentially and enable queue shortcut");
    internal static string Hotkey => T("队列下一条快捷键（留空禁用）", "佇列下一筆快速鍵（留空停用）", "Queue next shortcut (empty disables)");
    internal static string ShortcutHint => T("默认 Alt+V；松开按键后粘贴。关闭顺序模式会取消全局绑定，不会删除当前队列。", "預設 Alt+V；放開按鍵後貼上。關閉循序模式會取消全域綁定，不會刪除目前佇列。", "Default Alt+V; release keys to paste. Disabling sequential mode releases the shortcut, not the current queue.");
    internal static string ReleaseKeys => T("请松开按键后重试，队列仍保留。", "請放開按鍵後重試，佇列仍保留。", "Release the keys and retry; the queue is retained.");
    internal static string Empty => T("队列为空", "佇列為空", "Queue is empty");
    internal static string ResetRequired => T("历史版本已变化，无法确认条目身份。请从历史重新选择后建立队列。", "歷史版本已變更，無法確認項目身分。請從歷史重新選擇後建立佇列。", "History identity evidence expired. Select records in history and rebuild the queue.");
    internal static string Full => T("队列已满（1000 条）；新采集仍保存到历史。", "佇列已滿（1000 筆）；新擷取仍儲存於歷史。", "Queue is full (1000 items); new captures remain in history.");
    internal static string Sent => T("系统已接受本次输入，目标应用是否接收需另行确认。", "系統已接受本次輸入，目標應用程式是否接收需另行確認。", "Windows accepted input; target consumption is not confirmed.");
    internal static string Sensitive => T("队首包含敏感内容。只允许本次粘贴？", "佇首含敏感內容。只允許本次貼上？", "The next item is sensitive. Allow this paste only?");
    internal static string Count(int count) => T($"剩余 {count} 条", $"剩餘 {count} 筆", $"{count} items remaining");
    internal static string Row(int index, long id) => T($"{index}. 记录 #{id}", $"{index}. 記錄 #{id}", $"{index}. Record #{id}");
}
