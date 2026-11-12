namespace TieZ.Native;

/// <summary>
/// Every localized string of the tag directory and the tag picker. Tag names, colors and counts stay data: only the
/// chrome around them is translated here. Values that embed data keep explicit {n} placeholders and are formatted at
/// the call site, so a count or a name is never baked into a language table.
/// </summary>
internal sealed class TagStrings
{
    internal required string Title { get; init; }
    internal required string NewTagHeader { get; init; }
    internal required string NewTagPlaceholder { get; init; }
    internal required string CreateTag { get; init; }
    internal required string SearchPlaceholder { get; init; }
    internal required string SearchName { get; init; }
    internal required string ClearSearch { get; init; }
    internal required string SortHeader { get; init; }
    internal required string SortByCount { get; init; }
    internal required string SortByName { get; init; }
    internal required string ListName { get; init; }
    internal required string MoreTags { get; init; }
    internal required string StatusLoading { get; init; }
    internal required string StatusSaving { get; init; }
    internal required string StatusLoaded { get; init; }
    internal required string StatusMore { get; init; }
    internal required string StatusSearch { get; init; }
    internal required string Separator { get; init; }
    internal required string NoTagSelected { get; init; }
    internal required string FilterByTag { get; init; }
    internal required string DeleteTag { get; init; }
    internal required string DeleteTagHint { get; init; }
    internal required string RenameHeader { get; init; }
    internal required string RenameButton { get; init; }
    internal required string RenameHint { get; init; }
    internal required string ColorHeader { get; init; }
    internal required string ColorPlaceholder { get; init; }
    internal required string UseDefaultColor { get; init; }
    internal required string SaveColor { get; init; }
    internal required string EditorHint { get; init; }
    internal required string ColorDefaultHint { get; init; }
    internal required string ColorAlphaHint { get; init; }
    internal required string ColorInvalidHint { get; init; }
    internal required string ColorUnsupportedHint { get; init; }
    internal required string DraftBlocked { get; init; }
    internal required string SingleLineName { get; init; }
    internal required string ReadFailed { get; init; }
    internal required string TagSaved { get; init; }
    internal required string TagUnchanged { get; init; }
    internal required string SaveUnrefreshed { get; init; }
    internal required string SaveFailed { get; init; }
    internal required string Renamed { get; init; }
    internal required string RenameUnrefreshed { get; init; }
    internal required string RenameFailed { get; init; }
    internal required string Deleted { get; init; }
    internal required string DeleteUnrefreshed { get; init; }
    internal required string CleanupIncomplete { get; init; }
    internal required string DeleteFailed { get; init; }
    internal required string StaleNotice { get; init; }
    internal required string PollFailed { get; init; }
    internal required string RenameTitle { get; init; }
    internal required string RenameBody { get; init; }
    internal required string RenameConfirm { get; init; }
    internal required string DeleteTitle { get; init; }
    internal required string DeleteBody { get; init; }
    internal required string DeleteConfirm { get; init; }
    internal required string TagDetails { get; init; }
    internal required string SensitiveSuffix { get; init; }
    internal required string UnnamedTag { get; init; }
    internal required string PickerIntro { get; init; }
    internal required string PickerSearchName { get; init; }
    internal required string PickerChoicesName { get; init; }
    internal required string PickerMore { get; init; }
    internal required string PickerSelectedHeader { get; init; }
    internal required string PickerSelectedName { get; init; }
    internal required string PickerRemove { get; init; }
    internal required string PickerNewNamePlaceholder { get; init; }
    internal required string PickerAdd { get; init; }
    internal required string PickerSensitiveHint { get; init; }
    internal required string PickerReload { get; init; }
    internal required string PickerStale { get; init; }
    internal required string PickerStaleSuffix { get; init; }
    internal required string PickerRetrySuffix { get; init; }
    internal required string PickerNeedName { get; init; }
    internal required string PickerAddBeforeSave { get; init; }
    internal required string PickerSaveIncomplete { get; init; }
    internal static TagStrings Zh { get; } = new()
    {
        Title = "标签",
        NewTagHeader = "新标签",
        NewTagPlaceholder = "输入标签名称",
        CreateTag = "添加标签",
        SearchPlaceholder = "搜索全部标签（不区分大小写）",
        SearchName = "搜索标签目录",
        ClearSearch = "清除搜索",
        SortHeader = "标签排序",
        SortByCount = "历史数量（多到少）",
        SortByName = "名称",
        ListName = "标签目录",
        MoreTags = "载入更多标签",
        StatusLoading = "正在读取标签…",
        StatusSaving = "正在保存…",
        StatusLoaded = "已载入 {0} 个标签",
        StatusMore = "可继续载入",
        StatusSearch = "搜索\u201C{0}\u201D",
        Separator = " · ",
        NoTagSelected = "选择一个标签",
        FilterByTag = "查看此标签的历史",
        DeleteTag = "删除标签及其历史…",
        DeleteTagHint = "删除标签会同时删除所有带此标签的历史记录，包括置顶和带其他标签的记录。",
        RenameHeader = "新名称",
        RenameButton = "重命名或合并…",
        RenameHint = "名称已存在时会合并，保留目标标签颜色。敏感标签名称会改变内容保护状态。",
        ColorHeader = "标签颜色",
        ColorPlaceholder = "默认颜色；可输入 HEX、RGB、RGBA 或颜色名称",
        UseDefaultColor = "使用默认颜色",
        SaveColor = "保存颜色",
        EditorHint = "切换页面会保留草稿。颜色与名称请分别保存；刷新会放弃当前标签修改，并保留尚未添加的新标签名称。",
        ColorDefaultHint = "使用默认颜色。",
        ColorAlphaHint = "支持透明度；使用颜色选择器可直接调整。",
        ColorInvalidHint = "无法解析此颜色。请输入 HEX、RGB、RGBA 或颜色名称后再保存。",
        ColorUnsupportedHint = "此颜色格式暂不支持预览；未修改的原值会保留，也可选择新的颜色。",
        DraftBlocked = "请先保存当前修改，或刷新以放弃草稿。",
        SingleLineName = "标签名称请保持在一行。",
        ReadFailed = "读取标签失败，当前内容和草稿已保留：{0}",
        TagSaved = "标签已保存。",
        TagUnchanged = "内容未变化，无需重复保存。",
        SaveUnrefreshed = "保存已完成；目录尚未刷新，返回标签页后刷新查看。",
        SaveFailed = "未能保存，草稿已保留。请刷新后重试：{0}",
        Renamed = "标签已重命名或合并。",
        RenameUnrefreshed = "标签已修改；请刷新目录查看。",
        RenameFailed = "重命名失败，名称草稿已保留。请刷新后重试：{0}",
        Deleted = "标签已删除，同时删除 {0} 条历史（已保存 {1}、本次会话 {2}）。",
        DeleteUnrefreshed = " 目录尚未刷新，请刷新查看。",
        CleanupIncomplete = " 附件清理未完成；历史删除已生效。",
        DeleteFailed = "未能删除，当前条目和输入已保留。请刷新并重新确认：{0}",
        StaleNotice = "标签或历史已有更新。当前草稿已保留；刷新后可继续操作。",
        PollFailed = "检查更新失败：{0}",
        RenameTitle = "重命名或合并标签？",
        RenameBody = "\u201C{0}\u201D \u2192 \u201C{1}\u201D\n将更新此标签的 {2} 条历史。目标标签已存在时会合并，保留目标标签颜色。\n敏感标签（sensitive、密码、password，不区分英文大小写）会改变相关内容的保护状态；进入敏感状态会清除 OCR。",
        RenameConfirm = "确认重命名",
        DeleteTitle = "删除标签及其历史？",
        DeleteBody = "删除标签\u201C{0}\u201D，并永久删除带此标签的全部 {1} 条历史：已保存 {2} 条、本次会话 {3} 条。\n置顶或同时带其他标签的匹配记录也会整条删除；其关联数据和不再被引用的应用附件将清理。\n此操作无法撤销；仅删除标签标记并保留记录不是此操作的行为。",
        DeleteConfirm = "删除标签及历史",
        TagDetails = "{0} 条 · 已保存 {1} · 本次会话 {2}",
        SensitiveSuffix = " · 敏感内容",
        UnnamedTag = "（未命名标签）",
        PickerIntro = "从全部标签中选择，或添加新名称。保存前不会更改记录。",
        PickerSearchName = "搜索可选标签",
        PickerChoicesName = "可选标签",
        PickerMore = "载入更多",
        PickerSelectedHeader = "已选标签（包含其他页）",
        PickerSelectedName = "已选标签",
        PickerRemove = "移除所选",
        PickerNewNamePlaceholder = "新标签名称",
        PickerAdd = "加入选择",
        PickerSensitiveHint = "选中名为 sensitive、密码或 password 的标签会保护内容并清除 OCR；移除最后一个敏感标签会解除保护。",
        PickerReload = "重新读取（放弃选择）",
        PickerStale = "历史或标签目录已更新。选择已保留，请重新读取后再保存。",
        PickerStaleSuffix = " 选择已保留，请重新读取。",
        PickerRetrySuffix = " 选择已保留，可重试。",
        PickerNeedName = "请输入单行标签名称。",
        PickerAddBeforeSave = "请先将新名称加入选择，或清空输入后保存。",
        PickerSaveIncomplete = "保存未完成，选择已保留。请稍后重试或重新读取。"
    };
    internal static TagStrings Tw { get; } = new()
    {
        Title = "標籤",
        NewTagHeader = "新標籤",
        NewTagPlaceholder = "輸入標籤名稱",
        CreateTag = "新增標籤",
        SearchPlaceholder = "搜尋全部標籤（不分大小寫）",
        SearchName = "搜尋標籤目錄",
        ClearSearch = "清除搜尋",
        SortHeader = "標籤排序",
        SortByCount = "歷史筆數（多到少）",
        SortByName = "名稱",
        ListName = "標籤目錄",
        MoreTags = "載入更多標籤",
        StatusLoading = "正在讀取標籤…",
        StatusSaving = "正在儲存…",
        StatusLoaded = "已載入 {0} 個標籤",
        StatusMore = "可繼續載入",
        StatusSearch = "搜尋「{0}」",
        Separator = " · ",
        NoTagSelected = "選擇一個標籤",
        FilterByTag = "檢視此標籤的歷史",
        DeleteTag = "刪除標籤及其歷史…",
        DeleteTagHint = "刪除標籤會同時刪除所有帶此標籤的歷史記錄，包括置頂和帶其他標籤的記錄。",
        RenameHeader = "新名稱",
        RenameButton = "重新命名或合併…",
        RenameHint = "名稱已存在時會合併，保留目標標籤顏色。敏感標籤名稱會改變內容保護狀態。",
        ColorHeader = "標籤顏色",
        ColorPlaceholder = "預設顏色；可輸入 HEX、RGB、RGBA 或顏色名稱",
        UseDefaultColor = "使用預設顏色",
        SaveColor = "儲存顏色",
        EditorHint = "切換頁面會保留草稿。顏色與名稱請分別儲存；重新整理會放棄目前標籤修改，並保留尚未新增的新標籤名稱。",
        ColorDefaultHint = "使用預設顏色。",
        ColorAlphaHint = "支援透明度；使用顏色選擇器可直接調整。",
        ColorInvalidHint = "無法解析此顏色。請輸入 HEX、RGB、RGBA 或顏色名稱後再儲存。",
        ColorUnsupportedHint = "此顏色格式暫不支援預覽；未修改的原值會保留，也可選擇新的顏色。",
        DraftBlocked = "請先儲存目前修改，或重新整理以放棄草稿。",
        SingleLineName = "標籤名稱請保持在一行。",
        ReadFailed = "讀取標籤失敗，目前內容和草稿已保留：{0}",
        TagSaved = "標籤已儲存。",
        TagUnchanged = "內容未變更，無需重複儲存。",
        SaveUnrefreshed = "儲存已完成；目錄尚未重新整理，返回標籤頁後重新整理檢視。",
        SaveFailed = "未能儲存，草稿已保留。請重新整理後重試：{0}",
        Renamed = "標籤已重新命名或合併。",
        RenameUnrefreshed = "標籤已修改；請重新整理目錄檢視。",
        RenameFailed = "重新命名失敗，名稱草稿已保留。請重新整理後重試：{0}",
        Deleted = "標籤已刪除，同時刪除 {0} 筆歷史（已儲存 {1}、本次工作階段 {2}）。",
        DeleteUnrefreshed = " 目錄尚未重新整理，請重新整理檢視。",
        CleanupIncomplete = " 附件清理未完成；歷史刪除已生效。",
        DeleteFailed = "未能刪除，目前項目和輸入已保留。請重新整理並重新確認：{0}",
        StaleNotice = "標籤或歷史已有更新。目前草稿已保留；重新整理後可繼續操作。",
        PollFailed = "檢查更新失敗：{0}",
        RenameTitle = "重新命名或合併標籤？",
        RenameBody = "「{0}」\u2192「{1}」\n將更新此標籤的 {2} 筆歷史。目標標籤已存在時會合併，保留目標標籤顏色。\n敏感標籤（sensitive、密碼、password，不分英文大小寫）會改變相關內容的保護狀態；進入敏感狀態會清除 OCR。",
        RenameConfirm = "確認重新命名",
        DeleteTitle = "刪除標籤及其歷史？",
        DeleteBody = "刪除標籤「{0}」，並永久刪除帶此標籤的全部 {1} 筆歷史：已儲存 {2} 筆、本次工作階段 {3} 筆。\n置頂或同時帶其他標籤的符合記錄也會整筆刪除；其關聯資料和不再被引用的應用程式附件將清理。\n此操作無法復原；僅刪除標籤標記並保留記錄不是此操作的行為。",
        DeleteConfirm = "刪除標籤及歷史",
        TagDetails = "{0} 筆 · 已儲存 {1} · 本次工作階段 {2}",
        SensitiveSuffix = " · 敏感內容",
        UnnamedTag = "（未命名標籤）",
        PickerIntro = "從全部標籤中選擇，或新增名稱。儲存前不會變更記錄。",
        PickerSearchName = "搜尋可選標籤",
        PickerChoicesName = "可選標籤",
        PickerMore = "載入更多",
        PickerSelectedHeader = "已選標籤（包含其他頁）",
        PickerSelectedName = "已選標籤",
        PickerRemove = "移除所選",
        PickerNewNamePlaceholder = "新標籤名稱",
        PickerAdd = "加入選擇",
        PickerSensitiveHint = "選取名為 sensitive、密碼或 password 的標籤會保護內容並清除 OCR；移除最後一個敏感標籤會解除保護。",
        PickerReload = "重新讀取（放棄選擇）",
        PickerStale = "歷史或標籤目錄已更新。選擇已保留，請重新讀取後再儲存。",
        PickerStaleSuffix = " 選擇已保留，請重新讀取。",
        PickerRetrySuffix = " 選擇已保留，可重試。",
        PickerNeedName = "請輸入單行標籤名稱。",
        PickerAddBeforeSave = "請先將新名稱加入選擇，或清空輸入後儲存。",
        PickerSaveIncomplete = "儲存未完成，選擇已保留。請稍後重試或重新讀取。"
    };
    internal static TagStrings En { get; } = new()
    {
        Title = "Tags",
        NewTagHeader = "New tag",
        NewTagPlaceholder = "Enter a tag name",
        CreateTag = "Add tag",
        SearchPlaceholder = "Search all tags (case-insensitive)",
        SearchName = "Search the tag directory",
        ClearSearch = "Clear search",
        SortHeader = "Sort tags",
        SortByCount = "History count (high to low)",
        SortByName = "Name",
        ListName = "Tag directory",
        MoreTags = "Load more tags",
        StatusLoading = "Reading tags...",
        StatusSaving = "Saving...",
        StatusLoaded = "{0} tags loaded",
        StatusMore = "more to load",
        StatusSearch = "search \"{0}\"",
        Separator = " - ",
        NoTagSelected = "Select a tag",
        FilterByTag = "View this tag's history",
        DeleteTag = "Delete tag and history...",
        DeleteTagHint = "Deleting a tag also deletes every record that carries it, including pinned records and records with other tags.",
        RenameHeader = "New name",
        RenameButton = "Rename or merge...",
        RenameHint = "An existing name is merged and keeps the target color. A sensitive name changes content protection.",
        ColorHeader = "Tag color",
        ColorPlaceholder = "Default color; enter HEX, RGB, RGBA or a color name",
        UseDefaultColor = "Use default color",
        SaveColor = "Save color",
        EditorHint = "Drafts survive page switches. Save the color and the name separately; refresh discards the tag edits and keeps an unadded new name.",
        ColorDefaultHint = "Using the default color.",
        ColorAlphaHint = "Alpha is supported; the color picker adjusts it directly.",
        ColorInvalidHint = "This color cannot be parsed. Enter HEX, RGB, RGBA or a color name before saving.",
        ColorUnsupportedHint = "This color format cannot be previewed; the unchanged value is kept and a new color can be chosen.",
        DraftBlocked = "Save the current changes first, or refresh to discard the draft.",
        SingleLineName = "Keep the tag name on one line.",
        ReadFailed = "Reading tags failed; the current content and draft are kept: {0}",
        TagSaved = "Tag saved.",
        TagUnchanged = "Nothing changed; no need to save again.",
        SaveUnrefreshed = "Saved; the directory is not refreshed yet, refresh it after returning to the tag page.",
        SaveFailed = "Saving failed; the draft is kept. Refresh and retry: {0}",
        Renamed = "Tag renamed or merged.",
        RenameUnrefreshed = "Tag changed; refresh the directory to see it.",
        RenameFailed = "Renaming failed; the name draft is kept. Refresh and retry: {0}",
        Deleted = "Tag deleted with {0} history records ({1} saved, {2} from this session).",
        DeleteUnrefreshed = " The directory is not refreshed yet; refresh to see it.",
        CleanupIncomplete = " Attachment cleanup did not finish; the history deletion already applied.",
        DeleteFailed = "Deleting failed; the current item and input are kept. Refresh and confirm again: {0}",
        StaleNotice = "Tags or history changed. The current draft is kept; refresh to continue.",
        PollFailed = "Checking for updates failed: {0}",
        RenameTitle = "Rename or merge tag?",
        RenameBody = "\"{0}\" -> \"{1}\"\nUpdates {2} history records of this tag. An existing target name is merged and keeps the target color.\nA sensitive tag (sensitive, password, any English case) changes content protection; entering the sensitive state clears OCR.",
        RenameConfirm = "Rename",
        DeleteTitle = "Delete tag and its history?",
        DeleteBody = "Deletes the tag \"{0}\" and permanently deletes all {1} history records with it: {2} saved and {3} from this session.\nPinned matches and matches that also carry other tags are deleted whole; linked data and unreferenced app attachments are cleaned up.\nThis cannot be undone; removing only the tag mark and keeping the records is not what this does.",
        DeleteConfirm = "Delete tag and history",
        TagDetails = "{0} entries - {1} saved - {2} this session",
        SensitiveSuffix = " - sensitive",
        UnnamedTag = "(unnamed tag)",
        PickerIntro = "Choose from all tags or add a new name. Nothing changes before saving.",
        PickerSearchName = "Search selectable tags",
        PickerChoicesName = "Selectable tags",
        PickerMore = "Load more",
        PickerSelectedHeader = "Selected tags (including other pages)",
        PickerSelectedName = "Selected tags",
        PickerRemove = "Remove selected",
        PickerNewNamePlaceholder = "New tag name",
        PickerAdd = "Add to selection",
        PickerSensitiveHint = "Selecting a tag named sensitive or password protects the content and clears OCR; removing the last sensitive tag removes the protection.",
        PickerReload = "Read again (discard selection)",
        PickerStale = "History or the tag directory changed. The selection is kept; read again before saving.",
        PickerStaleSuffix = " The selection is kept; read again.",
        PickerRetrySuffix = " The selection is kept; retry.",
        PickerNeedName = "Enter a one-line tag name.",
        PickerAddBeforeSave = "Add the new name to the selection first, or clear the input before saving.",
        PickerSaveIncomplete = "Saving did not finish; the selection is kept. Retry later or read again."
    };

    internal static TagStrings Current => LanguageService.Current switch
    {
        Language.TraditionalChinese => Tw,
        Language.English => En,
        _ => Zh
    };
}
