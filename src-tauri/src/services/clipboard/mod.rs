mod capture_helpers;
mod data;
mod processing;
mod image_roots;
mod pipeline;
mod utils;

use crate::app_state::SettingsState;
pub use crate::database::DbState;
use crate::database::{
    calc_image_hash, calc_image_hash_from_bytes, calc_image_hash_from_rgba, calc_text_hash,
};
use arboard::Clipboard;
use base64::Engine;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

use utils::*;
use capture_helpers::*;
pub use capture_helpers::{capture_preserved_named_formats_from_clipboard, clipboard_image_fallback_data_url};

pub(crate) use utils::is_animated_webp_payload;

#[cfg(target_os = "windows")]
pub fn start_clipboard_monitor(app_handle: AppHandle) {
    use std::sync::{Arc, Mutex};

    // Initial state for deduplication and self-copy detection
    let mut last_text = String::new();
    let last_seq =
        crate::infrastructure::windows_api::win_clipboard::get_clipboard_sequence_number();
    let mut last_image_hash = 0u64;

    // We can initialize these with current content to avoid capturing on startup
    if let Ok(mut cb) = Clipboard::new() {
        last_text = normalize_clipboard_plain_text(&cb.get_text().unwrap_or_default());
        last_image_hash = unsafe {
            if let Some(image) =
                crate::infrastructure::windows_api::win_clipboard::get_clipboard_image()
            {
                let mut hash = image.bytes.len() as u64;
                if !image.bytes.is_empty() {
                    hash = hash
                        .wrapping_add(image.bytes[0] as u64)
                        .wrapping_add(image.bytes[image.bytes.len() / 2] as u64)
                        .wrapping_add(image.bytes[image.bytes.len() - 1] as u64);
                }
                hash
            } else {
                0u64
            }
        };
    }

    struct MonitorState {
        last_text: String,
        last_seq: u32,
        last_image_hash: u64,
        last_content_hash: Option<u64>,
        last_process_time: u64,
    }

    let state = Arc::new(Mutex::new(MonitorState {
        last_text,
        last_seq,
        last_image_hash,
        last_content_hash: None,
        last_process_time: 0,
    }));

    let app_clone = app_handle.clone();
    let state_lock = state.clone();

    // Start the native Windows listener
    crate::services::clipboard_listener::listen_clipboard(Arc::new(move || {
        let app = app_clone.clone();
        let mut monitor_state = state_lock.lock().unwrap();

        // 1. Check for pause
        if crate::CLIPBOARD_MONITOR_PAUSED.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }

        // 2. Sequence check (De-bounce Windows firing multiple events for one copy)
        let current_seq =
            crate::infrastructure::windows_api::win_clipboard::get_clipboard_sequence_number();
        if current_seq == monitor_state.last_seq {
            return;
        }
        monitor_state.last_seq = current_seq;
        let source_snapshot =
            crate::infrastructure::windows_api::window_tracker::get_clipboard_source_app_info();

        // Give source apps time to finish writing clipboard payloads before we start
        // probing formats. Snipping Tool needs a longer quiet period or its save
        // pipeline may race with clipboard-manager reads.
        let settle_delay_ms = if is_snipping_tool_source(&source_snapshot) {
            SNIPPING_TOOL_SETTLE_DELAY_MS
        } else {
            DEFAULT_CLIPBOARD_SETTLE_DELAY_MS
        };
        std::thread::sleep(std::time::Duration::from_millis(settle_delay_ms));

        // Initialize clipboard for this thread
        let mut clipboard = match Clipboard::new() {
            Ok(cb) => cb,
            Err(_) => return,
        };

        let mut cached_text: Option<Option<String>> = None;
        let mut cached_image: Option<
            Option<crate::infrastructure::windows_api::win_clipboard::ImageData>,
        > = None;
        let mut cached_html_animated_image: Option<Option<String>> = None;

        // 3. Content-based deduplication with time window (for Chrome address bar, etc.)
        // Some apps trigger multiple clipboard updates with different sequence numbers
        // but identical content within a short time window
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Include CF_HDROP before deduplication and reuse the same file snapshot.
        let clipboard_files = unsafe {
            crate::infrastructure::windows_api::win_clipboard::get_clipboard_files()
        };
        let preflight_text = read_clipboard_text_once(&mut clipboard, &mut cached_text);
        // Keep bitmap rendering lazy for Office/WPS delayed-rendering providers.
        let preflight_image = if is_likely_rich_text_source(&source_snapshot) {
            None
        } else {
            read_clipboard_image_once(&mut cached_image)
        };
        let current_content_hash = clipboard_preflight_hash(
            preflight_text.as_deref(),
            preflight_image.as_ref().map(|image| image.bytes.as_slice()),
            clipboard_files.as_deref(),
        );

        // If content is identical to last processed content within 2000ms window, skip.
        // Rich text sources (Office/WPS) may fire multiple clipboard events over >500ms
        // because they write formats sequentially and probe_rich_text_payload retries.
        if current_content_hash == monitor_state.last_content_hash
            && current_content_hash.is_some()
            && now.saturating_sub(monitor_state.last_process_time) < 2000
        {
            return;
        }

        monitor_state.last_content_hash = current_content_hash;
        monitor_state.last_process_time = now;

        let mut handled = false;

        // --- Core processing logic (same as before) ---

        // 1. Check Files
        if let Some(files) = clipboard_files {
            let content = files.join("\n");
            if !content.is_empty() {
                let is_new = content != monitor_state.last_text;
                let mut should_process = is_new;
                if !is_new {
                    if let Some(db_state) = app.try_state::<DbState>() {
                        if let Ok(conn) = db_state.conn.lock() {
                            if let Ok(None) = db_state
                                .repo
                                .find_by_content_with_conn(&conn, &content, None)
                            {
                                should_process = true;
                            }
                        }
                    }
                }

                if should_process {
                    let normalized = content.trim().replace("\r\n", "\n");
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    use std::hash::{Hash, Hasher};
                    normalized.hash(&mut hasher);
                    let current_hash = hasher.finish();

                    let last_app_hash = crate::LAST_APP_SET_HASH.load(Ordering::SeqCst);
                    let last_app_hash_alt = crate::LAST_APP_SET_HASH_ALT.load(Ordering::SeqCst);
                    let last_app_time = crate::LAST_APP_SET_TIMESTAMP.load(Ordering::SeqCst);
                    let now_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    if (last_app_hash != 0
                        && (last_app_hash == current_hash || last_app_hash_alt == current_hash))
                        && (now_secs - last_app_time) < 10
                    {
                        crate::LAST_APP_SET_HASH.store(0, Ordering::SeqCst);
                        crate::LAST_APP_SET_HASH_ALT.store(0, Ordering::SeqCst);
                    } else {
                        crate::LAST_APP_SET_HASH.store(0, Ordering::SeqCst);
                        crate::LAST_APP_SET_HASH_ALT.store(0, Ordering::SeqCst);
                        monitor_state.last_text = content.clone();

                        let settings = app.state::<SettingsState>();
                        if let Some(data) = file_clipboard_data(
                            files,
                            settings.capture_files.load(Ordering::Relaxed),
                        ) {
                            process_new_entry(
                                &app,
                                data,
                                None,
                                Some(source_snapshot.clone()),
                            );
                        }
                    }
                }
                handled = true;
            }
        }

        let clipboard_gif_data = if handled {
            None
        } else {
            clipboard_gif_payload()
        };

        if !handled {
            let settings = app.state::<SettingsState>();
            let rich_text_enabled = settings.capture_rich_text.load(Ordering::Relaxed);
            let initial_text = read_clipboard_text_once(&mut clipboard, &mut cached_text)
                .filter(|text| !text.trim().is_empty());

            // Fast-path: if the clipboard has a GIF format, skip the expensive
            // rich text probing entirely and let the image handler (section 3)
            // process it directly.  Previously GIFs went through rich text
            // probing → clipboard_image_fallback_data_url → only to be discarded
            // by `prefer_image`, wasting 0.5–2 s.
            let clipboard_has_gif = clipboard_gif_data.is_some();

            // When there's no text and the source isn't a dedicated rich text
            // application, this is almost certainly a pure image copy (e.g.
            // right-click → Copy image in a browser).  Rich text probing would
            // just be discarded by `prefer_image = true` later, so skip it
            // entirely to avoid 100–1400 ms of wasted retries.
            let pure_image_copy =
                initial_text.is_none() && !is_likely_rich_text_source(&source_snapshot);

            let should_probe_rich_text = rich_text_enabled
                && !clipboard_has_gif
                && !pure_image_copy
                && (initial_text.is_some()
                    || is_likely_rich_text_source(&source_snapshot)
                    || has_rich_text_candidate_format());

            if should_probe_rich_text {
                if let Some((text, html)) =
                    probe_rich_text_payload(&source_snapshot, initial_text.clone())
                {
                    let normalized_text = normalize_clipboard_plain_text(&text);
                    let current_hash = calc_text_hash(&normalized_text);
                    let current_html_hash =
                        calc_text_hash(&crate::services::clipboard::repair_html_fragment(&html));

                    let last_app_hash = crate::LAST_APP_SET_HASH.load(Ordering::SeqCst);
                    let last_app_hash_alt = crate::LAST_APP_SET_HASH_ALT.load(Ordering::SeqCst);
                    let last_app_time = crate::LAST_APP_SET_TIMESTAMP.load(Ordering::SeqCst);
                    let now_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    if last_app_hash != 0
                        && ((current_hash == last_app_hash || current_hash == last_app_hash_alt)
                            || (current_html_hash == last_app_hash
                                || current_html_hash == last_app_hash_alt))
                        && (now_secs - last_app_time) < 10
                    {
                        crate::LAST_APP_SET_HASH.store(0, Ordering::SeqCst);
                        crate::LAST_APP_SET_HASH_ALT.store(0, Ordering::SeqCst);
                        monitor_state.last_text = normalized_text;
                        handled = true;
                    } else {
                        let html_animated_gif_fallback =
                            extract_animated_image_data_url_from_html(&html);
                        let prefer_image = should_prefer_image_over_rich_text(
                            &text,
                            &html,
                            clipboard_gif_data.is_some(),
                            html_animated_gif_fallback.is_some(),
                            is_likely_rich_text_source(&source_snapshot),
                            || read_clipboard_image_once(&mut cached_image).is_some(),
                        );
                        let mut html_to_store = html;

                        // When the HTML already has <img> tags with local/renderable
                        // sources, skip the very expensive clipboard bitmap capture
                        // (clipboard_image_fallback_data_url reads CF_DIB, does PNG
                        // encoding + base64, typically 500–2000 ms).  HtmlContent
                        // renders those images directly via Tauri asset paths.
                        let html_has_renderable_images =
                            html_to_store.to_ascii_lowercase().contains("<img ");

                        let conservative_wps_capture = is_wps_writer_source(&source_snapshot);
                        if let Some(data_url) = html_animated_gif_fallback.clone().or_else(|| {
                            if html_has_renderable_images || conservative_wps_capture {
                                None
                            } else {
                                clipboard_image_fallback_data_url()
                            }
                        }) {
                            html_to_store = attach_rich_image_fallback(&html_to_store, &data_url);
                        }

                        let preserved_named_formats = if conservative_wps_capture {
                            Vec::new()
                        } else {
                            capture_preserved_named_formats_from_clipboard(Some(&source_snapshot))
                        };
                        if !preserved_named_formats.is_empty() {
                            html_to_store =
                                attach_rich_named_formats(&html_to_store, &preserved_named_formats);
                        }

                        cached_html_animated_image = Some(html_animated_gif_fallback);

                        if !prefer_image {
                            monitor_state.last_text = normalized_text.clone();
                            process_new_entry(
                                &app,
                                ClipboardData::RichText {
                                    text: normalized_text,
                                    html: html_to_store,
                                },
                                None,
                                Some(source_snapshot.clone()),
                            );
                            handled = true;

                            // Update content hash after rich text processing so that
                            // a subsequent clipboard event with the same raw clipboard
                            // content is correctly deduped within the time window.
                            // We reuse `current_content_hash` (computed from the raw
                            // clipboard text + image at the top of the handler) to
                            // ensure it matches the next event's initial hash.
                            monitor_state.last_content_hash = current_content_hash;
                            monitor_state.last_process_time = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis()
                                as u64;
                        }
                    }
                }
            }
        }

        // 3. Check Image
        if !handled {
            unsafe {
                let gif_data_opt = clipboard_gif_data;

                let animated_image_fallback = if gif_data_opt.is_none() {
                    resolve_animated_image_fallback(
                        cached_html_animated_image,
                        clipboard_html_animated_image_data_url,
                        || {
                            read_clipboard_text_fresh()
                                .and_then(|text| extract_animated_image_data_url_from_text(&text))
                        },
                    )
                } else {
                    None
                };

                if let Some(gif_data) = gif_data_opt {
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    use std::hash::{Hash, Hasher};
                    gif_data.hash(&mut hasher);
                    let hash = hasher.finish();
                    let visual_hash =
                        calc_image_hash_from_bytes(&gif_data).unwrap_or(hash as i64) as u64;
                    handled = true;

                    if hash != monitor_state.last_image_hash {
                        if should_ignore_recent_image_echo(hash, visual_hash) {
                            clear_recent_image_echo_state();
                        } else {
                            let mime = animated_image_mime(&gif_data).unwrap_or("image/gif");
                            let b64 = base64::engine::general_purpose::STANDARD.encode(gif_data);
                            process_new_entry(
                                &app,
                                ClipboardData::Image {
                                    data_url: format!("data:{};base64,{}", mime, b64),
                                },
                                None,
                                Some(source_snapshot.clone()),
                            );
                            monitor_state.last_text = String::new();
                        }
                        monitor_state.last_image_hash = hash;
                    }
                } else if let Some(data_url) = animated_image_fallback {
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    use std::hash::{Hash, Hasher};
                    data_url.hash(&mut hasher);
                    let hash = hasher.finish();
                    let visual_hash = calc_image_hash(&data_url).unwrap_or(hash as i64) as u64;
                    handled = true;

                    if hash != monitor_state.last_image_hash {
                        if should_ignore_recent_image_echo(hash, visual_hash) {
                            clear_recent_image_echo_state();
                        } else {
                            process_new_entry(
                                &app,
                                ClipboardData::Image { data_url },
                                None,
                                Some(source_snapshot.clone()),
                            );
                            monitor_state.last_text = String::new();
                        }
                        monitor_state.last_image_hash = hash;
                    }
                }

                if !handled {
                    // Fast path: try native PNG/JPEG clipboard formats first.
                    // Browsers often provide these alongside CF_DIB, and using
                    // them directly avoids the expensive bitmap → PNG encode.
                    let fast_data_url: Option<(String, u64, u64)> = {
                        use std::hash::{Hash, Hasher};
                        let mut result = None;
                        for name in ["PNG", "image/png", "JFIF", "JPEG", "image/jpeg"] {
                            if let Some(raw) =
                                crate::infrastructure::windows_api::win_clipboard::get_clipboard_raw_format(name)
                            {
                                if raw.len() > 8 && raw[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
                                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                                    raw.hash(&mut hasher);
                                    let hash = hasher.finish();
                                    let visual_hash =
                                        calc_image_hash_from_bytes(&raw).unwrap_or(hash as i64) as u64;
                                    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
                                    result = Some((
                                        format!("data:image/png;base64,{}", b64),
                                        hash,
                                        visual_hash,
                                    ));
                                    break;
                                }
                                if raw.len() > 2 && raw[0] == 0xFF && raw[1] == 0xD8 {
                                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                                    raw.hash(&mut hasher);
                                    let hash = hasher.finish();
                                    let visual_hash =
                                        calc_image_hash_from_bytes(&raw).unwrap_or(hash as i64) as u64;
                                    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
                                    result = Some((
                                        format!("data:image/jpeg;base64,{}", b64),
                                        hash,
                                        visual_hash,
                                    ));
                                    break;
                                }
                            }
                        }
                        result
                    };

                    if let Some((data_url, hash, visual_hash)) = fast_data_url {
                        if hash != monitor_state.last_image_hash {
                            if should_ignore_recent_image_echo(hash, visual_hash) {
                                clear_recent_image_echo_state();
                                handled = true;
                            } else {
                                process_new_entry(
                                    &app,
                                    ClipboardData::Image { data_url },
                                    None,
                                    Some(source_snapshot.clone()),
                                );
                                handled = true;
                            }
                            monitor_state.last_image_hash = hash;
                        } else {
                            handled = true;
                        }
                    }

                    // Slow fallback: read CF_DIB bitmap and encode to PNG
                    if !handled {
                        if let Some(image) = read_clipboard_image_once(&mut cached_image) {
                            use std::hash::{Hash, Hasher};
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            image.bytes.hash(&mut hasher);
                            let hash = hasher.finish();
                            let visual_hash = calc_image_hash_from_rgba(
                                image.width as u32,
                                image.height as u32,
                                &image.bytes,
                            )
                            .unwrap_or(hash as i64)
                                as u64;

                            if hash != monitor_state.last_image_hash {
                                if should_ignore_recent_image_echo(hash, visual_hash) {
                                    clear_recent_image_echo_state();
                                    handled = true;
                                } else {
                                    if let Some(img_buf) = image::RgbaImage::from_raw(
                                        image.width as u32,
                                        image.height as u32,
                                        image.bytes,
                                    ) {
                                        let mut bytes: Vec<u8> = Vec::new();
                                        let mut cursor = std::io::Cursor::new(&mut bytes);
                                        if img_buf
                                            .write_to(&mut cursor, image::ImageFormat::Png)
                                            .is_ok()
                                        {
                                            let b64 = base64::engine::general_purpose::STANDARD
                                                .encode(bytes);
                                            process_new_entry(
                                                &app,
                                                ClipboardData::Image {
                                                    data_url: format!(
                                                        "data:image/png;base64,{}",
                                                        b64
                                                    ),
                                                },
                                                None,
                                                Some(source_snapshot.clone()),
                                            );
                                            handled = true;
                                        }
                                    }
                                }
                                monitor_state.last_image_hash = hash;
                            }
                        }
                    }
                }
            }
        }

        // 4. Check Text
        if !handled {
            if let Some(text) = read_clipboard_text_once(&mut clipboard, &mut cached_text) {
                let normalized_text = normalize_clipboard_plain_text(&text);
                if !normalized_text.trim().is_empty() {
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    use std::hash::{Hash, Hasher};
                    normalized_text.hash(&mut hasher);
                    let current_hash = hasher.finish();

                    let last_app_hash = crate::LAST_APP_SET_HASH.load(Ordering::SeqCst);
                    let last_app_time = crate::LAST_APP_SET_TIMESTAMP.load(Ordering::SeqCst);
                    let now_secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    if (last_app_hash != 0
                        && (current_hash == last_app_hash
                            || current_hash == crate::LAST_APP_SET_HASH_ALT.load(Ordering::SeqCst)))
                        && (now_secs - last_app_time) < 10
                    {
                        crate::LAST_APP_SET_HASH.store(0, Ordering::SeqCst);
                        crate::LAST_APP_SET_HASH_ALT.store(0, Ordering::SeqCst);
                        monitor_state.last_text = normalized_text.clone();
                        return;
                    }

                    if last_app_hash != 0 {
                        crate::LAST_APP_SET_HASH.store(0, Ordering::SeqCst);
                    }
                    monitor_state.last_text = normalized_text.clone();

                    // Update content hash so subsequent events with the same
                    // text content are deduped within the time window.
                    monitor_state.last_content_hash = current_content_hash;
                    monitor_state.last_process_time = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    process_new_entry(
                        &app,
                        ClipboardData::Text(normalized_text),
                        None,
                        Some(source_snapshot.clone()),
                    );
                }
            }
        }
    }));
}

#[cfg(not(target_os = "windows"))]
pub fn start_clipboard_monitor(app_handle: AppHandle) {
    use std::hash::{Hash, Hasher};
    use std::sync::{Arc, Mutex};

    struct MonitorState {
        last_content_hash: u64,
        last_process_time: u64,
    }

    let state = Arc::new(Mutex::new(MonitorState {
        last_content_hash: 0,
        last_process_time: 0,
    }));
    let app_clone = app_handle.clone();
    let state_lock = state.clone();

    crate::services::clipboard_listener::listen_clipboard(Arc::new(move || {
        if crate::CLIPBOARD_MONITOR_PAUSED.load(Ordering::Relaxed) {
            return;
        }

        // Clipboard owners may publish formats in multiple steps. A short delay
        // avoids taking a partial snapshot without adding noticeable latency.
        std::thread::sleep(std::time::Duration::from_millis(40));

        let app = app_clone.clone();
        let mut clipboard = match Clipboard::new() {
            Ok(value) => value,
            Err(error) => {
                eprintln!("[WARN] Failed to open clipboard after change: {error}");
                return;
            }
        };
        let source_snapshot =
            crate::infrastructure::windows_api::window_tracker::get_clipboard_source_app_info();
        let settings = app.state::<SettingsState>();

        let files = clipboard
            .get()
            .file_list()
            .ok()
            .filter(|paths| !paths.is_empty());
        let text = clipboard
            .get_text()
            .ok()
            .map(|value| normalize_clipboard_plain_text(&value))
            .filter(|value| !value.trim().is_empty());
        let html = if settings.capture_rich_text.load(Ordering::Relaxed) {
            use clipboard_rs::{Clipboard as _, ClipboardContext};
            ClipboardContext::new()
                .ok()
                .and_then(|context| context.get_html().ok())
                .filter(|value| !value.trim().is_empty())
        } else {
            None
        };
        let image = clipboard.get_image().ok();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut monitor_state = match state_lock.lock() {
            Ok(value) => value,
            Err(error) => error.into_inner(),
        };

        let mut process_hash = |hash: u64| {
            if hash != 0
                && hash == monitor_state.last_content_hash
                && now.saturating_sub(monitor_state.last_process_time) < 750
            {
                return false;
            }
            monitor_state.last_content_hash = hash;
            monitor_state.last_process_time = now;
            true
        };

        if let Some(paths) = files {
            let display_paths: Vec<String> = paths
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            let content = display_paths.join("\n");
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            "files".hash(&mut hasher);
            content.hash(&mut hasher);
            let hash = hasher.finish();

            if process_hash(hash)
                && settings.capture_files.load(Ordering::Relaxed)
                && !is_recent_self_copy(calc_text_hash(&content))
            {
                process_new_entry(
                    &app,
                    ClipboardData::Files(display_paths),
                    None,
                    Some(source_snapshot),
                );
            }
            return;
        }

        if let (Some(text), Some(html)) = (text.as_ref(), html.as_ref()) {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            "rich_text".hash(&mut hasher);
            text.hash(&mut hasher);
            html.hash(&mut hasher);
            let hash = hasher.finish();

            if process_hash(hash) && !is_recent_self_copy(calc_text_hash(text)) {
                process_new_entry(
                    &app,
                    ClipboardData::RichText {
                        text: text.clone(),
                        html: html.clone(),
                    },
                    None,
                    Some(source_snapshot),
                );
            }
            return;
        }

        if let Some(image) = image {
            let raw_bytes = image.bytes.into_owned();
            let mut raw_hasher = std::collections::hash_map::DefaultHasher::new();
            raw_bytes.hash(&mut raw_hasher);
            let raw_hash = raw_hasher.finish();
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            "image".hash(&mut hasher);
            image.width.hash(&mut hasher);
            image.height.hash(&mut hasher);
            raw_bytes.hash(&mut hasher);
            let hash = hasher.finish();
            let visual_hash =
                calc_image_hash_from_rgba(image.width as u32, image.height as u32, &raw_bytes)
                    .unwrap_or(hash as i64) as u64;

            if process_hash(hash) {
                if should_ignore_recent_image_echo(raw_hash, visual_hash) {
                    clear_recent_image_echo_state();
                } else if let Some(buffer) =
                    image::RgbaImage::from_raw(image.width as u32, image.height as u32, raw_bytes)
                {
                    let mut png = Vec::new();
                    if buffer
                        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                        .is_ok()
                    {
                        process_new_entry(
                            &app,
                            ClipboardData::Image {
                                data_url: format!(
                                    "data:image/png;base64,{}",
                                    base64::engine::general_purpose::STANDARD.encode(png)
                                ),
                            },
                            None,
                            Some(source_snapshot),
                        );
                    }
                }
            }
            return;
        }

        if let Some(text) = text {
            let hash = calc_text_hash(&text);
            if process_hash(hash) && !is_recent_self_copy(hash) {
                process_new_entry(&app, ClipboardData::Text(text), None, Some(source_snapshot));
            }
        }
    }));
}

#[cfg(not(target_os = "windows"))]
fn is_recent_self_copy(hash: u64) -> bool {
    let last_hash = crate::LAST_APP_SET_HASH.load(Ordering::SeqCst);
    let last_hash_alt = crate::LAST_APP_SET_HASH_ALT.load(Ordering::SeqCst);
    let last_time = crate::LAST_APP_SET_TIMESTAMP.load(Ordering::SeqCst);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let is_echo = last_hash != 0
        && (hash == last_hash || hash == last_hash_alt)
        && now.saturating_sub(last_time) < 10;
    if is_echo {
        crate::LAST_APP_SET_HASH.store(0, Ordering::SeqCst);
        crate::LAST_APP_SET_HASH_ALT.store(0, Ordering::SeqCst);
    }
    is_echo
}

pub use pipeline::{ClipboardData, ClipboardPipeline, PipelineContext};
pub use utils::{
    attach_rich_image_fallback, attach_rich_named_formats, build_clipboard_text_fingerprint,
    build_entry_preview, derive_rich_text_content, extract_animated_image_data_url_from_html,
    extract_animated_image_data_url_from_text, extract_first_image_data_url_from_html,
    parse_cf_html, repair_html_fragment, split_rich_html_and_image_fallback,
    split_rich_html_and_named_formats, truncate_html_for_preview,
};

pub fn process_new_entry(
    app_handle: &AppHandle,
    data: ClipboardData,
    source_override: Option<String>,
    source_snapshot: Option<crate::infrastructure::windows_api::window_tracker::ActiveAppInfo>,
) {
    let mut ctx = PipelineContext::new(app_handle.clone(), data, source_snapshot);
    if let Some(source) = source_override {
        ctx.source_app = source;
        ctx.source_app_path = None;
    }

    let pipeline = ClipboardPipeline::new();
    pipeline.execute(&mut ctx);
}
