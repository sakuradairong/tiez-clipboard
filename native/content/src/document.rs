//! Browser-free semantic HTML preview; clipboard payloads are never rewritten.
use scraper::{ElementRef, Html, Node, Selector};
use serde::Serialize;

const TEXT_LIMIT: usize = 65_536;
const NODE_LIMIT: usize = 10_000;
const BLOCK_LIMIT: usize = 128;
const CELL_LIMIT: usize = 512;
const RUN_LIMIT: usize = 2048;
const LINK_BYTES_LIMIT: usize = 128 * 1024;

#[derive(Clone, Default, PartialEq, Serialize)]
pub struct TextRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Block {
    Paragraph {
        runs: Vec<TextRun>,
        heading: u8,
        preformatted: bool,
    },
    Table {
        rows: usize,
        columns: usize,
        cells: Vec<Cell>,
    },
}
#[derive(Serialize)]
pub struct Cell {
    pub row: usize,
    pub column: usize,
    pub row_span: usize,
    pub column_span: usize,
    pub header: bool,
    pub runs: Vec<TextRun>,
}
#[derive(Serialize)]
pub struct DocumentPreview {
    pub blocks: Vec<Block>,
    pub truncated: bool,
    pub omitted_media: bool,
}
impl std::fmt::Debug for DocumentPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentPreview")
            .field("blocks", &self.blocks.len())
            .field("truncated", &self.truncated)
            .finish_non_exhaustive()
    }
}

struct Writer {
    document: DocumentPreview,
    runs: Vec<TextRun>,
    text: usize,
    nodes: usize,
    cells: usize,
    run_count: usize,
    link_bytes: usize,
    last_space: bool,
    heading: u8,
    pre: bool,
}
impl Writer {
    fn new() -> Self {
        Self {
            document: DocumentPreview {
                blocks: vec![],
                truncated: false,
                omitted_media: false,
            },
            runs: vec![],
            text: 0,
            nodes: 0,
            cells: 0,
            run_count: 0,
            link_bytes: 0,
            last_space: true,
            heading: 0,
            pre: false,
        }
    }
    fn text(&mut self, text: &str, style: &TextRun) {
        let mut out = String::new();
        for c in text.chars() {
            let space = c.is_ascii_whitespace();
            if !self.pre && space && self.last_space {
                continue;
            }
            if self.text >= TEXT_LIMIT {
                self.document.truncated = true;
                break;
            }
            out.push(if !self.pre && space { ' ' } else { c });
            self.last_space = space;
            self.text += 1;
        }
        if out.is_empty() {
            return;
        }
        let mut run = style.clone();
        if let Some(last) = self.runs.last_mut() {
            if run.bold == last.bold
                && run.italic == last.italic
                && run.underline == last.underline
                && run.strike == last.strike
                && run.code == last.code
                && run.link == last.link
            {
                last.text.push_str(&out);
                return;
            }
        }
        let link_bytes = run.link.as_ref().map_or(0, String::len);
        if self.run_count >= RUN_LIMIT || self.link_bytes + link_bytes > LINK_BYTES_LIMIT {
            self.document.truncated = true;
            return;
        }
        self.run_count += 1;
        self.link_bytes += link_bytes;
        run.text = out;
        self.runs.push(run);
    }
    fn flush(&mut self) {
        if !self.runs.is_empty() {
            if self.document.blocks.len() < BLOCK_LIMIT {
                self.document.blocks.push(Block::Paragraph {
                    runs: std::mem::take(&mut self.runs),
                    heading: self.heading,
                    preformatted: self.pre,
                });
            } else {
                self.document.truncated = true;
                self.runs.clear();
            }
        }
        self.last_space = true;
    }
    fn visit(&mut self, element: ElementRef<'_>, style: &TextRun, depth: usize, inline: bool) {
        self.nodes += 1;
        if self.nodes > NODE_LIMIT
            || depth > 32
            || self.text >= TEXT_LIMIT
            || self.run_count >= RUN_LIMIT
            || self.link_bytes >= LINK_BYTES_LIMIT
            || self.document.blocks.len() >= BLOCK_LIMIT
        {
            self.document.truncated = true;
            return;
        }
        let name = element.value().name();
        if matches!(name, "script" | "style" | "head" | "template" | "noscript") {
            return;
        }
        if matches!(
            name,
            "img" | "svg" | "canvas" | "iframe" | "object" | "embed" | "video" | "audio" | "input"
        ) {
            self.document.omitted_media = true;
            self.text("[嵌入对象未显示]", style);
            if let Some(alt) = element.attr("alt") {
                self.text(alt, style);
            }
            return;
        }
        if name == "br" {
            let pre = self.pre;
            self.pre = true;
            self.text("\n", style);
            self.pre = pre;
            return;
        }
        let block = matches!(
            name,
            "p" | "div"
                | "section"
                | "article"
                | "blockquote"
                | "li"
                | "pre"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "hr"
        );
        if name == "table" && !inline {
            self.flush();
            self.table(element, style, depth);
            return;
        }
        if block && !inline {
            self.flush();
        } else if block && inline && !self.runs.is_empty() {
            let pre = self.pre;
            self.pre = true;
            self.text("\n", style);
            self.pre = pre;
        }
        let old_heading = self.heading;
        let old_pre = self.pre;
        if matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
            self.heading = name.as_bytes()[1] - b'0';
        }
        self.pre |= name == "pre";
        let style = styled(element, style);
        if name == "li" {
            let ordered = element
                .parent()
                .and_then(ElementRef::wrap)
                .filter(|e| e.value().name() == "ol");
            let prefix = if let Some(parent) = ordered {
                let start = parent
                    .attr("start")
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(1);
                let preceding = element
                    .prev_siblings()
                    .filter_map(ElementRef::wrap)
                    .filter(|e| e.value().name() == "li")
                    .count();
                format!("{}. ", start.saturating_add(preceding as i64))
            } else {
                "• ".into()
            };
            self.text(&prefix, &style);
        }
        for child in element.children() {
            if self.nodes > NODE_LIMIT || self.text >= TEXT_LIMIT {
                self.document.truncated = true;
                break;
            }
            match child.value() {
                Node::Text(text) => {
                    self.nodes += 1;
                    self.text(text, &style);
                }
                Node::Element(_) => {
                    if let Some(child) = ElementRef::wrap(child) {
                        self.visit(child, &style, depth + 1, inline);
                    }
                }
                _ => {}
            }
        }
        if block && !inline {
            self.flush();
        }
        self.heading = old_heading;
        self.pre = old_pre;
    }
    fn table(&mut self, table: ElementRef<'_>, style: &TextRun, depth: usize) {
        for caption in table
            .children()
            .filter_map(ElementRef::wrap)
            .filter(|e| e.value().name() == "caption")
        {
            self.visit(caption, style, depth + 1, false);
            self.flush();
        }
        if self.document.blocks.len() >= BLOCK_LIMIT {
            self.document.truncated = true;
            return;
        }
        let selector = Selector::parse("tr").expect("constant selector");
        let mut occupied = [[false; 32]; 64];
        let mut cells = Vec::new();
        let (mut rows, mut columns) = (0, 0);
        for (row_index, row) in table
            .select(&selector)
            .filter(|row| {
                row.ancestors()
                    .filter_map(ElementRef::wrap)
                    .find(|e| e.value().name() == "table")
                    .map(|parent| parent.id())
                    == Some(table.id())
            })
            .enumerate()
        {
            if row_index >= 64 {
                self.document.truncated = true;
                break;
            }
            let mut column = 0;
            for cell in row
                .children()
                .filter_map(ElementRef::wrap)
                .filter(|e| matches!(e.value().name(), "td" | "th"))
            {
                while column < 32 && occupied[row_index][column] {
                    column += 1;
                }
                if column >= 32
                    || self.cells >= CELL_LIMIT
                    || self.nodes > NODE_LIMIT
                    || self.text >= TEXT_LIMIT
                {
                    self.document.truncated = true;
                    break;
                }
                let row_span = span(cell.attr("rowspan"), 64 - row_index);
                let mut column_span = span(cell.attr("colspan"), 32 - column);
                // Malformed overlapping spans are clipped, never allowed to overlap controls.
                while (column..column + column_span).any(|c| occupied[row_index][c]) {
                    column_span -= 1;
                }
                for row in occupied.iter_mut().skip(row_index).take(row_span) {
                    for item in row.iter_mut().skip(column).take(column_span) {
                        *item = true;
                    }
                }
                let saved = std::mem::take(&mut self.runs);
                self.last_space = true;
                self.visit(cell, style, depth + 1, true);
                cells.push(Cell {
                    row: row_index,
                    column,
                    row_span,
                    column_span,
                    header: cell.value().name() == "th",
                    runs: std::mem::replace(&mut self.runs, saved),
                });
                self.cells += 1;
                rows = rows.max(row_index + row_span);
                columns = columns.max(column + column_span);
                column += column_span;
            }
        }
        if !cells.is_empty() {
            self.document.blocks.push(Block::Table {
                rows,
                columns,
                cells,
            });
        }
        self.last_space = true;
    }
}
fn span(value: Option<&str>, max: usize) -> usize {
    value
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, max)
}
fn styled(element: ElementRef<'_>, parent: &TextRun) -> TextRun {
    let mut style = parent.clone();
    style.text.clear();
    match element.value().name() {
        "b" | "strong" | "th" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => style.bold = true,
        "i" | "em" => style.italic = true,
        "u" => style.underline = true,
        "s" | "del" | "strike" => style.strike = true,
        "code" | "pre" | "kbd" | "samp" => style.code = true,
        "a" => {
            style.link = element
                .attr("href")
                .filter(|value| value.len() <= 2048)
                .and_then(|value| reqwest::Url::parse(value).ok())
                .filter(|url| {
                    matches!(url.scheme(), "http" | "https")
                        && url.as_str().len() <= 2048
                        && url.username().is_empty()
                        && url.password().is_none()
                })
                .map(|url| url.to_string())
        }
        _ => {}
    }
    // Only semantic emphasis is interpreted; arbitrary CSS never reaches XAML.
    if let Some(css) = element.attr("style") {
        for declaration in css.split(';') {
            let Some((key, value)) = declaration.split_once(':') else {
                continue;
            };
            let value = value.trim().to_ascii_lowercase();
            match key.trim().to_ascii_lowercase().as_str() {
                "font-weight"
                    if value == "bold" || value.parse::<u32>().is_ok_and(|n| n >= 600) =>
                {
                    style.bold = true
                }
                "font-style" if value == "italic" || value == "oblique" => style.italic = true,
                "text-decoration" | "text-decoration-line" => {
                    style.underline |= value.contains("underline");
                    style.strike |= value.contains("line-through");
                }
                _ => {}
            }
        }
    }
    style
}

pub fn parse(html: &str) -> Result<DocumentPreview, &'static str> {
    if html.len() > 4 * 1024 * 1024 {
        return Err("document_too_large");
    }
    let (html, _) = crate::utils::split_rich_html_and_named_formats(html);
    let (html, _) = crate::utils::split_rich_html_and_image_fallback(&html);
    if html.len() > 1024 * 1024 {
        return Err("document_too_large");
    }
    let parsed = Html::parse_fragment(&html);
    let mut writer = Writer::new();
    writer.visit(parsed.root_element(), &TextRun::default(), 0, false);
    writer.flush();
    Ok(writer.document)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn runs(document: &DocumentPreview) -> Vec<&TextRun> {
        document
            .blocks
            .iter()
            .flat_map(|block| match block {
                Block::Paragraph { runs, .. } => runs.iter().collect::<Vec<_>>(),
                Block::Table { cells, .. } => cells.iter().flat_map(|cell| &cell.runs).collect(),
            })
            .collect()
    }
    #[test]
    fn semantic_html_entities_styles_and_pre_are_preserved_without_changing_source() {
        let html = "<h2>中文 &amp; heading</h2><p>A <b>bold <i>both</i></b><span style='text-decoration:underline line-through'> decorated</span><br>line</p><pre>  x\n y </pre>";
        let preview = parse(html).unwrap();
        assert!(matches!(
            preview.blocks[0],
            Block::Paragraph { heading: 2, .. }
        ));
        let all = runs(&preview);
        assert!(all
            .iter()
            .any(|r| r.text.contains("both") && r.bold && r.italic));
        assert!(all.iter().any(|r| r.underline && r.strike));
        assert!(all.iter().any(|r| r.text == "  x\n y " && r.code));
        assert!(all.iter().any(|r| r.text.contains("中文 & heading")));
        assert!(!preview.truncated);
    }
    #[test]
    fn scripts_resources_and_unsafe_urls_never_become_active_content() {
        let preview = parse("<style>secret-css</style><script>secret-js</script><img src='https://invalid/image' alt='logo'><iframe src='file:///secret'></iframe><a href='javascript:alert(1)'>bad</a><a href='file:///secret'>file</a><a href='https://example.com/a'>safe</a>").unwrap();
        let all = runs(&preview);
        assert!(preview.omitted_media);
        assert!(!all
            .iter()
            .any(|r| r.text.contains("secret-js") || r.text.contains("secret-css")));
        assert_eq!(all.iter().filter(|r| r.link.is_some()).count(), 1);
        assert!(all
            .iter()
            .any(|r| r.link.as_deref() == Some("https://example.com/a")));
    }
    #[test]
    fn malformed_html_and_merged_table_cells_are_bounded_and_nonoverlapping() {
        let preview =
            parse("<table><tr><th rowspan=2>H<th colspan=2>top<tr><td>one<td>two</table><p>after")
                .unwrap();
        let Block::Table {
            rows,
            columns,
            cells,
        } = &preview.blocks[0]
        else {
            panic!("table");
        };
        assert_eq!((*rows, *columns, cells.len()), (2, 3, 4));
        assert_eq!(
            (cells[0].row_span, cells[1].column_span, cells[2].column),
            (2, 2, 1)
        );
        assert!(cells[0].header);
        assert!(runs(&preview).iter().any(|r| r.text == "after"));
    }
    #[test]
    fn hostile_size_depth_and_table_dimensions_are_limited_explicitly() {
        assert_eq!(
            parse(&"x".repeat(4 * 1024 * 1024 + 1)).unwrap_err(),
            "document_too_large"
        );
        let preview = parse(&format!("<p>{}</p>", "中".repeat(TEXT_LIMIT + 1))).unwrap();
        assert!(preview.truncated);
        assert_eq!(
            runs(&preview)
                .iter()
                .map(|r| r.text.chars().count())
                .sum::<usize>(),
            TEXT_LIMIT
        );
        assert!(
            parse(&format!("{}x{}", "<div>".repeat(100), "</div>".repeat(100)))
                .unwrap()
                .truncated
        );
        let preview = parse(&format!(
            "<table>{}</table>",
            "<tr><td rowspan=99999 colspan=99999>x</td></tr>".repeat(70)
        ))
        .unwrap();
        assert!(preview.truncated);
        let Block::Table { rows, columns, .. } = preview.blocks[0] else {
            panic!("table");
        };
        assert_eq!((rows, columns), (64, 32));
    }
    #[test]
    fn captions_lists_and_nested_tables_keep_readable_text() {
        let preview = parse("<ol start=3><li>first</li><li>second</li></ol><ul><li>bullet</li></ul><table><caption>caption</caption><tr><td>outer<table><tr><td>inner</td></tr></table></td></tr></table>").unwrap();
        let text = runs(&preview)
            .iter()
            .map(|r| r.text.as_str())
            .collect::<String>();
        assert!(
            text.contains("3. first") && text.contains("4. second") && text.contains("• bullet")
        );
        assert!(text.contains("caption") && text.contains("outer") && text.contains("inner"));
        assert_eq!(
            preview
                .blocks
                .iter()
                .filter(|b| matches!(b, Block::Table { .. }))
                .count(),
            1
        );
    }
    #[test]
    fn named_office_formats_remain_in_source_and_do_not_leak_into_preview() {
        let named = crate::infrastructure::windows_api::win_clipboard::NamedClipboardFormat {
            name: "Rich Text Format".into(),
            data: b"{\\rtf1 original}".to_vec(),
        };
        let html = crate::utils::attach_rich_named_formats("<b>visible</b>", &[named]);
        let original = html.clone();
        let preview = parse(&html).unwrap();
        assert_eq!(
            runs(&preview)
                .iter()
                .map(|r| r.text.as_str())
                .collect::<String>(),
            "visible"
        );
        assert_eq!(html, original);
        assert_eq!(
            crate::utils::split_rich_html_and_named_formats(&html).1[0].data,
            b"{\\rtf1 original}"
        );
    }
    #[test]
    fn node_block_and_cell_budgets_are_explicit() {
        assert!(
            parse(&"<span></span>".repeat(NODE_LIMIT + 1))
                .unwrap()
                .truncated
        );
        let preview = parse(&"<p>x</p>".repeat(BLOCK_LIMIT + 1)).unwrap();
        assert!(preview.truncated && preview.blocks.len() <= BLOCK_LIMIT);
        let row = format!("<tr>{}</tr>", "<td>x</td>".repeat(32));
        let preview = parse(&format!("<table>{}</table>", row.repeat(20))).unwrap();
        let Block::Table { cells, .. } = &preview.blocks[0] else {
            panic!("table");
        };
        assert!(preview.truncated && cells.len() == CELL_LIMIT);
        let preview = parse(&format!(
            "<a href='https://example.invalid/{}'>{}</a>",
            "x".repeat(1500),
            "<b>x</b><i>y</i>".repeat(1500)
        ))
        .unwrap();
        assert!(preview.truncated);
        let all = runs(&preview);
        assert!(all.len() <= RUN_LIMIT);
        assert!(
            all.iter()
                .filter_map(|r| r.link.as_ref())
                .map(String::len)
                .sum::<usize>()
                <= LINK_BYTES_LIMIT
        );
    }
}
