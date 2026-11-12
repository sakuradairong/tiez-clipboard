namespace TieZ.Native;

/// <summary>
/// Chrome of the main window shell: navigation entries, pane footer lifecycle actions, InfoBars, exit progress
/// and the shell status reported by <see cref="ShellController"/>.
/// </summary>
internal sealed class ShellStrings
{
    internal required string WindowTitle { get; init; }
    internal required string NavigationHistory { get; init; }
    internal required string NavigationTags { get; init; }
    internal required string NavigationSettings { get; init; }
    internal required string HideToTray { get; init; }
    internal required string HideToTrayHint { get; init; }
    internal required string Quit { get; init; }
    internal required string QuitHint { get; init; }
    internal required string ExitProgressName { get; init; }
    internal required string Exiting { get; init; }
    internal required string MigrationNote { get; init; }
    internal required string MigrationNoteName { get; init; }
    internal required string MigrationNoteHint { get; init; }
    internal required string ShellStatusTitle { get; init; }
    internal required string OpenCleanupTitle { get; init; }
    internal required string OpenCleanupMessage { get; init; }
    internal required string ExitTitle { get; init; }
    internal required string ExitRetry { get; init; }
    internal required string ExitStoppingMessage { get; init; }
    internal required string ExitFailedTitle { get; init; }
    internal required string ExitLockedMessage { get; init; }
    internal required string ExitFailedMessage { get; init; }
    internal required string HotkeyPending { get; init; }
    internal required string HotkeyOccupied { get; init; }
    internal required string HotkeyFailureFormat { get; init; }

    internal static ShellStrings Zh { get; } = new()
    {
        WindowTitle = "TieZ · 原生迁移版",
        NavigationHistory = "剪贴板",
        NavigationTags = "标签",
        NavigationSettings = "设置",
        HideToTray = "隐藏到托盘",
        HideToTrayHint = "隐藏到托盘，后台服务继续运行",
        Quit = "退出 TieZ",
        QuitHint = "退出 TieZ 并停止后台服务",
        ExitProgressName = "退出进度",
        Exiting = "正在退出…",
        MigrationNote = "原生迁移版 · 使用独立数据库",
        MigrationNoteName = "原生迁移版，使用独立数据库",
        MigrationNoteHint = "不会自动打开已安装旧版的剪贴板数据。",
        ShellStatusTitle = "后台服务状态",
        OpenCleanupTitle = "临时文件清理",
        OpenCleanupMessage = "部分临时导出文件尚未清理。关闭使用这些文件的编辑器后，应用将自动重试；原始历史内容保持不变。",
        ExitTitle = "正在退出",
        ExitRetry = "重试退出",
        ExitStoppingMessage = "正在停止后台服务并清理临时文件，请稍候。",
        ExitFailedTitle = "退出尚未完成",
        ExitLockedMessage = "临时导出文件仍被外部应用占用。请先关闭对应编辑器，再选择“重试退出”。剪贴板采集和其他后台服务已停止，临时文件仍由核心跟踪。",
        ExitFailedMessage = "核心资源释放失败。请重试退出；当前窗口会保留，避免丢失清理重试入口。",
        HotkeyPending = "上次快捷键修改未完成，原绑定仍可用。",
        HotkeyOccupied = "主快捷键已被系统或其他应用占用，请在设置中更改。",
        HotkeyFailureFormat = "主快捷键应用未完成：{0}"
    };

    internal static ShellStrings Tw { get; } = new()
    {
        WindowTitle = "TieZ · 原生遷移版",
        NavigationHistory = "剪貼簿",
        NavigationTags = "標籤",
        NavigationSettings = "設定",
        HideToTray = "隱藏至系統匣",
        HideToTrayHint = "隱藏至系統匣，背景服務繼續執行",
        Quit = "結束 TieZ",
        QuitHint = "結束 TieZ 並停止背景服務",
        ExitProgressName = "結束進度",
        Exiting = "正在結束…",
        MigrationNote = "原生遷移版 · 使用獨立資料庫",
        MigrationNoteName = "原生遷移版，使用獨立資料庫",
        MigrationNoteHint = "不會自動開啟已安裝舊版的剪貼簿資料。",
        ShellStatusTitle = "背景服務狀態",
        OpenCleanupTitle = "暫存檔清理",
        OpenCleanupMessage = "部分暫存匯出檔案尚未清理。關閉使用這些檔案的編輯器後，應用程式會自動重試；原始歷史內容保持不變。",
        ExitTitle = "正在結束",
        ExitRetry = "重試結束",
        ExitStoppingMessage = "正在停止背景服務並清理暫存檔，請稍候。",
        ExitFailedTitle = "結束尚未完成",
        ExitLockedMessage = "暫存匯出檔案仍被外部應用程式佔用。請先關閉對應編輯器，再選擇“重試結束”。剪貼簿擷取與其他背景服務已停止，暫存檔仍由核心追蹤。",
        ExitFailedMessage = "核心資源釋放失敗。請重試結束；目前視窗會保留，避免遺失清理重試入口。",
        HotkeyPending = "上次快速鍵修改未完成，原綁定仍可使用。",
        HotkeyOccupied = "主快速鍵已被系統或其他應用程式佔用，請在設定中變更。",
        HotkeyFailureFormat = "主快速鍵套用未完成：{0}"
    };

    internal static ShellStrings En { get; } = new()
    {
        WindowTitle = "TieZ - native migration",
        NavigationHistory = "Clipboard",
        NavigationTags = "Tags",
        NavigationSettings = "Settings",
        HideToTray = "Hide to tray",
        HideToTrayHint = "Hide to the tray and keep background services running",
        Quit = "Exit TieZ",
        QuitHint = "Exit TieZ and stop background services",
        ExitProgressName = "Exit progress",
        Exiting = "Exiting...",
        MigrationNote = "Native migration build - separate database",
        MigrationNoteName = "Native migration build with a separate database",
        MigrationNoteHint = "Clipboard data from an installed older version is not opened automatically.",
        ShellStatusTitle = "Background service status",
        OpenCleanupTitle = "Temporary file cleanup",
        OpenCleanupMessage = "Some temporary export files are not cleaned up yet. Close the editors that use them and the app retries automatically; original history content stays unchanged.",
        ExitTitle = "Exiting",
        ExitRetry = "Retry exit",
        ExitStoppingMessage = "Stopping background services and cleaning temporary files. Please wait.",
        ExitFailedTitle = "Exit did not finish",
        ExitLockedMessage = "A temporary export file is still open in another app. Close that editor, then choose Retry exit. Clipboard capture and other background services have stopped; the core still tracks the temporary file.",
        ExitFailedMessage = "Releasing core resources failed. Retry the exit; this window stays open so the cleanup retry is not lost.",
        HotkeyPending = "The previous shortcut change did not finish; the original binding still works.",
        HotkeyOccupied = "The main shortcut is taken by Windows or another app. Change it in settings.",
        HotkeyFailureFormat = "The main shortcut was not applied completely: {0}"
    };

    internal static ShellStrings Current => LanguageService.Current switch
    {
        Language.TraditionalChinese => Tw,
        Language.English => En,
        _ => Zh
    };
}