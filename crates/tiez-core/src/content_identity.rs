//! Stable clipboard-content identity shared by both desktop runtimes.

use base64::Engine;

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

/// Image identity compatible with the production Tauri repository.
///
/// File paths and data URLs are decoded and resized to 32×32 before hashing.
/// Undecodable bytes retain the historical raw-byte fallback.
pub fn calc_image_hash(content: &str) -> Option<i64> {
    let trimmed = content.trim();
    let bytes =
        if !trimmed.starts_with("data:") && (trimmed.starts_with('/') || trimmed.contains(":\\")) {
            std::fs::read(trimmed).ok()?
        } else {
            let parts: Vec<&str> = trimmed.splitn(2, ',').collect();
            let payload = if parts.len() == 2 { parts[1] } else { trimmed };
            let payload_clean = payload.replace('\r', "").replace('\n', "");
            if payload_clean.trim().is_empty() {
                return None;
            }
            base64::engine::general_purpose::STANDARD
                .decode(payload_clean.trim())
                .ok()?
        };

    if let Ok(image) = image::load_from_memory(&bytes) {
        let thumbnail = image.resize_exact(32, 32, image::imageops::FilterType::Nearest);
        return Some(hash_bytes(thumbnail.as_bytes()));
    }

    Some(hash_bytes(&bytes))
}

fn hash_bytes(bytes: &[u8]) -> i64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish() as i64
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

    #[test]
    fn image_hash_accepts_raw_base64_and_data_urls() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lW2ZAAAAAElFTkSuQmCC";

        assert_eq!(
            calc_image_hash(png),
            calc_image_hash(&format!("data:image/png;base64,{png}"))
        );
        assert!(calc_image_hash(png).is_some());
    }
}
