namespace TieZ.Native;

/// <summary>
/// Text of the advanced settings window and the three editors it hosts: the per-app policy editor, the cleanup
/// rule editor and the cleanup test preview.
/// </summary>
internal sealed class AdvancedStrings
{
    internal required string WindowTitle { get; init; }
    internal required string Heading { get; init; }
    internal required string Intro { get; init; }
    internal required string KindsHeader { get; init; }
    internal required string KindsPlaceholder { get; init; }
    internal required string PrivacyHeader { get; init; }
    internal required string PrivacyHint { get; init; }
    internal required string RulesExampleHint { get; init; }
    internal required string Save { get; init; }
    internal required string Reload { get; init; }
    internal required string RevisionFormat { get; init; }
    internal required string Conflict { get; init; }
    internal required string Saved { get; init; }
    internal required string SavedWarningFormat { get; init; }
    internal required string SaveConflict { get; init; }
    internal required string PreviewHeader { get; init; }
    internal required string PreviewIntro { get; init; }
    internal required string SampleHeader { get; init; }
    internal required string OutputHeader { get; init; }
    internal required string Run { get; init; }
    internal required string RunResultFormat { get; init; }
    internal required string IgnoredSuffix { get; init; }
    internal required string DoneSuffix { get; init; }
    internal required string InvalidRule { get; init; }
    internal required string LimitError { get; init; }
    internal required string FailureFormat { get; init; }
    internal required string PolicyHeading { get; init; }
    internal required string PolicyIntro { get; init; }
    internal required string PolicySelection { get; init; }
    internal required string PolicyAdd { get; init; }
    internal required string PolicyRemove { get; init; }
    internal required string MoveUp { get; init; }
    internal required string MoveDown { get; init; }
    internal required string PolicyEnabled { get; init; }
    internal required string AppNameHeader { get; init; }
    internal required string AppNamePlaceholder { get; init; }
    internal required string AppPathHeader { get; init; }
    internal required string AppPathPlaceholder { get; init; }
    internal required string ActionHeader { get; init; }
    internal required string ActionClean { get; init; }
    internal required string ActionIgnore { get; init; }
    internal required string TypesHeader { get; init; }
    internal required string TypeText { get; init; }
    internal required string TypeCode { get; init; }
    internal required string TypeUrl { get; init; }
    internal required string TypeRichText { get; init; }
    internal required string TypeImage { get; init; }
    internal required string TypeFile { get; init; }
    internal required string TypeVideo { get; init; }
    internal required string ListSeparator { get; init; }
    internal required string OtherTypesPrefix { get; init; }
    internal required string PolicyInvalid { get; init; }
    internal required string UnnamedApp { get; init; }
    internal required string DisabledSuffix { get; init; }
    internal required string PolicyRawHeader { get; init; }
    internal required string PolicyRawExpander { get; init; }
    internal required string PolicyIgnoreSemantics { get; init; }
    internal required string PolicyCleanSemantics { get; init; }
    internal required string PolicyMissingApp { get; init; }
    internal required string RuleSelection { get; init; }
    internal required string RulePatternHeader { get; init; }
    internal required string RuleReplacementHeader { get; init; }
    internal required string RuleReplacementPlaceholder { get; init; }
    internal required string RuleLabelHeader { get; init; }
    internal required string RuleActionReplace { get; init; }
    internal required string RuleAdd { get; init; }
    internal required string RuleRemove { get; init; }
    internal required string RuleHint { get; init; }
    internal required string RuleRawExpander { get; init; }
    internal required string RuleRawHeader { get; init; }
    internal required string RuleInputWarning { get; init; }
    internal required string RuleUnavailable { get; init; }

    internal static AdvancedStrings Zh { get; } = new()
    {
        WindowTitle = "TieZ · 高级设置",
        Heading = "高级设置",
        Intro = "规则在保存时由核心校验。关闭此窗口不影响剪贴板采集。",
        KindsHeader = "敏感匹配类别（逗号分隔）",
        KindsPlaceholder = "phone,idcard,email,secret",
        PrivacyHeader = "自定义敏感正则（每行一条）",
        PrivacyHint = "敏感规则使用正则表达式，一行一条；留空表示不添加自定义规则。",
        RulesExampleHint = "例如：foo => bar。空行和以 # 开头的注释行会忽略。",
        Save = "保存高级设置",
        Reload = "放弃草稿并重新载入",
        RevisionFormat = "已保存配置版本 {0}",
        Conflict = "配置已在其他窗口变化。当前草稿已保留，请重新载入后再修改。",
        Saved = "高级设置已保存并应用于采集。",
        SavedWarningFormat = "高级设置已保存，但运行时应用未完整完成：{0}",
        SaveConflict = "配置已在其他窗口变化，未保存。当前草稿已保留，请重新载入后再修改。",
        PreviewHeader = "规则测试预览",
        PreviewIntro = "仅测试这一组文字清理规则，不包含应用匹配、策略忽略动作、其他规则组或敏感检测。测试不会保存配置或操作剪贴板。最多 32 条规则、4096 次匹配、64 KiB 输出。",
        SampleHeader = "测试文本（最多 4096 字符）",
        OutputHeader = "处理结果",
        Run = "测试当前规则",
        RunResultFormat = "已执行 {0} 条规则，匹配 {1} 处。",
        IgnoredSuffix = "此规则组会忽略这条采集。",
        DoneSuffix = "测试完成，配置尚未因此保存。",
        InvalidRule = "规则含无效正则或缺少 =>，请修正后重试。",
        LimitError = "测试超过样本、规则、匹配或输出上限，请缩小样本或规则范围。",
        FailureFormat = "测试未完成：{0}",
        PolicyHeading = "按应用清理策略",
        PolicyIntro = "先按顺序执行匹配的应用策略，再执行全局清理；清理可累加，忽略会立即停止采集。名称和路径任一匹配即可；支持可执行文件名、应用 ID 和 /正则/。",
        PolicySelection = "应用策略（按执行顺序）",
        PolicyAdd = "添加策略",
        PolicyRemove = "删除此策略",
        MoveUp = "上移",
        MoveDown = "下移",
        PolicyEnabled = "启用此策略",
        AppNameHeader = "应用名称",
        AppNamePlaceholder = "例如：notepad",
        AppPathHeader = "路径、可执行文件名或应用 ID",
        AppPathPlaceholder = "例如：notepad.exe",
        ActionHeader = "匹配后执行",
        ActionClean = "清理文字",
        ActionIgnore = "忽略采集",
        TypesHeader = "适用内容类型",
        TypeText = "文本",
        TypeCode = "代码",
        TypeUrl = "链接",
        TypeRichText = "富文本",
        TypeImage = "图片",
        TypeFile = "文件",
        TypeVideo = "视频",
        ListSeparator = "、",
        OtherTypesPrefix = "保留的其他类型：",
        PolicyInvalid = "配置无法用表单读取，原始内容已保留。请展开兼容配置修正；无效配置不会保存。",
        UnnamedApp = "未指定应用",
        DisabledSuffix = "（已停用）",
        PolicyRawHeader = "策略配置",
        PolicyRawExpander = "兼容配置 · 查看或编辑 JSON",
        PolicyIgnoreSemantics = "未选择类型时忽略全部类型。已保存的清理规则会保留，切回清理后可继续使用。",
        PolicyCleanSemantics = "未选择类型时此策略不生效。清理规则仅转换文字类型；替换为空白或 __IGNORE_CAPTURE__ 会忽略本条采集。",
        PolicyMissingApp = " 请填写应用名称或路径，否则此策略不会匹配。",
        RuleSelection = "清理规则（按执行顺序）",
        RulePatternHeader = "匹配表达式（Rust 正则）",
        RuleReplacementHeader = "替换内容",
        RuleReplacementPlaceholder = "支持 $1、${name} 和 \\n、\\r、\\t",
        RuleLabelHeader = "规则说明（可选）",
        RuleActionReplace = "替换文字",
        RuleAdd = "添加规则",
        RuleRemove = "删除规则",
        RuleHint = "清空结果或替换为 __IGNORE_CAPTURE__ 会忽略采集。正则由核心在测试和保存时校验；实际换行请写成 \\n、\\r，制表符写成 \\t。",
        RuleRawExpander = "兼容配置 · 查看或编辑规则文本",
        RuleRawHeader = "原始规则文本",
        RuleInputWarning = "此文本无法安全写成一行规则。换行请使用 \\n、\\r；匹配 => 请将 > 写作 \\x3E；开头的 # 请写作 \\x23 或 [#]。",
        RuleUnavailable = "原始文本包含无法逐条表示的规则，请在兼容配置中修正。原始草稿已完整保留。"
    };
    internal static AdvancedStrings Tw { get; } = new()
    {
        WindowTitle = "TieZ · 進階設定",
        Heading = "進階設定",
        Intro = "規則在儲存時由核心驗證。關閉此視窗不影響剪貼簿擷取。",
        KindsHeader = "敏感比對類別（逗號分隔）",
        KindsPlaceholder = "phone,idcard,email,secret",
        PrivacyHeader = "自訂敏感正規表示式（每行一條）",
        PrivacyHint = "敏感規則使用正規表示式，一行一條；留空表示不新增自訂規則。",
        RulesExampleHint = "例如：foo => bar。空白行與以 # 開頭的註解行會被忽略。",
        Save = "儲存進階設定",
        Reload = "捨棄草稿並重新載入",
        RevisionFormat = "已儲存設定版本 {0}",
        Conflict = "設定已在其他視窗變更。目前草稿已保留，請重新載入後再修改。",
        Saved = "進階設定已儲存並套用於擷取。",
        SavedWarningFormat = "進階設定已儲存，但執行階段套用未完整完成：{0}",
        SaveConflict = "設定已在其他視窗變更，未儲存。目前草稿已保留，請重新載入後再修改。",
        PreviewHeader = "規則測試預覽",
        PreviewIntro = "僅測試這一組文字清理規則，不包含應用程式比對、策略忽略動作、其他規則組或敏感偵測。測試不會儲存設定或操作剪貼簿。最多 32 條規則、4096 次比對、64 KiB 輸出。",
        SampleHeader = "測試文字（最多 4096 字元）",
        OutputHeader = "處理結果",
        Run = "測試目前規則",
        RunResultFormat = "已執行 {0} 條規則，符合 {1} 處。",
        IgnoredSuffix = "此規則組會忽略這筆擷取。",
        DoneSuffix = "測試完成，設定尚未因此儲存。",
        InvalidRule = "規則含無效正規表示式或缺少 =>，請修正後重試。",
        LimitError = "測試超過樣本、規則、比對或輸出上限，請縮小樣本或規則範圍。",
        FailureFormat = "測試未完成：{0}",
        PolicyHeading = "依應用程式清理策略",
        PolicyIntro = "先依序執行符合的應用程式策略，再執行全域清理；清理可累加，忽略會立即停止擷取。名稱和路徑任一符合即可；支援執行檔名、應用程式識別碼和 /正規表示式/。",
        PolicySelection = "應用程式策略（依執行順序）",
        PolicyAdd = "新增策略",
        PolicyRemove = "刪除此策略",
        MoveUp = "上移",
        MoveDown = "下移",
        PolicyEnabled = "啟用此策略",
        AppNameHeader = "應用程式名稱",
        AppNamePlaceholder = "例如：notepad",
        AppPathHeader = "路徑、執行檔名或應用程式識別碼",
        AppPathPlaceholder = "例如：notepad.exe",
        ActionHeader = "符合後執行",
        ActionClean = "清理文字",
        ActionIgnore = "忽略擷取",
        TypesHeader = "適用內容類型",
        TypeText = "文字",
        TypeCode = "程式碼",
        TypeUrl = "連結",
        TypeRichText = "富文字",
        TypeImage = "圖片",
        TypeFile = "檔案",
        TypeVideo = "影片",
        ListSeparator = "、",
        OtherTypesPrefix = "保留的其他類型：",
        PolicyInvalid = "設定無法以表單讀取，原始內容已保留。請展開相容設定修正；無效設定不會儲存。",
        UnnamedApp = "未指定應用程式",
        DisabledSuffix = "（已停用）",
        PolicyRawHeader = "策略設定",
        PolicyRawExpander = "相容設定 · 檢視或編輯 JSON",
        PolicyIgnoreSemantics = "未選擇類型時忽略全部類型。已儲存的清理規則會保留，切回清理後可繼續使用。",
        PolicyCleanSemantics = "未選擇類型時此策略不生效。清理規則僅轉換文字類型；取代為空白或 __IGNORE_CAPTURE__ 會忽略這筆擷取。",
        PolicyMissingApp = " 請填寫應用程式名稱或路徑，否則此策略不會符合。",
        RuleSelection = "清理規則（依執行順序）",
        RulePatternHeader = "比對表達式（Rust 正規表示式）",
        RuleReplacementHeader = "取代內容",
        RuleReplacementPlaceholder = "支援 $1、${name} 與 \\n、\\r、\\t",
        RuleLabelHeader = "規則說明（選填）",
        RuleActionReplace = "取代文字",
        RuleAdd = "新增規則",
        RuleRemove = "刪除規則",
        RuleHint = "清空結果或取代為 __IGNORE_CAPTURE__ 會忽略擷取。正規表示式由核心在測試和儲存時驗證；實際換行請寫成 \\n、\\r，定位字元寫成 \\t。",
        RuleRawExpander = "相容設定 · 檢視或編輯規則文字",
        RuleRawHeader = "原始規則文字",
        RuleInputWarning = "此文字無法安全寫成一行規則。換行請使用 \\n、\\r；比對 => 請將 > 寫成 \\x3E；開頭的 # 請寫成 \\x23 或 [#]。",
        RuleUnavailable = "原始文字包含無法逐條表示的規則，請在相容設定中修正。原始草稿已完整保留。"
    };
    internal static AdvancedStrings En { get; } = new()
    {
        WindowTitle = "TieZ - advanced settings",
        Heading = "Advanced settings",
        Intro = "The core validates rules when they are saved. Closing this window does not affect clipboard capture.",
        KindsHeader = "Sensitive categories (comma separated)",
        KindsPlaceholder = "phone,idcard,email,secret",
        PrivacyHeader = "Custom sensitive patterns (one per line)",
        PrivacyHint = "Sensitive rules are regular expressions, one per line; leave empty to add no custom rule.",
        RulesExampleHint = "For example: foo => bar. Blank lines and lines starting with # are ignored.",
        Save = "Save advanced settings",
        Reload = "Discard drafts and reload",
        RevisionFormat = "Saved configuration revision {0}",
        Conflict = "The configuration changed in another window. The current draft is kept; reload before editing again.",
        Saved = "Advanced settings saved and applied to capture.",
        SavedWarningFormat = "Advanced settings saved, but the runtime update did not finish: {0}",
        SaveConflict = "The configuration changed in another window and was not saved. The current draft is kept; reload before editing again.",
        PreviewHeader = "Rule test preview",
        PreviewIntro = "Tests only this one text cleanup rule set: no app matching, policy ignore action, other rule set or sensitive detection. The test saves no configuration and never touches the clipboard. Limits: 32 rules, 4096 matches, 64 KiB output.",
        SampleHeader = "Sample text (up to 4096 characters)",
        OutputHeader = "Result",
        Run = "Test current rules",
        RunResultFormat = "Ran {0} rules; {1} matches.",
        IgnoredSuffix = "This rule set ignores the capture.",
        DoneSuffix = "Test finished; the configuration is not saved by it.",
        InvalidRule = "A rule has an invalid regular expression or misses =>; fix it and retry.",
        LimitError = "The test exceeded the sample, rule, match or output limit; use a smaller sample or fewer rules.",
        FailureFormat = "The test did not finish: {0}",
        PolicyHeading = "Cleanup policies by app",
        PolicyIntro = "Matching app policies run first, then the global cleanup; clean rules add up and ignore stops the capture at once. A name or a path match is enough; executable names, app IDs and /regex/ are supported.",
        PolicySelection = "App policies (in execution order)",
        PolicyAdd = "Add policy",
        PolicyRemove = "Delete this policy",
        MoveUp = "Move up",
        MoveDown = "Move down",
        PolicyEnabled = "Enable this policy",
        AppNameHeader = "App name",
        AppNamePlaceholder = "e.g. notepad",
        AppPathHeader = "Path, executable name or app ID",
        AppPathPlaceholder = "e.g. notepad.exe",
        ActionHeader = "When matched",
        ActionClean = "Clean text",
        ActionIgnore = "Ignore capture",
        TypesHeader = "Content types",
        TypeText = "Text",
        TypeCode = "Code",
        TypeUrl = "Link",
        TypeRichText = "Rich text",
        TypeImage = "Image",
        TypeFile = "File",
        TypeVideo = "Video",
        ListSeparator = ", ",
        OtherTypesPrefix = "Other kept types: ",
        PolicyInvalid = "The configuration cannot be read as a form; the raw text is kept. Expand the raw configuration to fix it; invalid configuration is not saved.",
        UnnamedApp = "Unnamed app",
        DisabledSuffix = " (disabled)",
        PolicyRawHeader = "Policy configuration",
        PolicyRawExpander = "Raw configuration - view or edit JSON",
        PolicyIgnoreSemantics = "With no type selected every type is ignored. Saved cleanup rules are kept and can be used again after switching back to clean.",
        PolicyCleanSemantics = "With no type selected this policy has no effect. Cleanup rules convert text types only; replacing with blank or __IGNORE_CAPTURE__ ignores this capture.",
        PolicyMissingApp = " Fill in an app name or path, otherwise this policy never matches.",
        RuleSelection = "Cleanup rules (in execution order)",
        RulePatternHeader = "Pattern (Rust regex)",
        RuleReplacementHeader = "Replacement",
        RuleReplacementPlaceholder = "Supports $1, ${name}, \\n, \\r and \\t",
        RuleLabelHeader = "Rule description (optional)",
        RuleActionReplace = "Replace text",
        RuleAdd = "Add rule",
        RuleRemove = "Delete rule",
        RuleHint = "Clearing the result or replacing it with __IGNORE_CAPTURE__ ignores the capture. The core validates the regex on test and save; write real newlines as \\n and \\r, and a tab as \\t.",
        RuleRawExpander = "Raw configuration - view or edit rule text",
        RuleRawHeader = "Raw rule text",
        RuleInputWarning = "This text cannot be written as a one line rule. Use \\n and \\r for newlines; write > in a => match as \\x3E; write a leading # as \\x23 or [#].",
        RuleUnavailable = "The raw text has rules that cannot be shown per row; fix them in the raw configuration. The raw draft is fully kept."
    };

    internal static AdvancedStrings Current => LanguageService.Current switch
    {
        Language.TraditionalChinese => Tw,
        Language.English => En,
        _ => Zh
    };
}