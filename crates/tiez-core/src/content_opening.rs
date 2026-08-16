//! Tauri-independent planning for opening clipboard content with platform handlers.
//!
//! The shared core resolves protected payloads, validates URLs, and materializes
//! temporary files. Native frontends remain responsible for the final OS launch
//! so no command shell or frontend-specific handle crosses this boundary.

use crate::clipboard_history::HistoryContent;
use base64::{engine::general_purpose, Engine as _};
use serde::Serialize;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENCODED_IMAGE_BYTES: usize = 192 * 1024 * 1024;
const MAX_TEMP_FILE_ATTEMPTS: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenTargetKind {
    Url,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OpenContentPlan {
    pub item_id: i64,
    pub kind: OpenTargetKind,
    pub target: String,
    pub temporary: bool,
    pub requires_confirmation: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenContentError {
    Unavailable { item_id: i64, reason: String },
    Sensitive { item_id: i64 },
    Empty { item_id: i64 },
    InvalidUrl { item_id: i64, reason: String },
    MissingFile { item_id: i64, path: String },
    InvalidImage { item_id: i64, reason: String },
    Storage { item_id: i64, reason: String },
}

impl fmt::Display for OpenContentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { item_id, reason } => {
                write!(formatter, "记录 {item_id} 的内容不可用：{reason}")
            }
            Self::Sensitive { item_id } => {
                write!(formatter, "记录 {item_id} 受隐私保护，不能交给外部应用打开")
            }
            Self::Empty { item_id } => write!(formatter, "记录 {item_id} 没有可打开的内容"),
            Self::InvalidUrl { item_id, reason } => {
                write!(formatter, "记录 {item_id} 的链接不安全或无效：{reason}")
            }
            Self::MissingFile { item_id, path } => {
                write!(formatter, "记录 {item_id} 指向的文件不存在：{path}")
            }
            Self::InvalidImage { item_id, reason } => {
                write!(formatter, "记录 {item_id} 的图片无法打开：{reason}")
            }
            Self::Storage { item_id, reason } => {
                write!(formatter, "无法为记录 {item_id} 创建临时文件：{reason}")
            }
        }
    }
}

impl std::error::Error for OpenContentError {}

/// Resolves one history payload into a validated URL or local file target.
///
/// Sensitive entries are rejected even when the writable history adapter can
/// decrypt them. This prevents external applications and temporary files from
/// bypassing the native privacy surface.
pub fn prepare_open_content(
    content: &HistoryContent,
    temporary_root: &Path,
) -> Result<OpenContentPlan, OpenContentError> {
    if content.is_sensitive {
        return Err(OpenContentError::Sensitive {
            item_id: content.id,
        });
    }
    if !content.available {
        return Err(OpenContentError::Unavailable {
            item_id: content.id,
            reason: content
                .unavailable_reason
                .clone()
                .unwrap_or_else(|| "内容仍受保护".to_owned()),
        });
    }

    match content.content_type.as_str() {
        "url" | "link" => prepare_url(content),
        "file" | "files" | "video" => prepare_existing_file(content),
        "image" => prepare_image(content, temporary_root),
        "rich_text" | "html" => prepare_rich_text(content, temporary_root),
        _ => prepare_text(content, temporary_root),
    }
}

fn prepare_url(content: &HistoryContent) -> Result<OpenContentPlan, OpenContentError> {
    let raw = content.content.trim();
    if raw.is_empty() {
        return Err(OpenContentError::Empty {
            item_id: content.id,
        });
    }
    if raw.chars().any(|character| character.is_control()) {
        return Err(invalid_url(content.id, "链接包含控制字符"));
    }

    let normalized = if raw.to_ascii_lowercase().starts_with("www.") {
        format!("https://{raw}")
    } else {
        raw.to_owned()
    };
    let scheme_end = normalized
        .find(':')
        .ok_or_else(|| invalid_url(content.id, "缺少协议"))?;
    let scheme = &normalized[..scheme_end];
    if !valid_scheme(scheme) {
        return Err(invalid_url(content.id, "协议格式无效"));
    }

    let lowercase_scheme = scheme.to_ascii_lowercase();
    let requires_confirmation = match lowercase_scheme.as_str() {
        "http" | "https" => {
            let expected_prefix = format!("{lowercase_scheme}://");
            if !normalized
                .to_ascii_lowercase()
                .starts_with(&expected_prefix)
            {
                return Err(invalid_url(content.id, "HTTP(S) 链接缺少 //"));
            }
            false
        }
        "mailto" => false,
        "javascript" | "data" | "file" | "shell" | "vbscript" | "ms-settings" => {
            return Err(invalid_url(content.id, "该协议不允许交给系统处理"));
        }
        _ => true,
    };

    Ok(OpenContentPlan {
        item_id: content.id,
        kind: OpenTargetKind::Url,
        target: normalized,
        temporary: false,
        requires_confirmation,
    })
}

fn prepare_existing_file(content: &HistoryContent) -> Result<OpenContentPlan, OpenContentError> {
    let path_value = content
        .content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or(OpenContentError::Empty {
            item_id: content.id,
        })?;
    let path = Path::new(path_value);
    if !path.exists() {
        return Err(OpenContentError::MissingFile {
            item_id: content.id,
            path: path_value.to_owned(),
        });
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| OpenContentError::Storage {
            item_id: content.id,
            reason: error.to_string(),
        })?;
    Ok(file_plan(content.id, canonical, false, false))
}

fn prepare_image(
    content: &HistoryContent,
    temporary_root: &Path,
) -> Result<OpenContentPlan, OpenContentError> {
    let value = content.content.trim();
    if value.is_empty() {
        return Err(OpenContentError::Empty {
            item_id: content.id,
        });
    }

    let path = Path::new(value);
    if path.exists() {
        let canonical = path
            .canonicalize()
            .map_err(|error| OpenContentError::Storage {
                item_id: content.id,
                reason: error.to_string(),
            })?;
        return Ok(file_plan(content.id, canonical, false, false));
    }

    if value.len() > MAX_ENCODED_IMAGE_BYTES {
        return Err(OpenContentError::InvalidImage {
            item_id: content.id,
            reason: "编码后的图片超过 192 MiB 上限".to_owned(),
        });
    }

    let (encoded, declared_extension) = if value.starts_with("data:image/") {
        let (metadata, encoded) =
            value
                .split_once(',')
                .ok_or_else(|| OpenContentError::InvalidImage {
                    item_id: content.id,
                    reason: "data URL 缺少图片数据".to_owned(),
                })?;
        if !metadata.to_ascii_lowercase().ends_with(";base64") {
            return Err(OpenContentError::InvalidImage {
                item_id: content.id,
                reason: "仅支持 base64 图片 data URL".to_owned(),
            });
        }
        (encoded, extension_from_media_type(metadata))
    } else {
        (value, None)
    };
    let bytes = general_purpose::STANDARD.decode(encoded).map_err(|error| {
        OpenContentError::InvalidImage {
            item_id: content.id,
            reason: error.to_string(),
        }
    })?;
    let detected_format =
        image::guess_format(&bytes).map_err(|error| OpenContentError::InvalidImage {
            item_id: content.id,
            reason: error.to_string(),
        })?;
    let extension =
        image_extension(detected_format).ok_or_else(|| OpenContentError::InvalidImage {
            item_id: content.id,
            reason: "不支持此图片格式".to_owned(),
        })?;
    if declared_extension.is_some_and(|declared| declared != extension) {
        return Err(OpenContentError::InvalidImage {
            item_id: content.id,
            reason: "data URL 声明格式与图片内容不一致".to_owned(),
        });
    }
    let path = write_temporary_file(content.id, temporary_root, extension, &bytes)?;
    Ok(file_plan(content.id, path, true, false))
}

fn prepare_rich_text(
    content: &HistoryContent,
    temporary_root: &Path,
) -> Result<OpenContentPlan, OpenContentError> {
    if let Some(html) = content
        .html_content
        .as_deref()
        .map(str::trim)
        .filter(|html| !html.is_empty())
    {
        let document = html_document(html);
        let path = write_temporary_file(content.id, temporary_root, "html", document.as_bytes())?;
        return Ok(file_plan(content.id, path, true, true));
    }
    prepare_text(content, temporary_root)
}

fn prepare_text(
    content: &HistoryContent,
    temporary_root: &Path,
) -> Result<OpenContentPlan, OpenContentError> {
    if content.content.is_empty() {
        return Err(OpenContentError::Empty {
            item_id: content.id,
        });
    }
    let path = write_temporary_file(
        content.id,
        temporary_root,
        "txt",
        content.content.as_bytes(),
    )?;
    Ok(file_plan(content.id, path, true, false))
}

fn write_temporary_file(
    item_id: i64,
    root: &Path,
    extension: &str,
    bytes: &[u8],
) -> Result<PathBuf, OpenContentError> {
    fs::create_dir_all(root).map_err(|error| OpenContentError::Storage {
        item_id,
        reason: error.to_string(),
    })?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let id_label = item_id.to_string().replace('-', "session-");

    for attempt in 0..MAX_TEMP_FILE_ATTEMPTS {
        let path = root.join(format!(
            "TieZ_Clip_{id_label}_{}_{timestamp}_{attempt}.{extension}",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(bytes)
                    .map_err(|error| OpenContentError::Storage {
                        item_id,
                        reason: error.to_string(),
                    })?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(OpenContentError::Storage {
                    item_id,
                    reason: error.to_string(),
                });
            }
        }
    }

    Err(OpenContentError::Storage {
        item_id,
        reason: "无法分配唯一的临时文件名".to_owned(),
    })
}

fn file_plan(
    item_id: i64,
    target: PathBuf,
    temporary: bool,
    requires_confirmation: bool,
) -> OpenContentPlan {
    OpenContentPlan {
        item_id,
        kind: OpenTargetKind::File,
        target: target.to_string_lossy().into_owned(),
        temporary,
        requires_confirmation,
    }
}

fn valid_scheme(scheme: &str) -> bool {
    let mut characters = scheme.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn invalid_url(item_id: i64, reason: &str) -> OpenContentError {
    OpenContentError::InvalidUrl {
        item_id,
        reason: reason.to_owned(),
    }
}

fn extension_from_media_type(metadata: &str) -> Option<&'static str> {
    let lowercase = metadata.to_ascii_lowercase();
    if lowercase.starts_with("data:image/gif;") {
        Some("gif")
    } else if lowercase.starts_with("data:image/jpeg;") || lowercase.starts_with("data:image/jpg;")
    {
        Some("jpg")
    } else if lowercase.starts_with("data:image/bmp;") {
        Some("bmp")
    } else if lowercase.starts_with("data:image/webp;") {
        Some("webp")
    } else if lowercase.starts_with("data:image/png;") {
        Some("png")
    } else {
        None
    }
}

fn image_extension(format: image::ImageFormat) -> Option<&'static str> {
    match format {
        image::ImageFormat::Png => Some("png"),
        image::ImageFormat::Gif => Some("gif"),
        image::ImageFormat::Jpeg => Some("jpg"),
        image::ImageFormat::Bmp => Some("bmp"),
        image::ImageFormat::WebP => Some("webp"),
        _ => None,
    }
}

fn html_document(html: &str) -> String {
    let lowercase = html.to_ascii_lowercase();
    if lowercase.contains("<html") || lowercase.starts_with("<!doctype") {
        html.to_owned()
    } else {
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"></head><body>{html}</body></html>"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn available(item_id: i64, content_type: &str, content: &str) -> HistoryContent {
        HistoryContent {
            id: item_id,
            content_type: content_type.to_owned(),
            content: content.to_owned(),
            html_content: None,
            available: true,
            is_sensitive: false,
            unavailable_reason: None,
        }
    }

    fn temporary_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tiez-core-open-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn normalizes_web_urls_and_requires_confirmation_for_custom_protocols() {
        let root = temporary_directory("urls");
        let web = prepare_open_content(&available(1, "url", " www.example.com/a "), &root).unwrap();
        assert_eq!(web.kind, OpenTargetKind::Url);
        assert_eq!(web.target, "https://www.example.com/a");
        assert!(!web.requires_confirmation);

        let custom =
            prepare_open_content(&available(2, "url", "myapp+desktop://open/page"), &root).unwrap();
        assert!(custom.requires_confirmation);
        assert!(prepare_open_content(&available(3, "url", "javascript:alert(1)"), &root).is_err());
    }

    #[test]
    fn opens_existing_files_without_copying_them() {
        let root = temporary_directory("existing");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("日常文件.txt");
        fs::write(&source, "hello").unwrap();

        let plan = prepare_open_content(
            &available(4, "file", &format!("{}\nignored", source.display())),
            &root.join("generated"),
        )
        .unwrap();
        assert_eq!(plan.kind, OpenTargetKind::File);
        assert_eq!(Path::new(&plan.target), source.canonicalize().unwrap());
        assert!(!plan.temporary);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materializes_utf8_text_and_rich_html_without_trimming() {
        let root = temporary_directory("documents");
        let text = available(5, "text", "  中文内容\r\n保留空白  ");
        let text_plan = prepare_open_content(&text, &root).unwrap();
        assert!(text_plan.temporary);
        assert_eq!(fs::read_to_string(&text_plan.target).unwrap(), text.content);

        let mut rich = available(6, "rich_text", "fallback");
        rich.html_content = Some("<b>富文本</b>".to_owned());
        let rich_plan = prepare_open_content(&rich, &root).unwrap();
        assert!(rich_plan.requires_confirmation);
        let html = fs::read_to_string(&rich_plan.target).unwrap();
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html.contains("<b>富文本</b>"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materializes_base64_images_and_rejects_sensitive_content() {
        let root = temporary_directory("images");
        let image = available(
            7,
            "image",
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB",
        );
        let plan = prepare_open_content(&image, &root).unwrap();
        assert!(plan.target.ends_with(".png"));
        assert!(Path::new(&plan.target).is_file());

        assert!(matches!(
            prepare_open_content(
                &available(8, "image", "data:image/png;base64,R0lGODlhAQAB"),
                &root
            ),
            Err(OpenContentError::InvalidImage { item_id: 8, .. })
        ));

        let mut sensitive = available(9, "text", "secret");
        sensitive.is_sensitive = true;
        assert!(matches!(
            prepare_open_content(&sensitive, &root),
            Err(OpenContentError::Sensitive { item_id: 9 })
        ));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unavailable_empty_and_missing_file_payloads() {
        let root = temporary_directory("rejections");
        let mut unavailable = available(9, "text", "secret");
        unavailable.available = false;
        unavailable.unavailable_reason = Some("encrypted".to_owned());
        assert!(matches!(
            prepare_open_content(&unavailable, &root),
            Err(OpenContentError::Unavailable { item_id: 9, .. })
        ));
        assert!(matches!(
            prepare_open_content(&available(10, "text", ""), &root),
            Err(OpenContentError::Empty { item_id: 10 })
        ));
        assert!(matches!(
            prepare_open_content(&available(11, "file", "Z:/missing.txt"), &root),
            Err(OpenContentError::MissingFile { item_id: 11, .. })
        ));
    }
}
