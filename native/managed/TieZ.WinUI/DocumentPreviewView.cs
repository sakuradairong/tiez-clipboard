using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using TieZ.Core;
using Windows.UI.Text;

namespace TieZ.Native;

// Render a bounded semantic model with native controls; HTML never enters XAML or a browser.
public sealed class DocumentPreviewView : Grid
{
    private readonly StackPanel content = new() { Spacing = 10 };
    internal long? DisplayedId { get; private set; }
    internal bool HasDocument => DisplayedId is not null && Visibility == Visibility.Visible;
    internal int CellCount { get; private set; }
    internal int MergedCellCount { get; private set; }
    internal int EmphasizedRuns { get; private set; }
    internal int SafeLinkCount { get; private set; }
    private int runCount;
    private readonly System.Text.StringBuilder renderedText = new();
    internal string RenderedText => renderedText.ToString();
    internal bool OmittedMedia { get; private set; }
    public DocumentPreviewView()
    {
        Children.Add(new ScrollViewer { Content = content,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto });
        AutomationProperties.SetName(this, "所选富文本的原生结构化预览");
        Visibility = Visibility.Collapsed;
    }
    internal void Clear()
    {
        content.Children.Clear(); DisplayedId = null; renderedText.Clear();
        CellCount = MergedCellCount = EmphasizedRuns = SafeLinkCount = runCount = 0;
        OmittedMedia = false; Visibility = Visibility.Collapsed;
    }
    internal void Show(DocumentPreviewData document, string originalText)
    {
        Clear();
        try
        {
            if (document.Blocks.Count > 128) throw new InvalidDataException("Too many preview blocks.");
            foreach (var block in document.Blocks)
            {
                if (block.Kind == "paragraph")
                {
                    var text = Text(block.Runs ?? [], block.Heading);
                    if (block.Preformatted)
                    {
                        text.TextWrapping = TextWrapping.NoWrap;
                        content.Children.Add(new ScrollViewer { Content = text,
                            HorizontalScrollBarVisibility = ScrollBarVisibility.Auto,
                            VerticalScrollBarVisibility = ScrollBarVisibility.Disabled });
                    }
                    else content.Children.Add(text);
                }
                else if (block.Kind == "table")
                {
                    if (block.Rows is < 1 or > 64 || block.Columns is < 1 or > 32 || block.Cells is null
                        || CellCount + block.Cells.Count > 512)
                        throw new InvalidDataException("Invalid native table dimensions.");
                    var table = new Grid();
                    for (int row = 0; row < block.Rows; row++) table.RowDefinitions.Add(new() { Height = GridLength.Auto });
                    for (int column = 0; column < block.Columns; column++)
                        table.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                    var occupied = new bool[block.Rows, block.Columns];
                    foreach (var cell in block.Cells)
                    {
                        if (cell.Row < 0 || cell.Column < 0 || cell.RowSpan < 1 || cell.ColumnSpan < 1
                            || cell.Row > block.Rows - cell.RowSpan || cell.Column > block.Columns - cell.ColumnSpan)
                            throw new InvalidDataException("Invalid native table span.");
                        for (int row = cell.Row; row < cell.Row + cell.RowSpan; row++)
                            for (int column = cell.Column; column < cell.Column + cell.ColumnSpan; column++)
                            {
                                if (occupied[row, column]) throw new InvalidDataException("Overlapping table cells.");
                                occupied[row, column] = true;
                            }
                        var text = Text(cell.Runs, 0); text.MaxWidth = 400;
                        var border = new Border { Child = text, Padding = new Thickness(8), MinWidth = 60,
                            BorderThickness = new Thickness(0.5),
                            Style = (Style)Application.Current.Resources[cell.Header ? "NativeDocumentHeaderStyle" : "NativeDocumentCellStyle"] };
                        Grid.SetRow(border, cell.Row); Grid.SetColumn(border, cell.Column);
                        Grid.SetRowSpan(border, cell.RowSpan); Grid.SetColumnSpan(border, cell.ColumnSpan);
                        table.Children.Add(border); CellCount++;
                        if (cell.RowSpan > 1 || cell.ColumnSpan > 1) MergedCellCount++;
                    }
                    content.Children.Add(new ScrollViewer { Content = table,
                        HorizontalScrollBarVisibility = ScrollBarVisibility.Auto,
                        VerticalScrollBarVisibility = ScrollBarVisibility.Disabled });
                }
                else throw new InvalidDataException("Unknown document block.");
            }
            string note = "结构化预览 · 字体、颜色和复杂布局可能简化；原始格式保留用于复制和粘贴。";
            if (document.OmittedMedia) note += "\n嵌入图片、媒体和对象以替代提示显示，不加载外部资源。";
            if (document.Truncated) note += "\n内容超过预览限制，仅显示部分；原始内容未修改。";
            if (document.Blocks.Count == 0) note += "\n没有可显示的结构，请展开原始文本。";
            content.Children.Add(new TextBlock { Text = note, TextWrapping = TextWrapping.Wrap, Opacity = 0.7 });
            var original = new TextBox { Text = originalText, IsReadOnly = true, AcceptsReturn = true,
                TextWrapping = TextWrapping.Wrap, MaxHeight = 240 };
            AutomationProperties.SetName(original, "富文本记录的原始纯文本");
            content.Children.Add(new Expander { Header = "原始文本", Content = original,
                HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch });
            DisplayedId = document.Id; OmittedMedia = document.OmittedMedia;
            Visibility = Visibility.Visible;
        }
        catch { Clear(); throw; }
    }
    private RichTextBlock Text(IReadOnlyList<DocumentTextRun> runs, int heading)
    {
        var block = new RichTextBlock { IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap,
            FontSize = heading is >= 1 and <= 6 ? 26 - heading * 2 : 14 };
        var paragraph = new Paragraph();
        foreach (var span in runs)
        {
            if (++runCount > 2048) throw new InvalidDataException("Too many preview runs.");
            if (renderedText.Length + span.Text.Length > 131072) throw new InvalidDataException("Too much preview text.");
            var run = new Run { Text = span.Text,
                FontWeight = span.Bold ? Microsoft.UI.Text.FontWeights.Bold : Microsoft.UI.Text.FontWeights.Normal,
                FontStyle = span.Italic ? FontStyle.Italic : FontStyle.Normal };
            if (span.Code) run.FontFamily = new FontFamily("Consolas");
            if (span.Underline) run.TextDecorations |= TextDecorations.Underline;
            if (span.Strike) run.TextDecorations |= TextDecorations.Strikethrough;
            if (span.Bold || span.Italic || span.Underline || span.Strike) EmphasizedRuns++;
            if (span.Link is not null && Uri.TryCreate(span.Link, UriKind.Absolute, out var uri)
                && uri.Scheme is "http" or "https" && string.IsNullOrEmpty(uri.UserInfo))
            {
                var link = new Hyperlink { NavigateUri = uri }; link.Inlines.Add(run);
                paragraph.Inlines.Add(link); SafeLinkCount++;
            }
            else paragraph.Inlines.Add(run);
            renderedText.Append(span.Text);
        }
        block.Blocks.Add(paragraph);
        return block;
    }
}
