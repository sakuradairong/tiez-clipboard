//! Shared clipboard format probes. Keep original source-specific behavior here.
use super::data::ClipboardData;
use super::utils::*;
use arboard::Clipboard;
use base64::Engine;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const DEFAULT_CLIPBOARD_SETTLE_DELAY_MS: u64 = 100;
pub(super) const SNIPPING_TOOL_SETTLE_DELAY_MS: u64 = 1200;
pub(super) const RICH_TEXT_RETRY_DELAYS_MS: [u64; 7] = [0, 40, 80, 140, 220, 360, 560];
pub(super) const PRESERVED_NAMED_FORMAT_MAX_COUNT: usize = 12;
pub(super) const PRESERVED_NAMED_FORMAT_MAX_BYTES: usize = 1_500_000;
pub(super) const PRESERVED_NAMED_FORMAT_TOTAL_BYTES: usize = 4_000_000;
pub(super) const GIF_SOURCE_FORMAT_NAMES: [&str; 13] = [
    "GIF",
    "Animated GIF",
    "gif",
    "image/gif",
    "Graphics Interchange Format",
    "image/x-gif",
    "PNG",
    "image/png",
    "JFIF",
    "JPEG",
    "image/jpeg",
    "image/webp",
    "WebP",
];

#[cfg(any(target_os = "windows", test))]
pub(super) fn clipboard_preflight_hash(
    text: Option<&str>,
    image_bytes: Option<&[u8]>,
    files: Option<&[String]>,
) -> Option<u64> {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut has_content = false;
    if let Some(text) = text {
        "text".hash(&mut hasher);
        normalize_clipboard_plain_text(text).hash(&mut hasher);
        has_content = true;
    }
    if let Some(bytes) = image_bytes {
        "image".hash(&mut hasher);
        bytes.hash(&mut hasher);
        has_content = true;
    }
    if let Some(files) = files.filter(|files| !files.is_empty()) {
        "files".hash(&mut hasher);
        files.hash(&mut hasher);
        has_content = true;
    }
    has_content.then(|| hasher.finish())
}

pub(super) fn clear_recent_image_echo_state() {
    crate::LAST_APP_SET_HASH.store(0, Ordering::SeqCst);
    crate::LAST_APP_SET_HASH_ALT.store(0, Ordering::SeqCst);
    crate::LAST_APP_SET_IMAGE_VISUAL_HASH.store(0, Ordering::SeqCst);
}

pub(super) fn should_ignore_recent_image_echo(raw_hash: u64, visual_hash: u64) -> bool {
    let last_app_hash = crate::LAST_APP_SET_HASH.load(Ordering::SeqCst);
    let last_app_hash_alt = crate::LAST_APP_SET_HASH_ALT.load(Ordering::SeqCst);
    let last_app_visual_hash = crate::LAST_APP_SET_IMAGE_VISUAL_HASH.load(Ordering::SeqCst);
    let last_app_time = crate::LAST_APP_SET_TIMESTAMP.load(Ordering::SeqCst);
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if now_secs.saturating_sub(last_app_time) >= 10 {
        return false;
    }

    ((last_app_hash != 0) && (last_app_hash == raw_hash || last_app_hash_alt == raw_hash))
        || ((visual_hash != 0) && last_app_visual_hash == visual_hash)
}

pub(super) fn should_capture_file_entries(capture_files_enabled: bool) -> bool {
    capture_files_enabled
}

pub(super) fn is_gif_payload(data: &[u8]) -> bool {
    is_gif_signature(data)
}

pub(super) fn animated_image_mime(data: &[u8]) -> Option<&'static str> {
    if is_gif_payload(data) {
        Some("image/gif")
    } else if is_animated_webp_payload(data) {
        Some("image/webp")
    } else {
        None
    }
}

pub(super) fn clipboard_gif_payload() -> Option<Vec<u8>> {
    GIF_SOURCE_FORMAT_NAMES.iter().find_map(|name| {
        let data = unsafe {
            crate::infrastructure::windows_api::win_clipboard::get_clipboard_raw_format(name)
        }?;
        animated_image_mime(&data).is_some().then_some(data)
    })
}

/// Prefer the image pipeline over rich-text when the clipboard is effectively a
/// (possibly HTML-wrapped) animated GIF. Storing as rich text causes paste to
/// emit HTML/static previews instead of GIF clipboard formats.
pub(super) fn should_prefer_image_over_rich_text(
    text: &str,
    html: &str,
    has_native_gif: bool,
    has_html_animated_gif: bool,
    is_rich_text_source: bool,
    has_raster_image: impl FnOnce() -> bool,
) -> bool {
    if is_rich_text_source {
        return false;
    }
    if has_html_animated_gif {
        return is_standalone_image_html(text, html);
    }
    // Keep the bitmap probe lazy: Office/WPS can crash while rendering CF_DIB.
    text.trim().is_empty() && (has_native_gif || has_raster_image())
}

pub(super) fn resolve_animated_image_fallback(
    cached_html_image: Option<Option<String>>,
    read_html_image: impl FnOnce() -> Option<String>,
    read_text_image: impl FnOnce() -> Option<String>,
) -> Option<String> {
    // Some(None) means HTML was already probed unsuccessfully in this event.
    cached_html_image
        .unwrap_or_else(read_html_image)
        .or_else(read_text_image)
}

pub(super) fn single_gif_file_data_url(files: &[String]) -> Option<String> {
    use std::io::Read;

    if files.len() != 1 {
        return None;
    }
    let path = files[0].trim();
    if path.is_empty() || !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    // QQ and browsers may supply animated GIF/WebP through CF_HDROP with a
    // .jpg cache filename. Inspect bytes, not the extension. Read only a
    // header for non-animated files so copying a large document/video does
    // not load it into memory.
    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0; 64];
    let n = file.read(&mut header).ok()?;
    let header = &header[..n];
    animated_image_mime(header)?;
    let mut bytes = header.to_vec();
    file.read_to_end(&mut bytes).ok()?;
    let mime = animated_image_mime(&bytes)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{};base64,{}", mime, b64))
}

pub(super) fn file_clipboard_data(files: Vec<String>, capture_files_enabled: bool) -> Option<ClipboardData> {
    // Normalize real animated GIF/WebP before either capture-files branch.
    // Otherwise the file pipeline decodes a .jpg/.webp animation as a still
    // image and re-encodes it as PNG.
    if let Some(data_url) = single_gif_file_data_url(&files) {
        Some(ClipboardData::Image { data_url })
    } else if should_capture_file_entries(capture_files_enabled) {
        Some(ClipboardData::Files(files))
    } else {
        None
    }
}

pub(super) fn is_snipping_tool_source(
    source_snapshot: &crate::infrastructure::windows_api::window_tracker::ActiveAppInfo,
) -> bool {
    let app_name = source_snapshot.app_name.to_ascii_lowercase();
    let process_path = source_snapshot
        .process_path
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();

    [
        "snippingtool.exe",
        "snipping tool",
        "screenclippinghost.exe",
        "screen clipping host",
        "screensketch.exe",
        "screen sketch",
        "snipandsketch",
        "snip & sketch",
    ]
    .iter()
    .any(|needle| app_name.contains(needle) || process_path.contains(needle))
}

pub(super) fn read_clipboard_text_once(
    clipboard: &mut Clipboard,
    cache: &mut Option<Option<String>>,
) -> Option<String> {
    if let Some(value) = cache.as_ref() {
        return value.clone();
    }

    let value = clipboard.get_text().ok();
    *cache = Some(value.clone());
    value
}

pub(super) fn read_clipboard_text_fresh() -> Option<String> {
    Clipboard::new()
        .ok()
        .and_then(|mut clipboard| clipboard.get_text().ok())
        .filter(|text| !text.trim().is_empty())
}

pub(super) fn read_clipboard_image_once(
    cache: &mut Option<Option<crate::infrastructure::windows_api::win_clipboard::ImageData>>,
) -> Option<crate::infrastructure::windows_api::win_clipboard::ImageData> {
    if let Some(value) = cache.as_ref() {
        return value.clone();
    }

    let value = unsafe { crate::infrastructure::windows_api::win_clipboard::get_clipboard_image() };
    *cache = Some(value.clone());
    value
}

pub(super) fn is_likely_rich_text_source(
    source_snapshot: &crate::infrastructure::windows_api::window_tracker::ActiveAppInfo,
) -> bool {
    let mut haystack = source_snapshot.app_name.to_ascii_lowercase();
    if let Some(path) = source_snapshot.process_path.as_deref() {
        if !haystack.is_empty() {
            haystack.push(' ');
        }
        haystack.push_str(&path.to_ascii_lowercase());
    }

    [
        "wps",
        "winword",
        "word",
        "excel",
        "powerpoint",
        "onenote",
        "outlook",
        "soffice",
        "libreoffice",
        "writer",
        "calc",
        "impress",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

pub(super) fn is_wps_writer_source(
    source_snapshot: &crate::infrastructure::windows_api::window_tracker::ActiveAppInfo,
) -> bool {
    let app_name = source_snapshot.app_name.to_ascii_lowercase();
    let process_path = source_snapshot
        .process_path
        .as_deref()
        .unwrap_or("")
        .replace('\\', "/")
        .to_ascii_lowercase();
    let executable = process_path.rsplit('/').next().unwrap_or("");

    executable == "wps.exe"
        || ((app_name.contains("wps") || process_path.contains("/wps/"))
            && executable != "et.exe"
            && !app_name.contains("spreadsheet"))
}

pub(super) fn is_likely_spreadsheet_source(
    source_snapshot: &crate::infrastructure::windows_api::window_tracker::ActiveAppInfo,
) -> bool {
    let mut haystack = source_snapshot.app_name.to_ascii_lowercase();
    if let Some(path) = source_snapshot.process_path.as_deref() {
        if !haystack.is_empty() {
            haystack.push(' ');
        }
        haystack.push_str(&path.to_ascii_lowercase());
    }

    ["excel", "et.exe", "wps", "spreadsheet", "calc", "numbers"]
        .iter()
        .any(|needle| haystack.contains(needle))
}

pub(super) fn is_ignored_named_clipboard_format(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return true;
    }

    if lower == "html format" {
        return true;
    }

    [
        "png",
        "image/png",
        "gif",
        "animated gif",
        "image/gif",
        "jfif",
        "jpeg",
        "image/jpeg",
        "bitmap",
        "dib",
        "dibv5",
        "tiff",
        "image/tiff",
        "webp",
        "image/webp",
        "emf",
        "wmf",
        "metafile",
        "object descriptor",
        "link source descriptor",
        "embed source",
        "embedded object",
        "link source",
        "ownerlink",
        "dataobject",
        "ole private data",
        "ole clipboard persist on flush",
        "shell idlist array",
        "preferred dropeffect",
        "performed dropeffect",
        "logical performed dropeffect",
        "filegroupdescriptorw",
        "filegroupdescriptora",
        "filecontents",
        "uniformresourcelocator",
        "uniformresourcelocatorw",
        "inetturl",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(super) fn should_preserve_named_clipboard_format(
    source_snapshot: Option<&crate::infrastructure::windows_api::window_tracker::ActiveAppInfo>,
    format_name: &str,
) -> bool {
    if is_ignored_named_clipboard_format(format_name) {
        return false;
    }

    let lower = format_name.trim().to_ascii_lowercase();
    if lower == "rich text format" {
        return true;
    }

    let Some(source_snapshot) = source_snapshot else {
        return true;
    };

    if !is_likely_spreadsheet_source(source_snapshot) {
        return false;
    }

    true
}

pub fn capture_preserved_named_formats_from_clipboard(
    source_snapshot: Option<&crate::infrastructure::windows_api::window_tracker::ActiveAppInfo>,
) -> Vec<crate::infrastructure::windows_api::win_clipboard::NamedClipboardFormat> {
    let should_retry = source_snapshot
        .map(is_likely_spreadsheet_source)
        .unwrap_or(false);

    for attempt in 0..3 {
        let formats = unsafe {
            crate::infrastructure::windows_api::win_clipboard::get_named_clipboard_formats(
                PRESERVED_NAMED_FORMAT_MAX_COUNT * 2,
                PRESERVED_NAMED_FORMAT_MAX_BYTES,
                PRESERVED_NAMED_FORMAT_TOTAL_BYTES,
            )
        }
        .into_iter()
        .filter(|format| should_preserve_named_clipboard_format(source_snapshot, &format.name))
        .take(PRESERVED_NAMED_FORMAT_MAX_COUNT)
        .collect::<Vec<_>>();

        if !formats.is_empty() || !should_retry || attempt == 2 {
            return formats;
        }

        std::thread::sleep(std::time::Duration::from_millis(35));
    }

    Vec::new()
}

pub(super) fn has_rich_text_candidate_format() -> bool {
    unsafe {
        ["HTML Format", "Rich Text Format"]
            .iter()
            .any(|format_name| {
                crate::infrastructure::windows_api::win_clipboard::get_clipboard_raw_format(
                    format_name,
                )
                .map(|raw| !raw.is_empty())
                .unwrap_or(false)
            })
    }
}

pub(super) fn clipboard_html_animated_image_data_url() -> Option<String> {
    unsafe {
        crate::infrastructure::windows_api::win_clipboard::get_clipboard_raw_format("HTML Format")
    }
    .and_then(|raw| parse_cf_html(&raw))
    .and_then(|html| extract_animated_image_data_url_from_html(&html))
}

pub(super) fn probe_rich_text_payload(
    source_snapshot: &crate::infrastructure::windows_api::window_tracker::ActiveAppInfo,
    initial_text: Option<String>,
) -> Option<(String, String)> {
    let mut latest_text = initial_text.filter(|text| !text.trim().is_empty());
    let mut saw_candidate_format = false;

    for (attempt_idx, delay_ms) in RICH_TEXT_RETRY_DELAYS_MS.iter().enumerate() {
        if attempt_idx > 0 {
            std::thread::sleep(std::time::Duration::from_millis(*delay_ms));
        }

        if let Some(text) = read_clipboard_text_fresh() {
            latest_text = Some(text);
        }

        let parsed_html = unsafe {
            let html_raw =
                crate::infrastructure::windows_api::win_clipboard::get_clipboard_raw_format(
                    "HTML Format",
                );
            if html_raw.is_some() {
                saw_candidate_format = true;
            }
            html_raw
                .and_then(|raw| parse_cf_html(&raw))
                .filter(|html| !html.trim().is_empty())
        };

        if let Some(html) = parsed_html {
            let plain_text = latest_text
                .as_deref()
                .map(|text| derive_rich_text_content(text, Some(&html)))
                .unwrap_or_else(|| derive_rich_text_content("", Some(&html)));
            let normalized_text = if plain_text.trim().is_empty() {
                latest_text
                    .as_deref()
                    .map(normalize_clipboard_plain_text)
                    .unwrap_or_default()
            } else {
                plain_text
            };

            return Some((normalized_text, html));
        }

        if !is_wps_writer_source(source_snapshot) {
            unsafe {
                if crate::infrastructure::windows_api::win_clipboard::get_clipboard_raw_format(
                    "Rich Text Format",
                )
                .map(|raw| !raw.is_empty())
                .unwrap_or(false)
                {
                    saw_candidate_format = true;
                }
            }
        }

        if let Some(text) = latest_text.as_deref() {
            if let Some(html) = infer_rich_html_from_plain_text(
                text,
                &source_snapshot.app_name,
                source_snapshot.process_path.as_deref(),
            ) {
                let plain_text = derive_rich_text_content(text, Some(&html));
                let normalized_text = if plain_text.trim().is_empty() {
                    normalize_clipboard_plain_text(text)
                } else {
                    plain_text
                };
                return Some((normalized_text, html));
            }
        }

        let should_keep_waiting = saw_candidate_format
            || latest_text
                .as_deref()
                .map(looks_like_cf_html_header_text)
                .unwrap_or(false)
            || is_likely_rich_text_source(source_snapshot);

        if !should_keep_waiting {
            break;
        }
    }

    None
}

pub fn clipboard_image_fallback_data_url() -> Option<String> {
    for _ in 0..3 {
        unsafe {
            // 1. Try GIF/animated WebP first to preserve animation
            if let Some(raw) = clipboard_gif_payload() {
                let mime = animated_image_mime(&raw).unwrap_or("image/gif");
                let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
                return Some(format!("data:{};base64,{}", mime, b64));
            }

            // 2. Some sources (e.g. Office apps) may provide PNG/JPEG custom formats.
            // Avoid the expensive decode→re-encode round-trip when possible.
            for name in [
                "PNG",
                "image/png",
                "JFIF",
                "JPEG",
                "image/jpeg",
                "image/webp",
                "WebP",
            ] {
                if let Some(raw) =
                    crate::infrastructure::windows_api::win_clipboard::get_clipboard_raw_format(
                        name,
                    )
                {
                    if let Some(mime) = animated_image_mime(&raw) {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
                        return Some(format!("data:{};base64,{}", mime, b64));
                    }
                    // Fast path: if the raw bytes are already valid PNG/JPEG, skip
                    // image::load_from_memory + re-encode (saves ~200-800ms).
                    if raw.len() > 8 && raw[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
                    {
                        // Valid PNG header — use directly
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
                        return Some(format!("data:image/png;base64,{}", b64));
                    }
                    if raw.len() > 2 && raw[0] == 0xFF && raw[1] == 0xD8 {
                        // Valid JPEG header — use directly
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
                        return Some(format!("data:image/jpeg;base64,{}", b64));
                    }
                    // Fallback: decode and re-encode if format isn't recognized
                    if let Ok(img) = image::load_from_memory(&raw) {
                        let mut bytes: Vec<u8> = Vec::new();
                        let mut cursor = std::io::Cursor::new(&mut bytes);
                        if img.write_to(&mut cursor, image::ImageFormat::Png).is_ok() {
                            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                            return Some(format!("data:image/png;base64,{}", b64));
                        }
                    }
                }
            }

            // 3. Fallback to CF_DIB/CF_DIBV5 decode.
            if let Some(image) =
                crate::infrastructure::windows_api::win_clipboard::get_clipboard_image()
            {
                if let Some(img_buf) =
                    image::RgbaImage::from_raw(image.width as u32, image.height as u32, image.bytes)
                {
                    let mut bytes: Vec<u8> = Vec::new();
                    let mut cursor = std::io::Cursor::new(&mut bytes);
                    if img_buf
                        .write_to(&mut cursor, image::ImageFormat::Png)
                        .is_ok()
                    {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                        return Some(format!("data:image/png;base64,{}", b64));
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(35));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        file_clipboard_data, is_gif_payload, is_snipping_tool_source, is_wps_writer_source,
        resolve_animated_image_fallback, should_capture_file_entries,
        should_prefer_image_over_rich_text, single_gif_file_data_url, ClipboardData,
    };
    use crate::infrastructure::windows_api::window_tracker::ActiveAppInfo;

    #[test]
    fn detects_snipping_tool_by_process_name() {
        let source = ActiveAppInfo {
            app_name: "SnippingTool.exe".to_string(),
            process_path: Some(r"C:\Windows\System32\SnippingTool.exe".to_string()),
        };

        assert!(is_snipping_tool_source(&source));
    }

    #[test]
    fn detects_legacy_screen_sketch_source() {
        let source = ActiveAppInfo {
            app_name: "ApplicationFrameHost.exe".to_string(),
            process_path: Some(
                r"C:\Program Files\WindowsApps\Microsoft.ScreenSketch_2022.2405.32.0_x64__8wekyb3d8bbwe\ScreenSketch.exe"
                    .to_string(),
            ),
        };

        assert!(is_snipping_tool_source(&source));
    }

    #[test]
    fn ignores_normal_apps() {
        let source = ActiveAppInfo {
            app_name: "WINWORD.EXE".to_string(),
            process_path: Some(
                r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE".to_string(),
            ),
        };

        assert!(!is_snipping_tool_source(&source));
    }

    #[test]
    fn file_capture_follows_setting_when_disabled() {
        assert!(!should_capture_file_entries(false));
    }

    #[test]
    fn file_capture_follows_setting_when_enabled() {
        assert!(should_capture_file_entries(true));
    }

    #[test]
    fn gif_payload_requires_a_real_gif_signature() {
        assert!(is_gif_payload(b"GIF89a\x01"));
        assert!(is_gif_payload(b"GIF87a\x01"));
        assert!(!is_gif_payload(b"https://example.test/image.gif"));
        assert!(!is_gif_payload(b"GIF89a"));
    }

    #[test]
    fn prefers_image_when_html_contains_animated_gif() {
        let html = r#"<div><img src="https://cdn.example/a.gif"></div>"#;
        assert!(should_prefer_image_over_rich_text(
            "https://cdn.example/a.gif",
            html,
            false,
            true,
            false,
            || panic!("an existing GIF must not trigger bitmap rendering"),
        ));
        assert!(should_prefer_image_over_rich_text(
            "",
            html,
            false,
            true,
            false,
            || panic!("an existing GIF must not trigger bitmap rendering"),
        ));
    }

    #[test]
    fn keeps_rich_text_for_office_sources_even_with_gif_fallback() {
        for has_gif in [false, true] {
            for text in ["", "caption"] {
                assert!(!should_prefer_image_over_rich_text(
                    text,
                    r#"<img src="a.gif">"#,
                    false,
                    has_gif,
                    true,
                    || panic!("Office/WPS must not be asked to render a bitmap"),
                ));
            }
        }
    }

    #[test]
    fn prefers_image_for_empty_text_raster_copies() {
        let reads = std::cell::Cell::new(0);
        assert!(should_prefer_image_over_rich_text(
            "", "", false, false, false,
            || {
                reads.set(reads.get() + 1);
                true
            },
        ));
        assert_eq!(reads.get(), 1);
        assert!(!should_prefer_image_over_rich_text(
            "hello",
            "<p>hello</p>",
            false,
            false,
            false,
            || panic!("plain text must not require a bitmap probe"),
        ));
    }

    #[test]
    fn keeps_mixed_html_even_when_it_contains_a_gif() {
        for (text, html) in [
            ("正文", r#"<p>正文</p><img src="a.gif">"#),
            ("", r#"<p>正文</p><img src="a.gif">"#),
            ("", r#"<img src="a.gif"><img src="b.png">"#),
            (
                "链接",
                r#"<a href="https://example.test">链接</a><img src="a.gif">"#,
            ),
            ("caption", r#"<img src="a.gif">"#),
            (
                "https://example.test/other",
                r#"<img src="https://cdn.example/a.gif">"#,
            ),
        ] {
            assert!(
                !should_prefer_image_over_rich_text(text, html, false, true, false, || {
                    panic!("mixed content must not require a bitmap probe")
                }),
                "must preserve text={text:?}, html={html}",
            );
        }
    }

    #[test]
    fn standalone_image_allows_escaped_url_metadata() {
        assert!(should_prefer_image_over_rich_text(
            "https://cdn.example/a.gif?x=1&y=2",
            r#"<div><IMG SRC="https://cdn.example/a.gif?x=1&amp;y=2"></div>"#,
            false,
            true,
            false,
            || panic!("GIF already available"),
        ));
    }

    #[test]
    fn animated_image_fallback_reuses_all_frames_without_refetching() {
        use base64::Engine;
        use image::{AnimationDecoder, Frame, Rgba, RgbaImage};

        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                encoder
                    .encode_frame(Frame::new(RgbaImage::from_pixel(2, 2, Rgba(color))))
                    .unwrap();
            }
        }
        let data_url = format!(
            "data:image/gif;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&bytes),
        );
        let html = format!(r#"<img src="{data_url}">"#);
        let recovered = super::extract_animated_image_data_url_from_html(&html).unwrap();
        let resolved = resolve_animated_image_fallback(
            Some(Some(recovered)),
            || panic!("already recovered HTML must not be fetched again"),
            || panic!("already recovered GIF must not probe text"),
        )
        .unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(resolved.split_once(',').unwrap().1)
            .unwrap();
        assert_eq!(decoded, bytes);
        let frames = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(decoded))
            .unwrap()
            .into_frames()
            .collect_frames()
            .unwrap();
        assert_eq!(frames.len(), 2);
        assert_ne!(frames[0].buffer(), frames[1].buffer());
    }

    #[test]
    fn animated_image_fallback_does_not_repeat_a_failed_html_probe() {
        assert_eq!(
            resolve_animated_image_fallback(
                Some(None),
                || panic!("a failed HTML probe is cached too"),
                || Some("text fallback".to_string()),
            )
            .as_deref(),
            Some("text fallback"),
        );
    }

    #[test]
    fn animated_image_fallback_probes_html_only_when_not_cached() {
        let html_reads = std::cell::Cell::new(0);
        let text_reads = std::cell::Cell::new(0);
        let resolved = resolve_animated_image_fallback(
            None,
            || {
                html_reads.set(html_reads.get() + 1);
                None
            },
            || {
                text_reads.set(text_reads.get() + 1);
                None
            },
        );
        assert!(resolved.is_none());
        assert_eq!(html_reads.get(), 1);
        assert_eq!(text_reads.get(), 1);
    }

    #[test]
    fn single_gif_file_data_url_reads_gif_bytes() {
        let dir = std::env::temp_dir().join(format!(
            "tiez_gif_hdrop_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.gif");
        // Minimal GIF89a header + one extra byte so the signature check passes.
        let gif_bytes = b"GIF89a\x01";
        std::fs::write(&path, gif_bytes).unwrap();

        let data_url = single_gif_file_data_url(&[path.to_string_lossy().into_owned()]).unwrap();
        assert!(data_url.starts_with("data:image/gif;base64,"));
        assert!(single_gif_file_data_url(&[path.to_string_lossy().into_owned(), "x".into()]).is_none());
        assert!(single_gif_file_data_url(&[dir.join("nope.png").to_string_lossy().into_owned()]).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn qq_mislabeled_gif_preserves_all_frames_with_file_capture_on_and_off() {
        use base64::Engine;
        use image::{AnimationDecoder, Frame, Rgba, RgbaImage};

        let dir = std::env::temp_dir().join(format!(
            "tiez_qq_gif_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut gif_bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif_bytes);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Infinite)
                .unwrap();
            for color in [
                [255, 0, 0, 255],
                [0, 255, 0, 255],
                [0, 0, 255, 255],
                [255, 255, 0, 255],
            ] {
                encoder
                    .encode_frame(Frame::new(RgbaImage::from_pixel(2, 2, Rgba(color))))
                    .unwrap();
            }
        }

        for name in ["qq-cache.jpg", "qq-cache.png", "sample.GIF", "no-extension"] {
            let source = dir.join(name);
            std::fs::write(&source, &gif_bytes).unwrap();
            for capture_files in [false, true] {
                let data = file_clipboard_data(
                    vec![source.to_string_lossy().into_owned()],
                    capture_files,
                )
                .unwrap();
                let ClipboardData::Image { data_url } = data else {
                    panic!("GIF must bypass still-image conversion: {name}, {capture_files}");
                };
                assert!(data_url.starts_with("data:image/gif;base64,"));
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(data_url.split_once(',').unwrap().1)
                    .unwrap();
                assert_eq!(decoded, gif_bytes);
                let frames = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(decoded))
                    .unwrap()
                    .into_frames()
                    .collect_frames()
                    .unwrap();
                assert_eq!(frames.len(), 4);
                assert_ne!(frames[0].buffer(), frames[1].buffer());

                let stored = crate::database::save_image_to_file(&data_url, &dir).unwrap();
                assert_eq!(std::path::Path::new(&stored).extension().unwrap(), "gif");
                assert_eq!(std::fs::read(stored).unwrap(), gif_bytes);
            }
        }

        // Ordinary files, including files merely named .gif, must still obey
        // capture-files. A multi-file selection must not lose its other files.
        let png = dir.join("still.png");
        RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255]))
            .save(&png)
            .unwrap();
        let fake_gif = dir.join("fake.gif");
        std::fs::copy(&png, &fake_gif).unwrap();
        let short = dir.join("short.gif");
        std::fs::write(&short, b"GIF89a").unwrap();
        for files in [
            vec![png.to_string_lossy().into_owned()],
            vec![fake_gif.to_string_lossy().into_owned()],
            vec![short.to_string_lossy().into_owned()],
            vec![dir.join("missing.gif").to_string_lossy().into_owned()],
            vec![dir.to_string_lossy().into_owned()],
            vec![
                dir.join("qq-cache.jpg").to_string_lossy().into_owned(),
                png.to_string_lossy().into_owned(),
            ],
        ] {
            assert!(file_clipboard_data(files.clone(), false).is_none());
            let Some(ClipboardData::Files(preserved)) = file_clipboard_data(files.clone(), true)
            else {
                panic!("ordinary files must retain the existing file pipeline");
            };
            assert_eq!(preserved, files);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn decode_hex(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    fn two_frame_animated_webp() -> Vec<u8> {
        decode_hex("52494646ca00000057454250565038580a00000002000000010000010000414e494d06000000000000000000414e4d464a0000000000000000000100000100005000000256503820320000003001009d012a0200020001402625a000037000fef2eb7ffff9b03ff6f3ff047a01ffffd2e0fffe9707fff4b83ff4a4000000414e4d464c0000000000000000000100000100005000000056503820340000003401009d012a0200020000002625a000037000fee9221ffff79f3fffb9f3fffb9f3fe8cfffff29fbfff238ffff238ffe50200000")
    }

    fn static_webp() -> Vec<u8> {
        decode_hex("524946463a00000057454250565038202e0000009001009d012a0200020001402625a00274ba00039800fefb55e3ffa5c1ffd2e0ffe9707fe9707f1bb2ce1ba40000")
    }

    #[test]
    fn mislabeled_animated_webp_preserves_bytes_with_file_capture_on_and_off() {
        use base64::Engine;

        let dir = std::env::temp_dir().join(format!(
            "tiez_webp_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let webp_bytes = two_frame_animated_webp();
        assert_eq!(webp_bytes.windows(4).filter(|w| *w == b"ANMF").count(), 2);

        for name in ["sticker.webp", "cache.jpg", "cache.png", "no-extension"] {
            let source = dir.join(name);
            std::fs::write(&source, &webp_bytes).unwrap();
            for capture_files in [false, true] {
                let data = file_clipboard_data(
                    vec![source.to_string_lossy().into_owned()],
                    capture_files,
                )
                .unwrap();
                let ClipboardData::Image { data_url } = data else {
                    panic!("animated WebP must bypass still-image conversion: {name}, {capture_files}");
                };
                assert!(data_url.starts_with("data:image/webp;base64,"));
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(data_url.split_once(',').unwrap().1)
                    .unwrap();
                assert_eq!(decoded, webp_bytes);

                let stored = crate::database::save_image_to_file(&data_url, &dir).unwrap();
                assert_eq!(std::path::Path::new(&stored).extension().unwrap(), "webp");
                assert_eq!(std::fs::read(stored).unwrap(), webp_bytes);
            }
        }

        let still = dir.join("still.webp");
        std::fs::write(&still, static_webp()).unwrap();
        for files in [
            vec![still.to_string_lossy().into_owned()],
            vec![
                dir.join("sticker.webp").to_string_lossy().into_owned(),
                still.to_string_lossy().into_owned(),
            ],
        ] {
            assert!(file_clipboard_data(files.clone(), false).is_none());
            let Some(ClipboardData::Files(preserved)) = file_clipboard_data(files.clone(), true)
            else {
                panic!("static WebP must retain the existing file pipeline");
            };
            assert_eq!(preserved, files);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_wps_writer_without_treating_spreadsheet_as_writer() {
        let writer = ActiveAppInfo {
            app_name: "WPS Office".to_string(),
            process_path: Some(r"C:\Program Files\Kingsoft\WPS Office\wps.exe".to_string()),
        };
        let spreadsheet = ActiveAppInfo {
            app_name: "WPS Spreadsheets".to_string(),
            process_path: Some(r"C:\Program Files\Kingsoft\WPS Office\et.exe".to_string()),
        };

        assert!(is_wps_writer_source(&writer));
        assert!(!is_wps_writer_source(&spreadsheet));
    }
}
    #[test]
    fn file_only_clipboards_have_distinct_preflight_hashes() {
        let first = vec![r"C:\files\a.txt".to_string()];
        let second = vec![r"C:\files\b.txt".to_string()];
        let first_hash = clipboard_preflight_hash(None, None, Some(&first));
        assert!(first_hash.is_some());
        assert_ne!(first_hash, clipboard_preflight_hash(None, None, Some(&second)));
        assert_eq!(first_hash, clipboard_preflight_hash(None, None, Some(&first)));
        assert_eq!(clipboard_preflight_hash(None, None, None), None);
        assert_eq!(clipboard_preflight_hash(None, None, Some(&[])), None);
    }
