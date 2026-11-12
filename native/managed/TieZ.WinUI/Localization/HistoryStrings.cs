namespace TieZ.Native;

/// <summary>
/// Chrome of the clipboard history workspace: header, capture/target status, search and order controls, selection
/// and detail captions, record metadata and the history dialogs. History bodies, tag names, source app names and
/// core messages never live here.
/// </summary>
internal sealed class HistoryStrings
{
    // Page header
    internal required string Title { get; init; }
    internal required string DatabaseNote { get; init; }
    internal required string DatabaseNoteToolTip { get; init; }

    // Capture status
    internal required string CaptureStart { get; init; }
    internal required string CapturePause { get; init; }
    internal required string CaptureListening { get; init; }
    internal required string CapturePaused { get; init; }
    internal required string CapturePreparing { get; init; }
    internal required string CaptureStatusFormat { get; init; }
    internal required string CaptureStatusToolTip { get; init; }
    internal required string CaptureUnknown { get; init; }
    internal required string CaptureUnknownFormat { get; init; }
    internal required string CaptureErrorFormat { get; init; }
    internal required string CaptureStatusTitle { get; init; }

    // Target tracking
    internal required string ReturnTargetLabel { get; init; }
    internal required string ReturnTargetToolTip { get; init; }
    internal required string StartTargetLabel { get; init; }
    internal required string StartTargetToolTip { get; init; }
    internal required string TargetReading { get; init; }
    internal required string TargetFormat { get; init; }
    internal required string TargetNone { get; init; }
    internal required string TargetStopped { get; init; }
    internal required string TargetErrorFormat { get; init; }
    internal required string TargetErrorSuffix { get; init; }
    internal required string ReturnedTitle { get; init; }
    internal required string ReturnedMessage { get; init; }

    // Search and filters
    internal required string SearchPlaceholder { get; init; }
    internal required string SearchName { get; init; }
    internal required string TypeFilterName { get; init; }
    internal required string TypeAll { get; init; }
    internal required string TypeText { get; init; }
    internal required string TypeCode { get; init; }
    internal required string TypeLink { get; init; }
    internal required string TypeRichText { get; init; }
    internal required string TypeImage { get; init; }
    internal required string TypeFile { get; init; }
    internal required string TypeVideo { get; init; }
    internal required string TagFilterPlaceholder { get; init; }
    internal required string TagFilterName { get; init; }
    internal required string PinnedOnly { get; init; }
    internal required string TagOnly { get; init; }

    // Order and layout
    internal required string SortName { get; init; }
    internal required string SortToolTip { get; init; }
    internal required string SortPinned { get; init; }
    internal required string SortTime { get; init; }
    internal required string SortUsage { get; init; }
    internal required string LayoutName { get; init; }
    internal required string LayoutToolTip { get; init; }
    internal required string LayoutList { get; init; }
    internal required string LayoutCards { get; init; }
    internal required string ReloadLayoutName { get; init; }
    internal required string ReloadLayoutToolTip { get; init; }
    internal required string LayoutSaveFailed { get; init; }

    // Selection and detail
    internal required string SelectionHint { get; init; }
    internal required string SelectionCountFormat { get; init; }
    internal required string DetailTitleDefault { get; init; }
    internal required string DetailTitleFormat { get; init; }
    internal required string DetailEmptyHint { get; init; }
    internal required string DetailLoading { get; init; }
    internal required string DetailMultipleHint { get; init; }
    internal required string DetailStaleHint { get; init; }
    internal required string DetailSensitiveHint { get; init; }
    internal required string DetailHeading { get; init; }
    internal required string DetailTextName { get; init; }
    internal required string RevealSensitive { get; init; }
    internal required string ImageUnavailable { get; init; }

    // Record metadata
    internal required string MetadataSeparator { get; init; }
    internal required string MetadataPinned { get; init; }
    internal required string MetadataUsageFormat { get; init; }
    internal required string MetadataSessionOnly { get; init; }
    internal required string SourceManual { get; init; }
    internal required string PreviewStale { get; init; }
    internal required string PreviewSensitive { get; init; }
    internal required string PreviewImageHint { get; init; }
    internal required string ListName { get; init; }
    internal required string CardsName { get; init; }
    internal required string ListStatusFormat { get; init; }
    internal required string LoadMore { get; init; }

    // Commands
    internal required string AddTitle { get; init; }
    internal required string EditTitle { get; init; }
    internal required string InputPlaceholder { get; init; }
    internal required string PasteToolTip { get; init; }
    internal required string CopyToolTip { get; init; }
    internal required string OpenToolTip { get; init; }
    internal required string PreviewLabel { get; init; }
    internal required string RefreshLabel { get; init; }
    internal required string RefreshPending { get; init; }
    internal required string RefreshHistory { get; init; }
    internal required string ClearHistory { get; init; }
    internal required string DeleteSelectedFormat { get; init; }
    internal required string PinAction { get; init; }
    internal required string UnpinAction { get; init; }

    // Card affordances
    internal required string CardCopyToolTip { get; init; }
    internal required string CardCopyName { get; init; }
    internal required string CardCopyPlainToolTip { get; init; }
    internal required string CardCopyPlainName { get; init; }
    internal required string CardOpenToolTip { get; init; }
    internal required string CardOpenName { get; init; }

    // Feedback
    internal required string ErrorTitle { get; init; }
    internal required string MutateBlocked { get; init; }
    internal required string CopyCancelled { get; init; }
    internal required string CopySuccessTitle { get; init; }
    internal required string CopySuccessPlain { get; init; }
    internal required string CopySuccessOriginal { get; init; }
    internal required string CopyConflict { get; init; }
    internal required string OpenCancelled { get; init; }
    internal required string OpenUnverified { get; init; }
    internal required string OpenConflict { get; init; }
    internal required string OpenNeedsReveal { get; init; }
    internal required string OpenFailed { get; init; }
    internal required string OpenSuccessTitle { get; init; }
    internal required string OpenUsageTitle { get; init; }
    internal required string OpenExportedNoWriteback { get; init; }
    internal required string OpenExported { get; init; }
    internal required string OpenHanded { get; init; }
    internal required string OpenUsageWarning { get; init; }
    internal required string PasteSentTitle { get; init; }
    internal required string PasteSentMessage { get; init; }

    // Dialogs
    internal required string TagPickerTitle { get; init; }
    internal required string DeleteOneTitle { get; init; }
    internal required string DeleteManyTitleFormat { get; init; }
    internal required string DeleteBody { get; init; }
    internal required string DeleteTooMany { get; init; }
    internal required string ClearNothingTitle { get; init; }
    internal required string ClearNothingMessage { get; init; }
    internal required string ClearTitle { get; init; }
    internal required string ClearBodyFormat { get; init; }
    internal required string ClearDoneTitle { get; init; }
    internal required string ClearDoneFormat { get; init; }
    internal required string ClearCompactionWarning { get; init; }
    internal required string ClearConflict { get; init; }

    internal static HistoryStrings Zh { get; } = new()
    {
        Title = "剪贴板历史",
        DatabaseNote = "· 独立数据库",
        DatabaseNoteToolTip = "原生迁移版使用独立数据库，不会自动打开已安装旧版的数据。",
        CaptureStart = "开始采集",
        CapturePause = "暂停采集",
        CaptureListening = "正在监听",
        CapturePaused = "采集已暂停",
        CapturePreparing = "正在准备采集",
        CaptureStatusFormat = "{0} · 已接收 {1} 条",
        CaptureStatusToolTip = "已接收 {0} 条 · 已忽略 {1} 条 · 失败 {2} 次。使用独立数据库。",
        CaptureUnknown = "采集状态未确认",
        CaptureUnknownFormat = "采集状态未确认：{0}",
        CaptureErrorFormat = "采集服务错误：{0}",
        CaptureStatusTitle = "采集服务状态",
        ReturnTargetLabel = "返回窗口",
        ReturnTargetToolTip = "返回上次输入窗口，不发送粘贴按键",
        StartTargetLabel = "启用目标跟踪",
        StartTargetToolTip = "开始记住上次输入窗口，供返回和粘贴使用",
        TargetReading = "正在读取上次输入窗口",
        TargetFormat = "上次输入窗口：{0}。返回窗口不会粘贴内容。",
        TargetNone = "尚无输入目标：先切换到需要粘贴的应用，再返回 TieZ。",
        TargetStopped = "窗口跟踪已停止，可点击启用目标跟踪。",
        TargetErrorFormat = "目标跟踪不可用：{0}",
        TargetErrorSuffix = " 服务错误：{0}",
        ReturnedTitle = "已返回目标窗口",
        ReturnedMessage = "未发送粘贴按键。",
        SearchPlaceholder = "搜索内容、来源、标签或识别结果",
        SearchName = "搜索剪贴板历史",
        TypeFilterName = "内容类型",
        TypeAll = "全部类型",
        TypeText = "文本",
        TypeCode = "代码",
        TypeLink = "链接",
        TypeRichText = "富文本",
        TypeImage = "图片",
        TypeFile = "文件",
        TypeVideo = "视频",
        TagFilterPlaceholder = "按精确标签筛选",
        TagFilterName = "按精确标签筛选",
        PinnedOnly = "仅置顶",
        TagOnly = "仅搜索标签",
        SortName = "记录排序",
        SortToolTip = "记录排序",
        SortPinned = "置顶优先",
        SortTime = "时间（新到旧）",
        SortUsage = "使用频率（高到低）",
        LayoutName = "记录布局",
        LayoutToolTip = "列表或卡片布局",
        LayoutList = "列表",
        LayoutCards = "卡片",
        ReloadLayoutName = "重读布局",
        ReloadLayoutToolTip = "重新读取已保存的布局",
        LayoutSaveFailed = "布局未保存，已保留原布局。{0} 可重读布局后重试。",
        SelectionHint = "选择记录以复制、粘贴或编辑",
        SelectionCountFormat = "已选择 {0} 条记录",
        DetailTitleDefault = "选择一条记录",
        DetailTitleFormat = "已选择 {0} 条",
        DetailEmptyHint = "选择一条记录查看详情",
        DetailLoading = "正在载入记录…",
        DetailMultipleHint = "已选择多条记录，可统一删除。",
        DetailStaleHint = "记录已更新，请刷新后查看。",
        DetailSensitiveHint = "敏感内容已隐藏，确认后可显示。",
        DetailHeading = "记录详情",
        DetailTextName = "所选记录的原始文本",
        RevealSensitive = "显示敏感内容",
        ImageUnavailable = "图片预览不可用，原始内容未修改。",
        MetadataSeparator = " · ",
        MetadataPinned = "置顶",
        MetadataUsageFormat = "已使用 {0} 次",
        MetadataSessionOnly = "仅本次会话",
        SourceManual = "手动添加",
        PreviewStale = "记录已更新 · 请刷新",
        PreviewSensitive = "敏感内容 · 选择后可显示",
        PreviewImageHint = "图片 · 选择后查看预览",
        ListName = "剪贴板历史记录",
        CardsName = "剪贴板历史卡片",
        ListStatusFormat = "已载入 {0} 条 · 本地 {1} 条 · 会话 {2} 条",
        LoadMore = "载入更多",
        AddTitle = "新增条目",
        EditTitle = "编辑内容",
        InputPlaceholder = "输入内容",
        PasteToolTip = "按原始格式粘贴到上次输入窗口",
        CopyToolTip = "复制原始格式",
        OpenToolTip = "使用应用打开；临时副本的修改不会回写历史",
        PreviewLabel = "紧凑预览",
        RefreshLabel = "刷新",
        RefreshPending = "刷新（有更新）",
        RefreshHistory = "刷新历史",
        ClearHistory = "清空普通历史…",
        DeleteSelectedFormat = "删除所选（{0}）",
        PinAction = "置顶",
        UnpinAction = "取消置顶",
        CardCopyToolTip = "复制此条记录的原始格式（敏感记录需先显示详情）",
        CardCopyName = "复制此条记录的原始格式",
        CardCopyPlainToolTip = "复制此条记录为纯文本（敏感记录需先显示详情）",
        CardCopyPlainName = "复制此条记录为纯文本",
        CardOpenToolTip = "使用应用打开此记录（敏感记录需先显示详情；临时副本修改不会回写历史）",
        CardOpenName = "使用应用打开此条记录",
        ErrorTitle = "操作未完成",
        MutateBlocked = "历史记录已更新，请取消后刷新再修改。草稿将保留在当前对话框中。",
        CopyCancelled = "复制请求已取消。",
        CopySuccessTitle = "已复制",
        CopySuccessPlain = "已复制为纯文本。",
        CopySuccessOriginal = "已按原始格式复制到剪贴板。",
        CopyConflict = "记录已变化，本次未复制。请刷新后重试。",
        OpenCancelled = "打开请求已取消。",
        OpenUnverified = "无法核实打开结果，请先检查外部应用，避免重复打开。",
        OpenConflict = "记录已变化，本次未打开。请刷新后重试。",
        OpenNeedsReveal = "请先单独选中敏感记录并显示详情，再打开。",
        OpenFailed = "无法打开记录，请检查默认应用或应用设置后重试。",
        OpenSuccessTitle = "已打开",
        OpenUsageTitle = "已打开，使用次数未更新",
        OpenExportedNoWriteback = "临时副本已打开，修改不会回写历史。",
        OpenExported = "临时副本已打开。",
        OpenHanded = "已交给系统或所选应用打开。",
        OpenUsageWarning = " 使用次数未能更新，请勿为重试计数而重复打开。",
        PasteSentTitle = "粘贴按键已发送",
        PasteSentMessage = "目标程序的接收结果需在目标窗口确认。",
        TagPickerTitle = "选择标签",
        DeleteOneTitle = "删除这条记录？",
        DeleteManyTitleFormat = "删除所选 {0} 条记录？",
        DeleteBody = "删除后无法撤销。置顶或带标签的所选记录也会删除。",
        DeleteTooMany = "一次最多删除 1000 条，请缩小选择范围。",
        ClearNothingTitle = "没有可清空的普通历史",
        ClearNothingMessage = "置顶和带标签的记录已保留。",
        ClearTitle = "清空普通历史？",
        ClearBodyFormat = "将删除 {0} 条本地记录和 {1} 条会话记录。\n保留所有置顶和带标签的记录。此操作作用于整个历史，不受搜索或筛选影响，删除后无法撤销。",
        ClearDoneTitle = "已清空普通历史",
        ClearDoneFormat = "已删除 {0} 条记录，置顶和带标签的记录已保留。",
        ClearCompactionWarning = " 数据库空间压缩未完成；历史清空已经生效。",
        ClearConflict = "历史记录已变化，本次清空未执行。请刷新后重新查看清空范围。"
    };

    internal static HistoryStrings Tw { get; } = new()
    {
        Title = "剪貼簿歷史",
        DatabaseNote = "· 獨立資料庫",
        DatabaseNoteToolTip = "原生遷移版使用獨立資料庫，不會自動開啟已安裝舊版的資料。",
        CaptureStart = "開始擷取",
        CapturePause = "暫停擷取",
        CaptureListening = "正在監聽",
        CapturePaused = "擷取已暫停",
        CapturePreparing = "正在準備擷取",
        CaptureStatusFormat = "{0} · 已接收 {1} 筆",
        CaptureStatusToolTip = "已接收 {0} 筆 · 已略過 {1} 筆 · 失敗 {2} 次。使用獨立資料庫。",
        CaptureUnknown = "擷取狀態未確認",
        CaptureUnknownFormat = "擷取狀態未確認：{0}",
        CaptureErrorFormat = "擷取服務錯誤：{0}",
        CaptureStatusTitle = "擷取服務狀態",
        ReturnTargetLabel = "返回視窗",
        ReturnTargetToolTip = "返回上次輸入視窗，不傳送貼上按鍵",
        StartTargetLabel = "啟用目標追蹤",
        StartTargetToolTip = "開始記住上次輸入視窗，供返回和貼上使用",
        TargetReading = "正在讀取上次輸入視窗",
        TargetFormat = "上次輸入視窗：{0}。返回視窗不會貼上內容。",
        TargetNone = "尚無輸入目標：先切換到需要貼上的應用程式，再返回 TieZ。",
        TargetStopped = "視窗追蹤已停止，可點選啟用目標追蹤。",
        TargetErrorFormat = "目標追蹤無法使用：{0}",
        TargetErrorSuffix = " 服務錯誤：{0}",
        ReturnedTitle = "已返回目標視窗",
        ReturnedMessage = "未傳送貼上按鍵。",
        SearchPlaceholder = "搜尋內容、來源、標籤或辨識結果",
        SearchName = "搜尋剪貼簿歷史",
        TypeFilterName = "內容類型",
        TypeAll = "全部類型",
        TypeText = "文字",
        TypeCode = "程式碼",
        TypeLink = "連結",
        TypeRichText = "富文字",
        TypeImage = "圖片",
        TypeFile = "檔案",
        TypeVideo = "影片",
        TagFilterPlaceholder = "依精確標籤篩選",
        TagFilterName = "依精確標籤篩選",
        PinnedOnly = "僅置頂",
        TagOnly = "僅搜尋標籤",
        SortName = "記錄排序",
        SortToolTip = "記錄排序",
        SortPinned = "置頂優先",
        SortTime = "時間（新到舊）",
        SortUsage = "使用頻率（高到低）",
        LayoutName = "記錄版面",
        LayoutToolTip = "清單或卡片版面",
        LayoutList = "清單",
        LayoutCards = "卡片",
        ReloadLayoutName = "重新讀取版面",
        ReloadLayoutToolTip = "重新讀取已儲存的版面",
        LayoutSaveFailed = "版面未儲存，已保留原版面。{0} 可重新讀取版面後再試。",
        SelectionHint = "選擇記錄以複製、貼上或編輯",
        SelectionCountFormat = "已選擇 {0} 筆記錄",
        DetailTitleDefault = "選擇一筆記錄",
        DetailTitleFormat = "已選擇 {0} 筆",
        DetailEmptyHint = "選擇一筆記錄以檢視詳細資料",
        DetailLoading = "正在載入記錄…",
        DetailMultipleHint = "已選擇多筆記錄，可一次刪除。",
        DetailStaleHint = "記錄已更新，請重新整理後檢視。",
        DetailSensitiveHint = "敏感內容已隱藏，確認後可顯示。",
        DetailHeading = "記錄詳細資料",
        DetailTextName = "所選記錄的原始文字",
        RevealSensitive = "顯示敏感內容",
        ImageUnavailable = "圖片預覽無法使用，原始內容未修改。",
        MetadataSeparator = " · ",
        MetadataPinned = "置頂",
        MetadataUsageFormat = "已使用 {0} 次",
        MetadataSessionOnly = "僅本次工作階段",
        SourceManual = "手動新增",
        PreviewStale = "記錄已更新 · 請重新整理",
        PreviewSensitive = "敏感內容 · 選擇後可顯示",
        PreviewImageHint = "圖片 · 選擇後檢視預覽",
        ListName = "剪貼簿歷史記錄",
        CardsName = "剪貼簿歷史卡片",
        ListStatusFormat = "已載入 {0} 筆 · 本機 {1} 筆 · 工作階段 {2} 筆",
        LoadMore = "載入更多",
        AddTitle = "新增項目",
        EditTitle = "編輯內容",
        InputPlaceholder = "輸入內容",
        PasteToolTip = "依原始格式貼上到上次輸入視窗",
        CopyToolTip = "複製原始格式",
        OpenToolTip = "使用應用程式開啟；臨時副本的修改不會回寫歷史",
        PreviewLabel = "精簡預覽",
        RefreshLabel = "重新整理",
        RefreshPending = "重新整理（有更新）",
        RefreshHistory = "重新整理歷史",
        ClearHistory = "清除一般歷史…",
        DeleteSelectedFormat = "刪除所選（{0}）",
        PinAction = "置頂",
        UnpinAction = "取消置頂",
        CardCopyToolTip = "複製此筆記錄的原始格式（敏感記錄需先顯示詳細資料）",
        CardCopyName = "複製此筆記錄的原始格式",
        CardCopyPlainToolTip = "複製此筆記錄為純文字（敏感記錄需先顯示詳細資料）",
        CardCopyPlainName = "複製此筆記錄為純文字",
        CardOpenToolTip = "使用應用程式開啟此筆記錄（敏感記錄需先顯示詳細資料；臨時副本的修改不會回寫歷史）",
        CardOpenName = "使用應用程式開啟此筆記錄",
        ErrorTitle = "作業未完成",
        MutateBlocked = "歷史記錄已更新，請取消後重新整理再修改。草稿將保留在目前對話方塊中。",
        CopyCancelled = "複製要求已取消。",
        CopySuccessTitle = "已複製",
        CopySuccessPlain = "已複製為純文字。",
        CopySuccessOriginal = "已依原始格式複製到剪貼簿。",
        CopyConflict = "記錄已變更，本次未複製。請重新整理後再試。",
        OpenCancelled = "開啟要求已取消。",
        OpenUnverified = "無法核實開啟結果，請先檢查外部應用程式，避免重複開啟。",
        OpenConflict = "記錄已變更，本次未開啟。請重新整理後再試。",
        OpenNeedsReveal = "請先單獨選取敏感記錄並顯示詳細資料，再開啟。",
        OpenFailed = "無法開啟記錄，請檢查預設應用程式或應用程式設定後再試。",
        OpenSuccessTitle = "已開啟",
        OpenUsageTitle = "已開啟，使用次數未更新",
        OpenExportedNoWriteback = "臨時副本已開啟，修改不會回寫歷史。",
        OpenExported = "臨時副本已開啟。",
        OpenHanded = "已交給系統或所選應用程式開啟。",
        OpenUsageWarning = " 使用次數未能更新，請勿為了重試計數而重複開啟。",
        PasteSentTitle = "已傳送貼上按鍵",
        PasteSentMessage = "目標程式的接收結果需在目標視窗確認。",
        TagPickerTitle = "選擇標籤",
        DeleteOneTitle = "刪除此筆記錄？",
        DeleteManyTitleFormat = "刪除所選 {0} 筆記錄？",
        DeleteBody = "刪除後無法復原。置頂或帶有標籤的所選記錄也會刪除。",
        DeleteTooMany = "一次最多刪除 1000 筆，請縮小選取範圍。",
        ClearNothingTitle = "沒有可清除的一般歷史",
        ClearNothingMessage = "置頂和帶有標籤的記錄已保留。",
        ClearTitle = "清除一般歷史？",
        ClearBodyFormat = "將刪除 {0} 筆本機記錄和 {1} 筆工作階段記錄。\n保留所有置頂和帶有標籤的記錄。此操作套用於整個歷史，不受搜尋或篩選影響，刪除後無法復原。",
        ClearDoneTitle = "已清除一般歷史",
        ClearDoneFormat = "已刪除 {0} 筆記錄，置頂和帶有標籤的記錄已保留。",
        ClearCompactionWarning = " 資料庫空間壓縮未完成；歷史清除已經生效。",
        ClearConflict = "歷史記錄已變更，本次清除未執行。請重新整理後重新查看清除範圍。"
    };

    internal static HistoryStrings En { get; } = new()
    {
        Title = "Clipboard history",
        DatabaseNote = "- Separate database",
        DatabaseNoteToolTip = "The native migration uses a separate database and never opens data from the installed legacy app.",
        CaptureStart = "Start capture",
        CapturePause = "Pause capture",
        CaptureListening = "Listening",
        CapturePaused = "Capture paused",
        CapturePreparing = "Preparing capture",
        CaptureStatusFormat = "{0} - {1} accepted",
        CaptureStatusToolTip = "Accepted {0} - ignored {1} - failed {2}. Separate database.",
        CaptureUnknown = "Capture state unconfirmed",
        CaptureUnknownFormat = "Capture state unconfirmed: {0}",
        CaptureErrorFormat = "Capture service error: {0}",
        CaptureStatusTitle = "Capture service status",
        ReturnTargetLabel = "Return to window",
        ReturnTargetToolTip = "Return to the last input window without sending a paste keystroke",
        StartTargetLabel = "Start target tracking",
        StartTargetToolTip = "Start remembering the last input window for return and paste",
        TargetReading = "Reading the last input window",
        TargetFormat = "Last input window: {0}. Returning does not paste anything.",
        TargetNone = "No input target yet: switch to the app you want to paste into, then return to TieZ.",
        TargetStopped = "Window tracking stopped; start target tracking to resume.",
        TargetErrorFormat = "Target tracking unavailable: {0}",
        TargetErrorSuffix = " Service error: {0}",
        ReturnedTitle = "Returned to the target window",
        ReturnedMessage = "No paste keystroke was sent.",
        SearchPlaceholder = "Search content, source, tags or OCR text",
        SearchName = "Search clipboard history",
        TypeFilterName = "Content type",
        TypeAll = "All types",
        TypeText = "Text",
        TypeCode = "Code",
        TypeLink = "Link",
        TypeRichText = "Rich text",
        TypeImage = "Image",
        TypeFile = "File",
        TypeVideo = "Video",
        TagFilterPlaceholder = "Filter by exact tag",
        TagFilterName = "Filter by exact tag",
        PinnedOnly = "Pinned only",
        TagOnly = "Search tags only",
        SortName = "Record order",
        SortToolTip = "Record order",
        SortPinned = "Pinned first",
        SortTime = "Newest first",
        SortUsage = "Most used first",
        LayoutName = "Record layout",
        LayoutToolTip = "List or card layout",
        LayoutList = "List",
        LayoutCards = "Cards",
        ReloadLayoutName = "Reload layout",
        ReloadLayoutToolTip = "Reload the saved layout",
        LayoutSaveFailed = "The layout was not saved; the previous layout is kept. {0} Reload the layout and try again.",
        SelectionHint = "Select a record to copy, paste or edit",
        SelectionCountFormat = "{0} selected",
        DetailTitleDefault = "Select a record",
        DetailTitleFormat = "{0} selected",
        DetailEmptyHint = "Select a record to see its details",
        DetailLoading = "Loading the record...",
        DetailMultipleHint = "Multiple records selected; you can delete them together.",
        DetailStaleHint = "The record changed; refresh to see it.",
        DetailSensitiveHint = "Sensitive content is hidden; confirm to show it.",
        DetailHeading = "Record details",
        DetailTextName = "Original text of the selected record",
        RevealSensitive = "Show sensitive content",
        ImageUnavailable = "Image preview unavailable; the original content is unchanged.",
        MetadataSeparator = " - ",
        MetadataPinned = "Pinned",
        MetadataUsageFormat = "Used {0} times",
        MetadataSessionOnly = "This session only",
        SourceManual = "Added manually",
        PreviewStale = "Record changed - refresh",
        PreviewSensitive = "Sensitive content - select to show",
        PreviewImageHint = "Image - select to preview",
        ListName = "Clipboard history records",
        CardsName = "Clipboard history cards",
        ListStatusFormat = "Loaded {0} - local {1} - session {2}",
        LoadMore = "Load more",
        AddTitle = "Add record",
        EditTitle = "Edit record",
        InputPlaceholder = "Enter text",
        PasteToolTip = "Paste into the last input window in its original format",
        CopyToolTip = "Copy in the original format",
        OpenToolTip = "Open with an app; edits to the temporary copy are not written back",
        PreviewLabel = "Compact preview",
        RefreshLabel = "Refresh",
        RefreshPending = "Refresh (updates)",
        RefreshHistory = "Refresh history",
        ClearHistory = "Clear ordinary history...",
        DeleteSelectedFormat = "Delete selected ({0})",
        PinAction = "Pin",
        UnpinAction = "Unpin",
        CardCopyToolTip = "Copy this record in its original format (show details first for sensitive records)",
        CardCopyName = "Copy this record in its original format",
        CardCopyPlainToolTip = "Copy this record as plain text (show details first for sensitive records)",
        CardCopyPlainName = "Copy this record as plain text",
        CardOpenToolTip = "Open this record with an app (show details first for sensitive records; edits to the temporary copy are not written back)",
        CardOpenName = "Open this record with an app",
        ErrorTitle = "The operation did not complete",
        MutateBlocked = "History changed; cancel, refresh and edit again. Your draft stays in this dialog.",
        CopyCancelled = "The copy request was cancelled.",
        CopySuccessTitle = "Copied",
        CopySuccessPlain = "Copied as plain text.",
        CopySuccessOriginal = "Copied to the clipboard in the original format.",
        CopyConflict = "The record changed and was not copied. Refresh and try again.",
        OpenCancelled = "The open request was cancelled.",
        OpenUnverified = "The open result could not be verified; check the external app before opening it again.",
        OpenConflict = "The record changed and was not opened. Refresh and try again.",
        OpenNeedsReveal = "Select the sensitive record alone and show its details before opening it.",
        OpenFailed = "The record could not be opened; check the default app or app settings and try again.",
        OpenSuccessTitle = "Opened",
        OpenUsageTitle = "Opened; the usage count was not updated",
        OpenExportedNoWriteback = "The temporary copy is open; edits are not written back to history.",
        OpenExported = "The temporary copy is open.",
        OpenHanded = "Handed to the system or the selected app.",
        OpenUsageWarning = " The usage count was not updated; do not open the record again just to retry the count.",
        PasteSentTitle = "Paste keystroke sent",
        PasteSentMessage = "Confirm the result in the target window.",
        TagPickerTitle = "Select tags",
        DeleteOneTitle = "Delete this record?",
        DeleteManyTitleFormat = "Delete the {0} selected records?",
        DeleteBody = "This cannot be undone. Pinned or tagged selections are deleted too.",
        DeleteTooMany = "Delete at most 1000 records at once; narrow the selection.",
        ClearNothingTitle = "No ordinary history to clear",
        ClearNothingMessage = "Pinned and tagged records are kept.",
        ClearTitle = "Clear ordinary history?",
        ClearBodyFormat = "This deletes {0} local records and {1} session records.\nAll pinned and tagged records are kept. It applies to the whole history regardless of search or filters and cannot be undone.",
        ClearDoneTitle = "Ordinary history cleared",
        ClearDoneFormat = "Deleted {0} records; pinned and tagged records are kept.",
        ClearCompactionWarning = " Database compaction did not finish; the clear already took effect.",
        ClearConflict = "History changed and nothing was cleared. Refresh and review the clear scope again."
    };

    internal static HistoryStrings Current => LanguageService.Current switch
    {
        Language.TraditionalChinese => Tw,
        Language.English => En,
        _ => Zh
    };
}