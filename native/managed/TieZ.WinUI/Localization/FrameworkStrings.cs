namespace TieZ.Native;

/// <summary>
/// Text Windows ships inside its own control templates: the navigation view pane toggle, the command bar
/// overflow affordance and the info bar severity icon and close button. Those parts are rendered by this app,
/// so their labels and automation names follow the accepted interface language instead of the Windows display
/// language; leaving them to the framework would mix two languages in one window.
/// </summary>
internal sealed class FrameworkStrings
{
    internal required string TogglePane { get; init; }
    internal required string ClosePane { get; init; }
    internal required string OverflowMore { get; init; }
    internal required string ClearText { get; init; }
    internal required string IconError { get; init; }
    internal required string IconWarning { get; init; }
    internal required string IconInformation { get; init; }
    internal required string IconSuccess { get; init; }

    internal static FrameworkStrings Zh { get; } = new()
    {
        TogglePane = "打开导航",
        ClosePane = "关闭导航",
        OverflowMore = "更多",
        ClearText = "清除文本",
        IconError = "错误图标",
        IconWarning = "警告图标",
        IconInformation = "信息图标",
        IconSuccess = "成功图标",
    };

    internal static FrameworkStrings Tw { get; } = new()
    {
        TogglePane = "開啟導覽",
        ClosePane = "關閉導覽",
        OverflowMore = "更多",
        ClearText = "清除文本",
        IconError = "錯誤圖示",
        IconWarning = "警告圖示",
        IconInformation = "資訊圖示",
        IconSuccess = "成功圖示",
    };

    internal static FrameworkStrings En { get; } = new()
    {
        TogglePane = "Open navigation",
        ClosePane = "Close navigation",
        OverflowMore = "More",
        ClearText = "Clear text",
        IconError = "Error icon",
        IconWarning = "Warning icon",
        IconInformation = "Information icon",
        IconSuccess = "Success icon",
    };

    internal static FrameworkStrings Current => LanguageService.Current switch
    {
        Language.TraditionalChinese => Tw,
        Language.English => En,
        _ => Zh,
    };
}