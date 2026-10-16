//! Shared discovery and transformation. Runtime/state ownership belongs to the host.
use super::data::ClipboardData;
use super::utils::*;
use crate::database::is_text_type;
use crate::domain::models::ClipboardEntry;
use base64::Engine;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct TransformOptions {
    pub persistent: bool,
    pub cleanup_rules: String,
    pub app_cleanup_policies: String,
    pub privacy_protection: bool,
    pub privacy_kinds: Vec<String>,
    pub privacy_custom_rules: Vec<String>,
}
impl Default for TransformOptions {
    fn default() -> Self {
        Self {
            persistent: false,
            cleanup_rules: String::new(),
            app_cleanup_policies: "[]".into(),
            privacy_protection: true,
            privacy_kinds: ["phone", "idcard", "email", "secret"]
                .map(str::to_string)
                .to_vec(),
            privacy_custom_rules: Vec::new(),
        }
    }
}
pub(super) fn normalize_text_preserving_edge_whitespace(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn discover(
    data: &ClipboardData,
    source_app: &str,
    source_app_path: Option<&str>,
    timestamp: i64,
) -> ClipboardEntry {
    let (content_type, content, html_content) = match data {
        ClipboardData::Text(t) => (detect_content_type(t), t.clone(), None),
        ClipboardData::RichText { text, html } => (
            "rich_text".to_string(),
            derive_rich_text_content(text, Some(html)),
            Some(html.clone()),
        ),
        ClipboardData::Image { data_url } => ("image".to_string(), data_url.clone(), None),
        ClipboardData::Files(f) => {
            let content = f.join("\n");
            if f.len() == 1 {
                let path = &f[0];
                let lower = path.to_lowercase();
                if lower.ends_with(".gif") {
                    ("image".to_string(), path.clone(), None)
                } else if lower.ends_with(".png")
                    || lower.ends_with(".jpg")
                    || lower.ends_with(".jpeg")
                    || lower.ends_with(".bmp")
                    || lower.ends_with(".webp")
                {
                    if let Ok(img_data) = std::fs::read(path) {
                        if is_gif_signature(&img_data) {
                            let b64 = base64::engine::general_purpose::STANDARD.encode(&img_data);
                            (
                                "image".to_string(),
                                format!("data:image/gif;base64,{}", b64),
                                None,
                            )
                        } else if is_animated_webp_payload(&img_data) {
                            let b64 = base64::engine::general_purpose::STANDARD.encode(&img_data);
                            (
                                "image".to_string(),
                                format!("data:image/webp;base64,{}", b64),
                                None,
                            )
                        } else if let Ok(img) = image::load_from_memory(&img_data) {
                            let mut bytes: Vec<u8> = Vec::new();
                            let mut cursor = std::io::Cursor::new(&mut bytes);
                            if img.write_to(&mut cursor, image::ImageFormat::Png).is_ok() {
                                let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                                (
                                    "image".to_string(),
                                    format!("data:image/png;base64,{}", b64),
                                    None,
                                )
                            } else {
                                ("file".to_string(), content, None)
                            }
                        } else {
                            ("file".to_string(), content, None)
                        }
                    } else {
                        ("file".to_string(), content, None)
                    }
                } else if lower.ends_with(".mp4")
                    || lower.ends_with(".mkv")
                    || lower.ends_with(".avi")
                    || lower.ends_with(".mov")
                    || lower.ends_with(".wmv")
                    || lower.ends_with(".flv")
                    || lower.ends_with(".webm")
                {
                    ("video".to_string(), path.clone(), None)
                } else {
                    ("file".to_string(), content, None)
                }
            } else {
                ("file".to_string(), content, None)
            }
        }
    };

    let preview = build_entry_preview(&content_type, &content, html_content.as_deref());

    let is_external =
        (content_type == "file" || content_type == "video" || content_type == "image")
            && !content.starts_with("data:");

    ClipboardEntry {
        id: 0,
        content_type,
        content,
        html_content,
        source_app: source_app.to_string(),
        source_app_path: source_app_path.map(str::to_string),
        timestamp: timestamp,
        preview,
        is_pinned: false,
        tags: Vec::new(),
        use_count: 0,
        is_external,
        pinned_order: 0,
        file_preview_exists: true,
    }
}

pub fn transform(entry: &mut ClipboardEntry, options: &TransformOptions, data_dir: &Path) -> bool {
    // Normalize line endings, but preserve meaningful leading/trailing whitespace.
    entry.content = normalize_text_preserving_edge_whitespace(&entry.content);

    let app_cleanup_policies_raw = options.app_cleanup_policies.clone();
    if !app_cleanup_policies_raw.trim().is_empty() {
        let app_cleanup_policies = parse_app_cleanup_policies(&app_cleanup_policies_raw);
        for policy in app_cleanup_policies {
            if !app_cleanup_policy_matches(
                &policy,
                &entry.source_app,
                entry.source_app_path.as_deref(),
                &entry.content_type,
            ) {
                continue;
            }

            if policy.action.eq_ignore_ascii_case("ignore") {
                return false;
            }

            if is_text_type(&entry.content_type) && !policy.cleanup_rules.trim().is_empty() {
                let rules = parse_cleanup_rules(&policy.cleanup_rules);
                if !rules.is_empty() {
                    let cleaned = apply_cleanup_rules(&entry.content, &rules);
                    if cleaned.trim().is_empty() || cleaned == "__IGNORE_CAPTURE__" {
                        return false;
                    }
                    entry.content = normalize_text_preserving_edge_whitespace(&cleaned);
                    entry.preview = build_entry_preview(&entry.content_type, &entry.content, None);
                }
            }
        }
    }

    if is_text_type(&entry.content_type) {
        let cleanup_rules_raw = options.cleanup_rules.clone();
        if !cleanup_rules_raw.trim().is_empty() {
            let cleanup_rules = parse_cleanup_rules(&cleanup_rules_raw);
            if !cleanup_rules.is_empty() {
                let cleaned = apply_cleanup_rules(&entry.content, &cleanup_rules);
                if cleaned.trim().is_empty() || cleaned == "__IGNORE_CAPTURE__" {
                    return false;
                }
                entry.content = normalize_text_preserving_edge_whitespace(&cleaned);
                entry.preview = build_entry_preview(&entry.content_type, &entry.content, None);
            }
        }
    }

    // Sensitive Info
    let protect_kinds = options.privacy_kinds.clone();
    let custom_rules = options.privacy_custom_rules.clone();
    if options.privacy_protection && is_text_type(&entry.content_type) {
        if contains_sensitive_info(&entry.content, &protect_kinds, &custom_rules) {
            entry.tags.push("sensitive".to_string());
        }
    }

    // Rich Text Image Processing
    if let Some(html) = &entry.html_content {
        entry.html_content = if options.persistent {
            let html_with_local_assets = process_local_images_in_html(html, &data_dir);
            Some(externalize_rich_image_fallback(
                &html_with_local_assets,
                &data_dir,
            ))
        } else {
            Some(embed_local_images(html, &data_dir))
        };
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_discovery_and_transform_preserve_edges_and_source() {
        let mut entry = discover(
            &ClipboardData::Text("  甲\r\n乙\r  ".into()),
            "editor.exe",
            Some("C:/editor.exe"),
            123,
        );
        assert!(transform(
            &mut entry,
            &TransformOptions::default(),
            Path::new("C:/isolated")
        ));
        assert_eq!(entry.content, "  甲\n乙\n  ");
        assert_eq!(entry.source_app, "editor.exe");
        assert_eq!(entry.source_app_path.as_deref(), Some("C:/editor.exe"));
        assert_eq!(entry.timestamp, 123);
    }
    #[test]
    fn per_app_ignore_and_cleanup_precede_global_cleanup() {
        let mut entry = discover(
            &ClipboardData::Text("  alpha  ".into()),
            "editor.exe",
            None,
            1,
        );
        let options = TransformOptions {
            app_cleanup_policies:
                r#"[{"appName":"editor.exe","action":"clean","cleanupRules":"alpha=>beta"}]"#.into(),
            cleanup_rules: "beta=>gamma".into(),
            ..Default::default()
        };
        assert!(transform(&mut entry, &options, Path::new("C:/isolated")));
        assert_eq!(entry.content, "  gamma  ");
        let ignore = TransformOptions {
            app_cleanup_policies: r#"[{"appName":"editor.exe","action":"ignore"}]"#.into(),
            ..Default::default()
        };
        assert!(!transform(&mut entry, &ignore, Path::new("C:/isolated")));
    }
    #[test]
    fn privacy_tagging_and_original_rich_payload_survive_discovery() {
        let html = "<p>user@example.com</p>";
        let mut entry = discover(
            &ClipboardData::RichText {
                text: " user@example.com ".into(),
                html: html.into(),
            },
            "word.exe",
            None,
            1,
        );
        assert!(transform(
            &mut entry,
            &TransformOptions::default(),
            Path::new("C:/isolated")
        ));
        assert!(entry.tags.iter().any(|tag| tag == "sensitive"));
        assert_eq!(entry.html_content.as_deref(), Some(html));
        assert_eq!(entry.content, " user@example.com ");
    }
    #[test]
    fn files_and_video_keep_original_paths_and_external_flag() {
        let paths = vec!["C:/first.txt".into(), "C:/second.txt".into()];
        let file = discover(
            &ClipboardData::Files(paths.clone()),
            "explorer.exe",
            None,
            1,
        );
        assert_eq!(file.content, paths.join("\n"));
        assert_eq!(file.content_type, "file");
        assert!(file.is_external);
        let video = discover(
            &ClipboardData::Files(vec!["C:/movie.mp4".into()]),
            "explorer.exe",
            None,
            1,
        );
        assert_eq!(video.content_type, "video");
        assert!(video.is_external);
    }
}
