//! Live clipboard ingest policy shared by native TieZ frontends.
//!
//! Platform adapters own OS notifications and format reads. This module owns
//! CRLF normalization (without trimming), consecutive-copy dedup, and
//! self-paste echo suppression.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

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

    /// Seed the filter with the clipboard already present at startup so the
    /// first notification does not ingest stale content.
    pub fn prime(&mut self, raw: &str) {
        let normalized = normalize_captured_text(raw);
        if !normalized.is_empty() {
            self.last = Some(fingerprint_text(&normalized));
        }
    }

    /// Remember a payload this process just wrote (paste or copy).
    pub fn note_self_write(&mut self, raw: &str) {
        let normalized = normalize_captured_text(raw);
        if normalized.is_empty() {
            return;
        }
        let fingerprint = fingerprint_text(&normalized);
        self.last = Some(fingerprint);
        self.echoes.push_front(fingerprint);
        self.echoes.truncate(ECHO_RING);
    }

    pub fn accept(&mut self, raw: &str) -> Result<String, CaptureSkip> {
        let normalized = normalize_captured_text(raw);
        if normalized.is_empty() {
            return Err(CaptureSkip::Empty);
        }
        let fingerprint = fingerprint_text(&normalized);
        if self.echoes.contains(&fingerprint) {
            return Err(CaptureSkip::Echo);
        }
        if self.last == Some(fingerprint) {
            return Err(CaptureSkip::Duplicate);
        }
        self.last = Some(fingerprint);
        Ok(normalized)
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
}
