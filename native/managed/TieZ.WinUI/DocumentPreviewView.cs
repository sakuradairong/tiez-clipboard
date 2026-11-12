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
    private TextBlock? noteBlock;
    private Expander? originalExpander;
    private TextBox? originalBox;
    private bool truncatedDocument, emptyDocument;
    public DocumentPreviewView()
    {
        Children.Add(new ScrollViewer { Content = content,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto });
        RenderChrome();
        Visibility = Visibility.Collapsed;
    }
    internal void Clear()
    {
        content.Children.Clear(); DisplayedId = null; renderedText.Clear();
        CellCount = MergedCellCount = EmphasizedRuns = SafeLinkCount = runCount = 0;
        OmittedMedia = false; Visibility = Visibility.Collapsed;
        noteBlock = null; originalExpander = null; originalBox = null; truncatedDocument = emptyDocument = false;
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
            noteBlock = new TextBlock { TextWrapping = TextWrapping.Wrap, Opacity = 0.7 };
            content.Children.Add(noteBlock);
            originalBox = new TextBox { Text = originalText, IsReadOnly = true, AcceptsReturn = true,
                TextWrapping = TextWrapping.Wrap, MaxHeight = 240 };
            originalExpander = new Expander { Content = originalBox,
                HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
            content.Children.Add(originalExpander);
            truncatedDocument = document.Truncated; emptyDocument = document.Blocks.Count == 0;
            DisplayedId = document.Id; OmittedMedia = document.OmittedMedia;
            Visibility = Visibility.Visible;
            RenderChrome();
        }
        catch { Clear(); throw; }
    }
    internal void ApplyLanguage() => RenderChrome();
    // A rendered document is never rebuilt by a language switch; only the chrome around it is re-derived.
    private void RenderChrome()
    {
        var text = PreviewStrings.Current;
        AutomationProperties.SetName(this, text.DocumentName);
        if (originalBox is not null) AutomationProperties.SetName(originalBox, text.DocumentOriginalName);
        if (originalExpander is not null) originalExpander.Header = text.OriginalTextHeader;
        if (noteBlock is null) return;
        string note = text.DocumentNote;
        if (OmittedMedia) note += "\n" + text.DocumentOmittedMedia;
        if (truncatedDocument) note += "\n" + text.DocumentTruncated;
        if (emptyDocument) note += "\n" + text.DocumentEmpty;
        noteBlock.Text = note;
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
