//! Stable clipboard-content identity shared by both desktop runtimes.

pub fn is_text_type(content_type: &str) -> bool {
    matches!(content_type, "text" | "code" | "url" | "rich_text")
}

/// Content types whose identity is the normalized, readable text stored in
/// `content`.
pub fn uses_text_content_hash(content_type: &str) -> bool {
    is_text_type(content_type) || matches!(content_type, "file" | "video")
}

fn normalize_text(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn calc_text_hash(content: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let normalized = normalize_text(content);
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    hasher.finish()
}

/// Hash used by pre-whitespace-preserving sync payloads. This intentionally
/// reproduces the historical algorithm byte-for-byte: trim first, normalize
/// CRLF pairs to LF, and leave standalone CR bytes unchanged.
pub fn calc_legacy_text_hash(content: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let normalized = content.trim().replace("\r\n", "\n");
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_hash_normalizes_line_endings_and_preserves_edge_whitespace() {
        assert_eq!(calc_text_hash("a\r\nb\r"), calc_text_hash("a\nb\n"));
        assert_ne!(calc_text_hash("hello"), calc_text_hash("hello "));
    }

    #[test]
    fn legacy_hash_keeps_historical_trim_and_cr_behavior() {
        assert_eq!(
            calc_legacy_text_hash(" hello \r\n"),
            calc_legacy_text_hash("hello")
        );
        assert_ne!(calc_legacy_text_hash("a\rb"), calc_legacy_text_hash("a\nb"));
    }

    #[test]
    fn file_and_video_paths_use_readable_text_identity() {
        assert!(uses_text_content_hash("text"));
        assert!(uses_text_content_hash("file"));
        assert!(uses_text_content_hash("video"));
        assert!(!uses_text_content_hash("image"));
    }
}
