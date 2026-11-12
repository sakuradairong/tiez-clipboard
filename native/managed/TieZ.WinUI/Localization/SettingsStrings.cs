namespace TieZ.Native;

/// <summary>Text of the ordinary settings window and its controls.</summary>
internal sealed class SettingsStrings
{
    internal required string WindowTitle { get; init; }
    internal required string Heading { get; init; }
    internal required string Intro { get; init; }
    internal required string LanguageHeader { get; init; }
    internal required string LanguageHint { get; init; }
    internal required string ThemeHeader { get; init; }
    internal required string ThemeSystem { get; init; }
    internal required string ThemeLight { get; init; }
    internal required string ThemeDark { get; init; }
    internal required string HotkeyHeader { get; init; }
    internal required string HotkeyHint { get; init; }
    internal required string SearchHotkeyHeader { get; init; }
    internal required string SearchHotkeyHint { get; init; }
    internal required string RichPasteHeader { get; init; }
    internal required string PlainPasteHeader { get; init; }
    internal required string QuickPasteHeader { get; init; }
    internal required string QuickPasteHint { get; init; }
    internal required string QuickPasteOff { get; init; }
    internal required string Deduplicate { get; init; }
    internal required string ShowSearch { get; init; }
    internal required string Compact { get; init; }
    internal required string PersistentHeader { get; init; }
    internal required string PersistentOn { get; init; }
    internal required string PersistentOff { get; init; }
    internal required string CaptureFiles { get; init; }
    internal required string CaptureRichText { get; init; }
    internal required string DeleteAfterPaste { get; init; }
    internal required string MoveToTop { get; init; }
    internal required string EdgeDocking { get; init; }
    internal required string Privacy { get; init; }
    internal required string LimitEnabled { get; init; }
    internal required string LimitHeader { get; init; }
    internal required string Advanced { get; init; }
    internal required string Openers { get; init; }
    internal required string Reload { get; init; }
    internal required string Save { get; init; }
    internal required string RevisionFormat { get; init; }
    internal required string Conflict { get; init; }
    internal required string Saved { get; init; }
    internal required string SavedWarningFormat { get; init; }
    internal required string HotkeyConflict { get; init; }

    internal static SettingsStrings Zh { get; } = new()
    {
        WindowTitle = "TieZ · 设置",
        Heading = "设置",
        Intro = "原生窗口 · 与主窗口共享同一配置核心",
        LanguageHeader = "界面语言",
        LanguageHint = "切换后立即应用到所有已打开的窗口，无需重启。",
        ThemeHeader = "外观",
        ThemeSystem = "跟随系统",
        ThemeLight = "浅色",
        ThemeDark = "深色",
        HotkeyHeader = "主窗口快捷键（留空禁用）",
        HotkeyHint = "Win+V 可能被 Windows 占用。支持键盘组合；留空可禁用。",
        SearchHotkeyHeader = "搜索快捷键（留空禁用）",
        SearchHotkeyHint = "唤出速贴窗口并定位到搜索框。默认 Alt+F。",
        RichPasteHeader = "原格式粘贴最新一条（留空禁用）",
        PlainPasteHeader = "纯文本粘贴最新一条（留空禁用）",
        QuickPasteHeader = "数字快捷粘贴",
        QuickPasteHint = "用修饰键加 1 到 9 粘贴对应位置的置顶条目，0 是第 10 条。默认关闭。",
        QuickPasteOff = "关闭",
        Deduplicate = "自动去重",
        ShowSearch = "显示主窗口搜索框",
        Compact = "紧凑列表（鼠标悬停 1 秒显示预览）",
        PersistentHeader = "保存采集历史到磁盘",
        PersistentOn = "持久化保存",
        PersistentOff = "仅保留本次会话",
        CaptureFiles = "采集文件路径",
        CaptureRichText = "采集富文本和表格原始格式",
        DeleteAfterPaste = "粘贴后删除未置顶且无标签的条目",
        MoveToTop = "粘贴后把未置顶条目移到最前",
        EdgeDocking = "窗口贴边后收成一条边",
        Privacy = "自动标记敏感内容",
        LimitEnabled = "限制磁盘历史数量（置顶和有标签条目不计入）",
        LimitHeader = "最多保留普通历史条数（0–1000000）",
        Advanced = "高级设置 · 隐私与清理规则",
        Openers = "打开记录使用的应用",
        Reload = "重新载入",
        Save = "保存设置",
        RevisionFormat = "已保存配置版本 {0}",
        Conflict = "配置已变化。请重新载入后再修改；当前草稿尚未保存。",
        Saved = "设置已保存，主窗口已同步。",
        SavedWarningFormat = "设置已保存，但快捷键未完整应用：{0}。可退出并重新打开应用恢复。",
        HotkeyConflict = "快捷键已被系统或其他应用占用。原配置和绑定保持，当前草稿已保留。"
    };

    internal static SettingsStrings Tw { get; } = new()
    {
        WindowTitle = "TieZ · 設定",
        Heading = "設定",
        Intro = "原生視窗 · 與主視窗共用同一設定核心",
        LanguageHeader = "介面語言",
        LanguageHint = "切換後立即套用到所有已開啟的視窗，無需重新啟動。",
        ThemeHeader = "外觀",
        ThemeSystem = "跟隨系統",
        ThemeLight = "淺色",
        ThemeDark = "深色",
        HotkeyHeader = "主視窗快速鍵（留空停用）",
        HotkeyHint = "Win+V 可能被 Windows 佔用。支援鍵盤組合；留空可停用。",
        SearchHotkeyHeader = "搜尋快速鍵（留空停用）",
        SearchHotkeyHint = "叫出速貼視窗並移到搜尋方塊。預設 Alt+F。",
        RichPasteHeader = "原格式貼上最新一筆（留空停用）",
        PlainPasteHeader = "純文字貼上最新一筆（留空停用）",
        QuickPasteHeader = "數字快速貼上",
        QuickPasteHint = "用修飾鍵加 1 到 9 貼上對應位置的置頂項目，0 是第 10 筆。預設關閉。",
        QuickPasteOff = "關閉",
        Deduplicate = "自動去重",
        ShowSearch = "顯示主視窗搜尋方塊",
        Compact = "精簡清單（滑鼠停留 1 秒顯示預覽）",
        PersistentHeader = "儲存擷取歷史到磁碟",
        PersistentOn = "永久儲存",
        PersistentOff = "僅保留本次工作階段",
        CaptureFiles = "擷取檔案路徑",
        CaptureRichText = "擷取富文字與表格原始格式",
        DeleteAfterPaste = "貼上後刪除未置頂且無標籤的項目",
        MoveToTop = "貼上後把未置頂項目移到最前",
        EdgeDocking = "視窗貼齊邊緣後收成一條邊",
        Privacy = "自動標記敏感內容",
        LimitEnabled = "限制磁碟歷史數量（置頂與有標籤項目不計入）",
        LimitHeader = "最多保留一般歷史筆數（0–1000000）",
        Advanced = "進階設定 · 隱私與清理規則",
        Openers = "開啟記錄使用的應用程式",
        Reload = "重新載入",
        Save = "儲存設定",
        RevisionFormat = "已儲存設定版本 {0}",
        Conflict = "設定已變更。請重新載入後再修改；目前草稿尚未儲存。",
        Saved = "設定已儲存，主視窗已同步。",
        SavedWarningFormat = "設定已儲存，但快速鍵未完整套用：{0}。可結束並重新開啟應用程式復原。",
        HotkeyConflict = "快速鍵已被系統或其他應用程式佔用。原設定與綁定保持，目前草稿已保留。"
    };

    internal static SettingsStrings En { get; } = new()
    {
        WindowTitle = "TieZ - settings",
        Heading = "Settings",
        Intro = "Native window - shares one configuration core with the main window",
        LanguageHeader = "Interface language",
        LanguageHint = "Applies to every open window immediately; no restart needed.",
        ThemeHeader = "Appearance",
        ThemeSystem = "Use system setting",
        ThemeLight = "Light",
        ThemeDark = "Dark",
        HotkeyHeader = "Main window shortcut (leave empty to disable)",
        HotkeyHint = "Win+V may be taken by Windows. Keyboard combinations are supported; leave empty to disable.",
        SearchHotkeyHeader = "Search shortcut (leave empty to disable)",
        SearchHotkeyHint = "Opens the quick-paste window and moves to the search box. Alt+F by default.",
        RichPasteHeader = "Paste the newest item with formatting (empty disables)",
        PlainPasteHeader = "Paste the newest text item as plain text (empty disables)",
        QuickPasteHeader = "Number quick paste",
        QuickPasteHint = "Modifier plus 1-9 pastes the pinned item at that position. 0 is the 10th. Off by default.",
        QuickPasteOff = "Off",
        Deduplicate = "Remove duplicates automatically",
        ShowSearch = "Show the search box in the main window",
        Compact = "Compact list (hover for 1 second to preview)",
        PersistentHeader = "Save captured history to disk",
        PersistentOn = "Keep on disk",
        PersistentOff = "This session only",
        CaptureFiles = "Capture file paths",
        CaptureRichText = "Capture rich text and table formatting",
        DeleteAfterPaste = "Delete unpinned, untagged items after paste",
        MoveToTop = "Move unpinned items to the front after paste",
        EdgeDocking = "Collapse a window to a sliver at the screen edge",
        Privacy = "Flag sensitive content automatically",
        LimitEnabled = "Limit stored history (pinned and tagged entries are excluded)",
        LimitHeader = "Maximum stored plain history entries (0-1000000)",
        Advanced = "Advanced settings - privacy and cleanup rules",
        Openers = "Apps used to open records",
        Reload = "Reload",
        Save = "Save settings",
        RevisionFormat = "Saved configuration revision {0}",
        Conflict = "The configuration changed. Reload before editing again; the current draft is not saved.",
        Saved = "Settings saved; the main window is updated.",
        SavedWarningFormat = "Settings saved, but the shortcut was not applied completely: {0}. Exit and reopen the app to recover.",
        HotkeyConflict = "The shortcut is taken by Windows or another app. The saved configuration and binding stay, and the current draft is kept."
    };

    internal static SettingsStrings Current => LanguageService.Current switch
    {
        Language.TraditionalChinese => Tw,
        Language.English => En,
        _ => Zh
    };
}