//! Live clipboard ingest policy shared by native TieZ frontends.
//!
//! Platform adapters own OS notifications and format reads. This module owns
//! format priority, CRLF normalization (without trimming), consecutive-copy
//! dedup, and self-paste echo suppression.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::path::Path;

const ECHO_RING: usize = 8;
const PREVIEW_CHARS: usize = 240;

/// Normalize captured text the same way the production pipeline does:
/// CRLF/CR become LF, leading and trailing whitespace are preserved.
pub fn normalize_captured_text(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn fingerprint_text(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

pub fn preview_from_content(content: &str) -> String {
    let mut preview: String = content.chars().take(PREVIEW_CHARS).collect();
    if content.chars().count() > PREVIEW_CHARS {
        preview.push('…');
    }
    preview
}

pub fn detect_content_type(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("www.")
        || (trimmed.contains("://")
            && trimmed.split("://").next().is_some_and(|scheme| {
                !scheme.is_empty()
                    && scheme
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
            }))
    {
        return "url".to_owned();
    }

    let mut score = 0;
    for keyword in [
        "import ",
        "const ",
        "let ",
        "var ",
        "function ",
        "class ",
        "pub fn ",
        "impl ",
        "#include",
        "package ",
        "interface ",
        "namespace ",
        "void ",
        "return ",
        "if (",
        "for (",
        "while (",
        "=>",
    ] {
        if text.contains(keyword) {
            score += 1;
        }
    }
    if text.contains(';') {
        score += 1;
    }
    if text.contains('{') && text.contains('}') {
        score += 1;
    }
    if text.contains("</") && text.contains('>') {
        score += 2;
    }
    if score >= 2 {
        return "code".to_owned();
    }
    "text".to_owned()
}

/// Extract a pasteable HTML fragment from CF_HTML or a raw fragment.
pub fn extract_html_fragment(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(start) = trimmed.find("<!--StartFragment-->") {
        let from = start + "<!--StartFragment-->".len();
        if let Some(end) = trimmed[from..].find("<!--EndFragment-->") {
            let fragment = trimmed[from..from + end].trim();
            if fragment.is_empty() {
                return None;
            }
            return Some(fragment.to_owned());
        }
    }
    if trimmed.contains("Version:") && trimmed.contains("StartHTML:") {
        return trimmed.find('<').map(|idx| trimmed[idx..].trim().to_owned());
    }
    if trimmed.contains('<') && trimmed.contains('>') {
        return Some(trimmed.to_owned());
    }
    None
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipboardSnapshot {
    pub text: Option<String>,
    pub html: Option<String>,
    pub image: Option<String>,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapturedPayload {
    Text {
        content: String,
        content_type: String,
    },
    RichText {
        content: String,
        html: String,
    },
    Image {
        content: String,
    },
    Files {
        paths: Vec<String>,
    },
}

impl CapturedPayload {
    pub fn content_type(&self) -> &str {
        match self {
            Self::Text { content_type, .. } => content_type,
            Self::RichText { .. } => "rich_text",
            Self::Image { .. } => "image",
            Self::Files { .. } => "file",
        }
    }

    pub fn content(&self) -> String {
        match self {
            Self::Text { content, .. } | Self::RichText { content, .. } | Self::Image { content } => {
                content.clone()
            }
            Self::Files { paths } => paths.join("\n"),
        }
    }

    pub fn html(&self) -> Option<&str> {
        match self {
            Self::RichText { html, .. } => Some(html.as_str()),
            _ => None,
        }
    }

    pub fn preview(&self) -> String {
        match self {
            Self::Text { content, .. } | Self::RichText { content, .. } => {
                preview_from_content(content)
            }
            Self::Image { content } => {
                let name = Path::new(content)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("image");
                format!("Image · {name}")
            }
            Self::Files { paths } => {
                let names: Vec<&str> = paths
                    .iter()
                    .map(|path| {
                        Path::new(path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or(path.as_str())
                    })
                    .collect();
                preview_from_content(&names.join("\n"))
            }
        }
    }

    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.content_type().hash(&mut hasher);
        self.content().hash(&mut hasher);
        self.html().unwrap_or_default().hash(&mut hasher);
        hasher.finish()
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text { content, .. } => content.is_empty(),
            Self::RichText { content, html } => content.is_empty() && html.trim().is_empty(),
            Self::Image { content } => content.trim().is_empty(),
            Self::Files { paths } => paths.is_empty(),
        }
    }
}

/// Choose one payload from a platform snapshot.
///
/// Priority matches production enough for a daily-driver probe: files, then
/// rich text when HTML is present with text, then a pure image, then text.
pub fn classify_snapshot(snapshot: ClipboardSnapshot) -> Option<CapturedPayload> {
    let files: Vec<String> = snapshot
        .files
        .into_iter()
        .map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())
        .collect();
    if !files.is_empty() {
        return Some(CapturedPayload::Files { paths: files });
    }

    let text = snapshot
        .text
        .as_deref()
        .map(normalize_captured_text)
        .filter(|value| !value.is_empty());
    let html = snapshot
        .html
        .as_deref()
        .and_then(extract_html_fragment)
        .filter(|value| !value.trim().is_empty());

    if let (Some(content), Some(html)) = (text.clone(), html) {
        return Some(CapturedPayload::RichText { content, html });
    }

    if text.is_none() {
        if let Some(content) = snapshot
            .image
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            return Some(CapturedPayload::Image { content });
        }
    }

    text.map(|content| {
        let content_type = detect_content_type(&content);
        CapturedPayload::Text {
            content,
            content_type,
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureSkip {
    Empty,
    Duplicate,
    Echo,
}

impl CaptureSkip {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Duplicate => "duplicate",
            Self::Echo => "echo",
        }
    }
}

/// Filters OS clipboard snapshots before they are persisted.
#[derive(Clone, Debug, Default)]
pub struct CaptureFilter {
    last: Option<u64>,
    echoes: VecDeque<u64>,
}

impl CaptureFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn prime(&mut self, raw: &str) {
        if let Some(payload) = classify_snapshot(ClipboardSnapshot {
            text: Some(raw.to_owned()),
            ..ClipboardSnapshot::default()
        }) {
            self.prime_payload(&payload);
        }
    }

    pub fn prime_payload(&mut self, payload: &CapturedPayload) {
        if !payload.is_empty() {
            self.last = Some(payload.fingerprint());
        }
    }

    pub fn note_self_write(&mut self, raw: &str) {
        if let Some(payload) = classify_snapshot(ClipboardSnapshot {
            text: Some(raw.to_owned()),
            ..ClipboardSnapshot::default()
        }) {
            self.note_payload(&payload);
        }
    }

    pub fn note_payload(&mut self, payload: &CapturedPayload) {
        if payload.is_empty() {
            return;
        }
        let fingerprint = payload.fingerprint();
        self.last = Some(fingerprint);
        self.echoes.push_front(fingerprint);
        self.echoes.truncate(ECHO_RING);
    }

    pub fn accept(&mut self, raw: &str) -> Result<String, CaptureSkip> {
        match self.accept_payload(classify_snapshot(ClipboardSnapshot {
            text: Some(raw.to_owned()),
            ..ClipboardSnapshot::default()
        }))? {
            CapturedPayload::Text { content, .. } | CapturedPayload::RichText { content, .. } => {
                Ok(content)
            }
            other => Ok(other.content()),
        }
    }

    pub fn accept_payload(
        &mut self,
        payload: Option<CapturedPayload>,
    ) -> Result<CapturedPayload, CaptureSkip> {
        let payload = payload.ok_or(CaptureSkip::Empty)?;
        if payload.is_empty() {
            return Err(CaptureSkip::Empty);
        }
        let fingerprint = payload.fingerprint();
        if self.echoes.contains(&fingerprint) {
            return Err(CaptureSkip::Echo);
        }
        if self.last == Some(fingerprint) {
            return Err(CaptureSkip::Duplicate);
        }
        self.last = Some(fingerprint);
        Ok(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_captured_text_maps_crlf_and_keeps_whitespace() {
        assert_eq!(normalize_captured_text("  a\r\nb\rc  "), "  a\nb\nc  ");
    }

    #[test]
    fn filter_skips_empty_duplicate_and_echo() {
        let mut filter = CaptureFilter::new();
        assert_eq!(filter.accept(""), Err(CaptureSkip::Empty));
        assert_eq!(filter.accept("hello").unwrap(), "hello");
        assert_eq!(filter.accept("hello"), Err(CaptureSkip::Duplicate));

        filter.note_self_write("pasted");
        assert_eq!(filter.accept("pasted"), Err(CaptureSkip::Echo));
        assert_eq!(filter.accept("next").unwrap(), "next");
    }

    #[test]
    fn prime_suppresses_the_startup_clipboard_snapshot() {
        let mut filter = CaptureFilter::new();
        filter.prime("already on clipboard");
        assert_eq!(
            filter.accept("already on clipboard"),
            Err(CaptureSkip::Duplicate)
        );
        assert_eq!(filter.accept("new copy").unwrap(), "new copy");
    }

    #[test]
    fn preview_from_content_does_not_trim() {
        let preview = preview_from_content("  keep  ");
        assert_eq!(preview, "  keep  ");
        let long = "x".repeat(300);
        let preview = preview_from_content(&long);
        assert!(preview.ends_with('…'));
        assert_eq!(preview.chars().count(), PREVIEW_CHARS + 1);
    }

    #[test]
    fn classify_prefers_files_then_html_then_image_then_text() {
        assert_eq!(
            classify_snapshot(ClipboardSnapshot {
                text: Some("ignored".into()),
                files: vec!["C:\\tmp\\a.txt".into()],
                ..ClipboardSnapshot::default()
            }),
            Some(CapturedPayload::Files {
                paths: vec!["C:\\tmp\\a.txt".into()]
            })
        );

        let rich = classify_snapshot(ClipboardSnapshot {
            text: Some("hello\r\nworld".into()),
            html: Some(
                "Version:1.0\r\nStartHTML:0000000000\r\n<!--StartFragment--><b>hello</b><!--EndFragment-->".into(),
            ),
            image: Some("C:\\tmp\\shot.png".into()),
            ..ClipboardSnapshot::default()
        })
        .unwrap();
        assert_eq!(rich.content_type(), "rich_text");
        assert_eq!(rich.content(), "hello\nworld");
        assert_eq!(rich.html(), Some("<b>hello</b>"));

        assert_eq!(
            classify_snapshot(ClipboardSnapshot {
                image: Some("C:\\tmp\\shot.png".into()),
                ..ClipboardSnapshot::default()
            }),
            Some(CapturedPayload::Image {
                content: "C:\\tmp\\shot.png".into()
            })
        );

        let url = classify_snapshot(ClipboardSnapshot {
            text: Some("https://example.com".into()),
            ..ClipboardSnapshot::default()
        })
        .unwrap();
        assert_eq!(url.content_type(), "url");
    }

    #[test]
    fn classify_does_not_treat_a_bitmap_as_an_image_when_text_is_present() {
        let text = classify_snapshot(ClipboardSnapshot {
            text: Some("plain".into()),
            image: Some("C:\\tmp\\shot.png".into()),
            ..ClipboardSnapshot::default()
        })
        .unwrap();
        assert_eq!(text.content_type(), "text");
        assert_eq!(text.content(), "plain");
    }

    #[test]
    fn filter_dedups_image_and_file_payloads() {
        let mut filter = CaptureFilter::new();
        let image = CapturedPayload::Image {
            content: "C:\\tmp\\shot.png".into(),
        };
        assert!(filter.accept_payload(Some(image.clone())).is_ok());
        assert_eq!(
            filter.accept_payload(Some(image.clone())),
            Err(CaptureSkip::Duplicate)
        );
        filter.note_payload(&CapturedPayload::Files {
            paths: vec!["C:\\tmp\\a.txt".into()],
        });
        assert_eq!(
            filter.accept_payload(Some(CapturedPayload::Files {
                paths: vec!["C:\\tmp\\a.txt".into()]
            })),
            Err(CaptureSkip::Echo)
        );
    }
}
