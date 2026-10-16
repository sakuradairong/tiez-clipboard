//! Real Windows clipboard reads using the production format probes.
use crate::capture_helpers::*;
use crate::data::ClipboardData;
use crate::utils::*;
use arboard::Clipboard;
use base64::Engine;
use std::sync::atomic::{AtomicBool, Ordering};
use tiez_native_platform::{clipboard_formats as formats, source_app::ActiveAppInfo};

#[derive(Clone, Copy, Debug, Default)]
pub struct ReadOptions {
    pub files: bool,
    pub rich_text: bool,
}
#[derive(Debug, Clone)]
pub struct CapturedSnapshot {
    pub sequence: u32,
    pub source: ActiveAppInfo,
    pub data: ClipboardData,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadError {
    Cancelled,
    Changed,
}

pub fn sequence() -> u32 {
    formats::get_clipboard_sequence_number()
}
pub fn source() -> ActiveAppInfo {
    tiez_native_platform::source_app::get_clipboard_source_app_info()
}
pub fn settle_delay(source: &ActiveAppInfo) -> std::time::Duration {
    std::time::Duration::from_millis(if is_snipping_tool_source(source) {
        SNIPPING_TOOL_SETTLE_DELAY_MS
    } else {
        DEFAULT_CLIPBOARD_SETTLE_DELAY_MS
    })
}
fn checkpoint(expected: u32, stop: &AtomicBool) -> Result<(), ReadError> {
    if stop.load(Ordering::Acquire) {
        Err(ReadError::Cancelled)
    } else if sequence() != expected {
        Err(ReadError::Changed)
    } else {
        Ok(())
    }
}

/// Reads off the message/UI threads. Original payloads are retained. A sequence
/// change rejects the whole result rather than mixing formats from two copies.
/// Call only with an explicit core resource scope; do not use this on startup to
/// import the user's existing clipboard. Delayed rendering probes are synchronous.
pub fn read(
    expected: u32,
    source: ActiveAppInfo,
    options: ReadOptions,
    stop: &AtomicBool,
) -> Result<Option<CapturedSnapshot>, ReadError> {
    checkpoint(expected, stop)?;
    let data = read_data(&source, options, stop);
    checkpoint(expected, stop)?;
    Ok(data.map(|data| CapturedSnapshot {
        sequence: expected,
        source,
        data,
    }))
}

fn read_data(
    source: &ActiveAppInfo,
    options: ReadOptions,
    stop: &AtomicBool,
) -> Option<ClipboardData> {
    if let Some(files) = unsafe { formats::get_clipboard_files() }.filter(|files| !files.is_empty())
    {
        // File capture off still preserves actual GIF/WebP animation payloads.
        return file_clipboard_data(files, options.files);
    }
    if stop.load(Ordering::Acquire) {
        return None;
    }
    if let Some(raw) = clipboard_gif_payload() {
        let mime = animated_image_mime(&raw).unwrap_or("image/gif");
        return Some(ClipboardData::Image {
            data_url: format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(raw)
            ),
        });
    }
    let mut clipboard = Clipboard::new().ok()?;
    let mut cached_text = None;
    let text = read_clipboard_text_once(&mut clipboard, &mut cached_text)
        .filter(|text| !text.trim().is_empty());
    let rich_source = is_likely_rich_text_source(source);
    let pure_image = text.is_none() && !rich_source;
    if options.rich_text
        && !pure_image
        && (text.is_some() || rich_source || has_rich_text_candidate_format())
    {
        if let Some((text, html)) = probe_rich_text_payload(source, text.clone()) {
            if stop.load(Ordering::Acquire) {
                return None;
            }
            let animated = extract_animated_image_data_url_from_html(&html);
            let mut cached_image = None;
            let prefer_image = should_prefer_image_over_rich_text(
                &text,
                &html,
                false,
                animated.is_some(),
                rich_source,
                || read_clipboard_image_once(&mut cached_image).is_some(),
            );
            if prefer_image {
                if let Some(data_url) = animated.or_else(clipboard_image_fallback_data_url) {
                    return Some(ClipboardData::Image { data_url });
                }
            } else {
                let wps = is_wps_writer_source(source);
                let has_images = html.to_ascii_lowercase().contains("<img ");
                let mut html = if let Some(image) = animated.or_else(|| {
                    if has_images || wps {
                        None
                    } else {
                        clipboard_image_fallback_data_url()
                    }
                }) {
                    attach_rich_image_fallback(&html, &image)
                } else {
                    html
                };
                if !wps {
                    let named = capture_preserved_named_formats_from_clipboard(Some(source));
                    if !named.is_empty() {
                        html = attach_rich_named_formats(&html, &named);
                    }
                }
                return Some(ClipboardData::RichText { text, html });
            }
        }
    }
    if stop.load(Ordering::Acquire) {
        return None;
    }
    let animated =
        resolve_animated_image_fallback(None, clipboard_html_animated_image_data_url, || {
            text.as_deref()
                .and_then(extract_animated_image_data_url_from_text)
        });
    if let Some(data_url) = animated.or_else(clipboard_image_fallback_data_url) {
        return Some(ClipboardData::Image { data_url });
    }
    text.map(ClipboardData::Text)
}

/// App-origin writes share the same echo markers as the production probes.
/// The single native core clears these on open/close; no view owns this state.
pub fn clear_echo() {
    clear_recent_image_echo_state();
    crate::LAST_APP_SET_TIMESTAMP.store(0, Ordering::SeqCst);
}

/// Debounce exact capture bursts without conflating different rich formats.
pub fn fingerprint(data: &ClipboardData) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    match data {
        ClipboardData::Text(text) => {
            0u8.hash(&mut hash);
            clipboard_preflight_hash(Some(text), None, None).hash(&mut hash);
        }
        ClipboardData::RichText { text, html } => {
            1u8.hash(&mut hash);
            clipboard_preflight_hash(Some(text), None, None).hash(&mut hash);
            repair_html_fragment(html).hash(&mut hash);
        }
        ClipboardData::Image { data_url } => {
            2u8.hash(&mut hash);
            clipboard_preflight_hash(None, Some(data_url.as_bytes()), None).hash(&mut hash);
        }
        ClipboardData::Files(files) => {
            3u8.hash(&mut hash);
            clipboard_preflight_hash(None, None, Some(files)).hash(&mut hash);
        }
    }
    hash.finish()
}
pub fn matches_image_echo(raw_hash: u64, visual_hash: u64) -> bool {
    should_ignore_recent_image_echo(raw_hash, visual_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelled_or_changed_read_never_opens_clipboard_payloads() {
        let stop = AtomicBool::new(true);
        assert_eq!(
            read(0, ActiveAppInfo::default(), ReadOptions::default(), &stop).unwrap_err(),
            ReadError::Cancelled
        );
        stop.store(false, Ordering::Release);
        let wrong_sequence = sequence().wrapping_add(1);
        assert_eq!(
            read(
                wrong_sequence,
                ActiveAppInfo::default(),
                ReadOptions::default(),
                &stop
            )
            .unwrap_err(),
            ReadError::Changed
        );
    }
    #[test]
    fn burst_fingerprint_preserves_rich_format_and_file_identity() {
        let rich = ClipboardData::RichText {
            text: " x ".into(),
            html: "<b>x</b>".into(),
        };
        let other = ClipboardData::RichText {
            text: " x ".into(),
            html: "<i>x</i>".into(),
        };
        assert_ne!(fingerprint(&rich), fingerprint(&other));
        assert_ne!(
            fingerprint(&rich),
            fingerprint(&ClipboardData::Text(" x ".into()))
        );
        assert_ne!(
            fingerprint(&ClipboardData::Files(vec!["a.txt".into()])),
            fingerprint(&ClipboardData::Files(vec!["b.txt".into()]))
        );
    }
}
