namespace TieZ.Native;

/// <summary>
/// Text of the opener rows in the settings window and of the application picker dialog they open, including the
/// localized labels of the picker rows.
/// </summary>
internal sealed class AppPickerStrings
{
    internal required string OpenerIntro { get; init; }
    internal required string OpenerHint { get; init; }
    internal required string Browse { get; init; }
    internal required string Apps { get; init; }
    internal required string AppsAutomationFormat { get; init; }
    internal required string BrowseAutomationFormat { get; init; }
    internal required string ResetAutomationFormat { get; init; }
    internal required string BrowseFailed { get; init; }
    internal required string StaleChoice { get; init; }
    internal required string DialogTitleFormat { get; init; }
    internal required string DialogPrimary { get; init; }
    internal required string DialogFailed { get; init; }
    internal required string DefaultApp { get; init; }
    internal required string FollowTextApp { get; init; }
    internal required string OpenerText { get; init; }
    internal required string OpenerCode { get; init; }
    internal required string OpenerRichText { get; init; }
    internal required string OpenerImage { get; init; }
    internal required string OpenerFile { get; init; }
    internal required string OpenerVideo { get; init; }
    internal required string OpenerUrl { get; init; }
    internal required string OpenerLegacyLink { get; init; }
    internal required string DefaultRowGroup { get; init; }
    internal required string DefaultRowDetailsCode { get; init; }
    internal required string DefaultRowDetailsOther { get; init; }
    internal required string SearchPlaceholder { get; init; }
    internal required string SearchName { get; init; }
    internal required string Refresh { get; init; }
    internal required string CategoryName { get; init; }
    internal required string CategoryAll { get; init; }
    internal required string CategoryRecommended { get; init; }
    internal required string CategoryOther { get; init; }
    internal required string CategoryUnavailable { get; init; }
    internal required string ChoicesName { get; init; }
    internal required string Footer { get; init; }
    internal required string CurrentDraftFormat { get; init; }
    internal required string DefaultSummaryCode { get; init; }
    internal required string DefaultSummaryFormat { get; init; }
    internal required string DefaultSummaryUnknown { get; init; }
    internal required string LoadingSummary { get; init; }
    internal required string SummaryFormat { get; init; }
    internal required string SummaryCached { get; init; }
    internal required string SummaryIncomplete { get; init; }
    internal required string SummaryFailed { get; init; }
    internal required string SelectionSelectedFormat { get; init; }
    internal required string SelectionUnavailableFormat { get; init; }
    internal required string SelectionNone { get; init; }
    internal required string Busy { get; init; }
    internal required string LoadFailed { get; init; }
    internal required string Invalidated { get; init; }
    internal required string GroupAssociationFormat { get; init; }
    internal required string GroupKeywordFormat { get; init; }
    internal required string GroupOtherFormat { get; init; }
    internal required string GroupUnavailable { get; init; }
    internal required string KindDesktop { get; init; }
    internal required string KindPackaged { get; init; }
    internal required string KindUnavailable { get; init; }
    internal required string UnavailableMissingTarget { get; init; }
    internal required string UnavailableInvalidIdentity { get; init; }
    internal required string UnavailableNotNative { get; init; }
    internal required string UnavailableGeneric { get; init; }
    internal required string DetailSeparator { get; init; }
    internal required string AccessibilityFormat { get; init; }
    internal required string IconName { get; init; }
    internal required string IconLoading { get; init; }
    internal required string IconUnavailable { get; init; }

    internal static AppPickerStrings Zh { get; } = new()
    {
        OpenerIntro = "选择的应用在保存后用于打开相应记录。空值使用下方默认规则；代码留空时跟随文本应用。",
        OpenerHint = "可从应用列表选择、浏览本地 .exe，或填写已知程序路径/打包应用标识。已有配置会保留，能否处理内容在打开时检查。.bat/.cmd 和 DOS 程序暂不支持，可改选原生程序或默认规则。",
        Browse = "浏览 .exe…",
        Apps = "从应用列表选择…",
        AppsAutomationFormat = "从列表选择{0}打开应用",
        BrowseAutomationFormat = "选择{0}的打开程序",
        ResetAutomationFormat = "{0}：{1}",
        BrowseFailed = "无法选择程序，请重试或直接填写已知程序路径。",
        StaleChoice = "配置或当前应用草稿已变化，本次选择未应用，请重新选择。",
        DialogTitleFormat = "选择{0}打开应用",
        DialogPrimary = "使用所选",
        DialogFailed = "无法打开应用选择，请关闭其他对话框后重试。",
        DefaultApp = "使用系统默认应用",
        FollowTextApp = "跟随文本应用",
        OpenerText = "文本",
        OpenerCode = "代码",
        OpenerRichText = "富文本与表格",
        OpenerImage = "图片",
        OpenerFile = "文件",
        OpenerVideo = "视频文件",
        OpenerUrl = "网址",
        OpenerLegacyLink = "旧链接类型",
        DefaultRowGroup = "默认规则",
        DefaultRowDetailsCode = "保存后跟随文本打开应用",
        DefaultRowDetailsOther = "打开时使用 Windows 当前关联",
        SearchPlaceholder = "搜索名称、程序路径或应用标识",
        SearchName = "搜索打开应用",
        Refresh = "刷新列表",
        CategoryName = "应用列表分类",
        CategoryAll = "全部",
        CategoryRecommended = "推荐",
        CategoryOther = "其他应用",
        CategoryUnavailable = "无法使用",
        ChoicesName = "打开应用选项",
        Footer = "推荐不保证应用支持所有格式。此处只选择草稿，返回后仍需保存设置。.bat/.cmd 和 DOS 程序暂不支持，可选择原生程序或默认规则。",
        CurrentDraftFormat = "当前草稿：{0}",
        DefaultSummaryCode = "默认跟随文本应用",
        DefaultSummaryFormat = "Windows 默认：{0}",
        DefaultSummaryUnknown = "Windows 默认应用名称不可读取",
        LoadingSummary = "正在读取应用目录…",
        SummaryFormat = "显示 {0} 项；目录中 {1} 项无法用作打开程序。",
        SummaryCached = " 使用上次成功读取的列表；可刷新。",
        SummaryIncomplete = " 部分目录或推荐未能完整读取。",
        SummaryFailed = "应用目录尚未读取成功。",
        SelectionSelectedFormat = "已选：{0} · {1}",
        SelectionUnavailableFormat = "不可选择：{0} · {1}",
        SelectionNone = "请选择一项；当前草稿不会自动更改。",
        Busy = "上一次读取仍在停止，请稍后刷新。",
        LoadFailed = "无法读取应用目录，请刷新重试，或取消后填写程序路径。",
        Invalidated = "配置或应用草稿已变化，请取消后重新选择。",
        GroupAssociationFormat = "系统关联推荐 · {0}",
        GroupKeywordFormat = "名称推荐 · {0}",
        GroupOtherFormat = "其他应用 · {0}",
        GroupUnavailable = "无法用作打开程序",
        KindDesktop = "桌面程序",
        KindPackaged = "打包应用",
        KindUnavailable = "无法使用",
        UnavailableMissingTarget = "没有可读取的启动目标",
        UnavailableInvalidIdentity = "应用标识无法使用",
        UnavailableNotNative = "目标缺失或不是支持的原生程序",
        UnavailableGeneric = "没有支持的打开方式",
        DetailSeparator = " · ",
        AccessibilityFormat = "{0}，{1}，{2}",
        IconName = "应用图标",
        IconLoading = "正在加载应用图标",
        IconUnavailable = "图标不可用，不影响选择应用"
    };
    internal static AppPickerStrings Tw { get; } = new()
    {
        OpenerIntro = "選擇的應用程式在儲存後用於開啟對應記錄。空值使用下方預設規則；程式碼留空時跟隨文字應用程式。",
        OpenerHint = "可從應用程式清單選擇、瀏覽本機 .exe，或填寫已知程式路徑/封裝應用程式識別碼。既有設定會保留，能否處理內容在開啟時檢查。.bat/.cmd 和 DOS 程式暫不支援，可改選原生程式或預設規則。",
        Browse = "瀏覽 .exe…",
        Apps = "從應用程式清單選擇…",
        AppsAutomationFormat = "從清單選擇{0}開啟應用程式",
        BrowseAutomationFormat = "選擇{0}的開啟程式",
        ResetAutomationFormat = "{0}：{1}",
        BrowseFailed = "無法選擇程式，請重試或直接填寫已知程式路徑。",
        StaleChoice = "設定或目前應用程式草稿已變更，本次選擇未套用，請重新選擇。",
        DialogTitleFormat = "選擇{0}開啟應用程式",
        DialogPrimary = "使用所選",
        DialogFailed = "無法開啟應用程式選擇，請關閉其他對話方塊後重試。",
        DefaultApp = "使用系統預設應用",
        FollowTextApp = "跟隨文字應用程式",
        OpenerText = "文字",
        OpenerCode = "程式碼",
        OpenerRichText = "富文字與表格",
        OpenerImage = "圖片",
        OpenerFile = "檔案",
        OpenerVideo = "影片檔案",
        OpenerUrl = "網址",
        OpenerLegacyLink = "舊連結類型",
        DefaultRowGroup = "預設規則",
        DefaultRowDetailsCode = "儲存後跟隨文字開啟應用程式",
        DefaultRowDetailsOther = "開啟時使用 Windows 目前關聯",
        SearchPlaceholder = "搜尋名稱、程式路徑或應用程式識別碼",
        SearchName = "搜尋開啟應用程式",
        Refresh = "重新整理清單",
        CategoryName = "應用程式清單分類",
        CategoryAll = "全部",
        CategoryRecommended = "建議",
        CategoryOther = "其他應用程式",
        CategoryUnavailable = "無法使用",
        ChoicesName = "開啟應用程式選項",
        Footer = "建議不保證應用程式支援所有格式。此處只選擇草稿，返回後仍需儲存設定。.bat/.cmd 和 DOS 程式暫不支援，可選擇原生程式或預設規則。",
        CurrentDraftFormat = "目前草稿：{0}",
        DefaultSummaryCode = "預設跟隨文字應用程式",
        DefaultSummaryFormat = "Windows 預設：{0}",
        DefaultSummaryUnknown = "無法讀取 Windows 預設應用程式名稱",
        LoadingSummary = "正在讀取應用程式目錄…",
        SummaryFormat = "顯示 {0} 項；目錄中 {1} 項無法作為開啟程式。",
        SummaryCached = " 使用上次成功讀取的清單；可重新整理。",
        SummaryIncomplete = " 部分目錄或建議未能完整讀取。",
        SummaryFailed = "應用程式目錄尚未讀取成功。",
        SelectionSelectedFormat = "已選：{0} · {1}",
        SelectionUnavailableFormat = "無法選擇：{0} · {1}",
        SelectionNone = "請選擇一項；目前草稿不會自動變更。",
        Busy = "上一次讀取仍在停止，請稍後重新整理。",
        LoadFailed = "無法讀取應用程式目錄，請重新整理重試，或取消後填寫程式路徑。",
        Invalidated = "設定或應用程式草稿已變更，請取消後重新選擇。",
        GroupAssociationFormat = "系統關聯建議 · {0}",
        GroupKeywordFormat = "名稱建議 · {0}",
        GroupOtherFormat = "其他應用程式 · {0}",
        GroupUnavailable = "無法作為開啟程式",
        KindDesktop = "桌面程式",
        KindPackaged = "封裝應用程式",
        KindUnavailable = "無法使用",
        UnavailableMissingTarget = "沒有可讀取的啟動目標",
        UnavailableInvalidIdentity = "應用程式識別碼無法使用",
        UnavailableNotNative = "目標缺少或不是支援的原生程式",
        UnavailableGeneric = "沒有支援的開啟方式",
        DetailSeparator = " · ",
        AccessibilityFormat = "{0}，{1}，{2}",
        IconName = "應用程式圖示",
        IconLoading = "正在載入應用程式圖示",
        IconUnavailable = "圖示無法使用，不影響選擇應用程式"
    };

    internal static AppPickerStrings En { get; } = new()
    {
        OpenerIntro = "The chosen app opens the matching records after saving. An empty value uses the default rule below; an empty code app follows the text app.",
        OpenerHint = "Choose from the app list, browse a local .exe, or type a known program path or packaged app identity. Existing configuration is kept; whether an app can handle the content is checked when opening. .bat/.cmd and DOS programs are not supported yet; pick a native program or use the default rule.",
        Browse = "Browse .exe...",
        Apps = "Choose from the app list...",
        AppsAutomationFormat = "Choose the app that opens {0}",
        BrowseAutomationFormat = "Choose the program that opens {0}",
        ResetAutomationFormat = "{0}: {1}",
        BrowseFailed = "The program could not be chosen; retry or type a known program path.",
        StaleChoice = "The configuration or the current app draft changed; this choice was not applied. Choose again.",
        DialogTitleFormat = "Choose the app that opens {0}",
        DialogPrimary = "Use selection",
        DialogFailed = "The app picker could not open; close other dialogs and retry.",
        DefaultApp = "Use the system default app",
        FollowTextApp = "Follow the text app",
        OpenerText = "Text",
        OpenerCode = "Code",
        OpenerRichText = "Rich text and tables",
        OpenerImage = "Image",
        OpenerFile = "File",
        OpenerVideo = "Video file",
        OpenerUrl = "URL",
        OpenerLegacyLink = "Legacy link type",
        DefaultRowGroup = "Default rule",
        DefaultRowDetailsCode = "Opens with the text app after saving",
        DefaultRowDetailsOther = "Opens with the current Windows association",
        SearchPlaceholder = "Search name, program path or app ID",
        SearchName = "Search for an app",
        Refresh = "Refresh list",
        CategoryName = "App list category",
        CategoryAll = "All",
        CategoryRecommended = "Recommended",
        CategoryOther = "Other apps",
        CategoryUnavailable = "Unavailable",
        ChoicesName = "Apps to open with",
        Footer = "A recommendation does not guarantee that an app supports every format. This only selects a draft; save the settings after returning. .bat/.cmd and DOS programs are not supported yet; pick a native program or the default rule.",
        CurrentDraftFormat = "Current draft: {0}",
        DefaultSummaryCode = "Follows the text app by default",
        DefaultSummaryFormat = "Windows default: {0}",
        DefaultSummaryUnknown = "The Windows default app name cannot be read",
        LoadingSummary = "Loading the app catalog...",
        SummaryFormat = "Showing {0} rows; {1} catalog entries cannot be used to open.",
        SummaryCached = " Using the last successful list; refresh to update.",
        SummaryIncomplete = " Part of the catalog or the recommendations could not be read completely.",
        SummaryFailed = "The app catalog has not loaded successfully.",
        SelectionSelectedFormat = "Selected: {0} - {1}",
        SelectionUnavailableFormat = "Not selectable: {0} - {1}",
        SelectionNone = "Choose one row; the current draft does not change by itself.",
        Busy = "The previous read is still stopping; refresh later.",
        LoadFailed = "The app catalog could not be read; refresh to retry, or cancel and type a program path.",
        Invalidated = "The configuration or app draft changed; cancel and choose again.",
        GroupAssociationFormat = "System association - {0}",
        GroupKeywordFormat = "Name match - {0}",
        GroupOtherFormat = "Other app - {0}",
        GroupUnavailable = "Cannot be used to open",
        KindDesktop = "Desktop app",
        KindPackaged = "Packaged app",
        KindUnavailable = "Unavailable",
        UnavailableMissingTarget = "No readable launch target",
        UnavailableInvalidIdentity = "The app identity cannot be used",
        UnavailableNotNative = "The target is missing or not a supported native program",
        UnavailableGeneric = "No supported way to open",
        DetailSeparator = " - ",
        AccessibilityFormat = "{0}, {1}, {2}",
        IconName = "App icon",
        IconLoading = "Loading the app icon",
        IconUnavailable = "The icon is unavailable; the app can still be chosen"
    };

    internal static AppPickerStrings Current => LanguageService.Current switch
    {
        Language.TraditionalChinese => Tw,
        Language.English => En,
        _ => Zh
    };
}