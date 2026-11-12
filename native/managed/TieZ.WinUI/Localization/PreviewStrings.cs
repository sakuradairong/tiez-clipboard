namespace TieZ.Native;

/// <summary>
/// Chrome of the native previews: image caption and thumbnail placeholders, the structured rich text renderer and
/// the compact preview window. Previewed content itself is never localized.
/// </summary>
internal sealed class PreviewStrings
{
    internal required string WindowTitle { get; init; }
    internal required string Separator { get; init; }
    internal required string MetadataSessionOnly { get; init; }
    internal required string MetadataSaved { get; init; }

    // Image preview
    internal required string ImageName { get; init; }
    internal required string ImageCaptionFormat { get; init; }
    internal required string ImageAnimationNote { get; init; }
    internal required string ThumbnailName { get; init; }
    internal required string ThumbnailLoading { get; init; }
    internal required string ThumbnailFailed { get; init; }
    internal required string ThumbnailHint { get; init; }
    internal required string ThumbnailStaticLabel { get; init; }
    internal required string ThumbnailAnimationLabel { get; init; }

    // Rich text preview
    internal required string DocumentName { get; init; }
    internal required string DocumentNote { get; init; }
    internal required string DocumentOmittedMedia { get; init; }
    internal required string DocumentTruncated { get; init; }
    internal required string DocumentEmpty { get; init; }
    internal required string DocumentOriginalName { get; init; }
    internal required string OriginalTextHeader { get; init; }

    internal required string UnavailableFormat { get; init; }

    internal static PreviewStrings Zh { get; } = new()
    {
        WindowTitle = "TieZ · 紧凑预览",
        Separator = " · ",
        MetadataSessionOnly = "仅本次会话",
        MetadataSaved = "已保存",
        ImageName = "所选图片的静态预览",
        ImageCaptionFormat = "静态预览 {0} × {1} · 原图 {2} × {3}",
        ImageAnimationNote = "动画显示首帧，原始动画保留用于复制和粘贴。",
        ThumbnailName = "图片缩略图",
        ThumbnailLoading = "正在加载图片预览",
        ThumbnailFailed = "图片预览不可用 · 选择后查看详情",
        ThumbnailHint = "图片 · 选择后查看详情",
        ThumbnailStaticLabel = "图片缩略图 · 原始图片保留",
        ThumbnailAnimationLabel = "图片缩略图 · 动画显示首帧，原始动画保留",
        DocumentName = "所选富文本的原生结构化预览",
        DocumentNote = "结构化预览 · 字体、颜色和复杂布局可能简化；原始格式保留用于复制和粘贴。",
        DocumentOmittedMedia = "嵌入图片、媒体和对象以替代提示显示，不加载外部资源。",
        DocumentTruncated = "内容超过预览限制，仅显示部分；原始内容未修改。",
        DocumentEmpty = "没有可显示的结构，请展开原始文本。",
        DocumentOriginalName = "富文本记录的原始纯文本",
        OriginalTextHeader = "原始文本",
        UnavailableFormat = "紧凑预览不可用：{0}"
    };

    internal static PreviewStrings Tw { get; } = new()
    {
        WindowTitle = "TieZ · 精簡預覽",
        Separator = " · ",
        MetadataSessionOnly = "僅本次工作階段",
        MetadataSaved = "已儲存",
        ImageName = "所選圖片的靜態預覽",
        ImageCaptionFormat = "靜態預覽 {0} × {1} · 原圖 {2} × {3}",
        ImageAnimationNote = "動畫僅顯示第一個影格，原始動畫保留供複製和貼上。",
        ThumbnailName = "圖片縮圖",
        ThumbnailLoading = "正在載入圖片預覽",
        ThumbnailFailed = "圖片預覽無法使用 · 選擇後檢視詳細資料",
        ThumbnailHint = "圖片 · 選擇後檢視詳細資料",
        ThumbnailStaticLabel = "圖片縮圖 · 原始圖片保留",
        ThumbnailAnimationLabel = "圖片縮圖 · 顯示第一個影格，原始動畫保留",
        DocumentName = "所選富文字的原生結構化預覽",
        DocumentNote = "結構化預覽 · 字型、顏色和複雜版面可能簡化；原始格式保留供複製和貼上。",
        DocumentOmittedMedia = "內嵌圖片、媒體和物件以替代提示顯示，不載入外部資源。",
        DocumentTruncated = "內容超過預覽限制，僅顯示部分；原始內容未修改。",
        DocumentEmpty = "沒有可顯示的結構，請展開原始文字。",
        DocumentOriginalName = "富文字記錄的原始純文字",
        OriginalTextHeader = "原始文字",
        UnavailableFormat = "精簡預覽無法使用：{0}"
    };

    internal static PreviewStrings En { get; } = new()
    {
        WindowTitle = "TieZ - Compact preview",
        Separator = " - ",
        MetadataSessionOnly = "This session only",
        MetadataSaved = "Saved",
        ImageName = "Static preview of the selected image",
        ImageCaptionFormat = "Static preview {0} x {1} - original {2} x {3}",
        ImageAnimationNote = "The first frame of the animation is shown; the original animation is kept for copy and paste.",
        ThumbnailName = "Image thumbnail",
        ThumbnailLoading = "Loading the image preview",
        ThumbnailFailed = "Image preview unavailable - select to see details",
        ThumbnailHint = "Image - select to see details",
        ThumbnailStaticLabel = "Image thumbnail - original image kept",
        ThumbnailAnimationLabel = "Image thumbnail - first frame shown, original animation kept",
        DocumentName = "Native structured preview of the selected rich text",
        DocumentNote = "Structured preview - fonts, colors and complex layout may be simplified; the original format is kept for copy and paste.",
        DocumentOmittedMedia = "Embedded images, media and objects appear as placeholders; no external resources are loaded.",
        DocumentTruncated = "The content exceeds the preview limit and is only partly shown; the original content is unchanged.",
        DocumentEmpty = "There is no structure to show; expand the original text.",
        DocumentOriginalName = "Original plain text of the rich text record",
        OriginalTextHeader = "Original text",
        UnavailableFormat = "Compact preview unavailable: {0}"
    };

    internal static PreviewStrings Current => LanguageService.Current switch
    {
        Language.TraditionalChinese => Tw,
        Language.English => En,
        _ => Zh
    };
}