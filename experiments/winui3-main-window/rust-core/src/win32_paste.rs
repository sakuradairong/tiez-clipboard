//! Windows clipboard + Ctrl+V executor for the WinUI probe.
//!
//! Focus restore and window hide stay in the C++ `UiLifecycle` shell so this
//! module never takes a Tauri `AppHandle` or HWND through the C ABI.

#[cfg(not(test))]
use tiez_core::paste_coordinator::PastePlan;

#[cfg(not(test))]
use std::ptr;
#[cfg(not(test))]
use tiez_core::paste_coordinator::{execute_paste, PasteExecutor, PastePayload};
#[cfg(not(test))]
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
#[cfg(not(test))]
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
#[cfg(not(test))]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};

/// Builds a CF_HTML payload from an HTML fragment or an already-wrapped document.
fn encode_cf_html(html: &str) -> String {
    let fragment = html.trim();
    let html_content = if fragment.contains("<!--StartFragment-->")
        && fragment.contains("<!--EndFragment-->")
        && fragment.to_ascii_lowercase().contains("<html")
    {
        fragment.to_owned()
    } else {
        format!(
            "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n</head>\n<body>\n<!--StartFragment-->{fragment}<!--EndFragment-->\n</body>\n</html>"
        )
    };

    let header_template = "Version:1.0\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let header_len = header_template.len();
    let start_html = header_len;
    let end_html = start_html + html_content.len();
    let start_marker = "<!--StartFragment-->";
    let end_marker = "<!--EndFragment-->";
    let start_fragment = start_html
        + html_content.find(start_marker).unwrap_or(0)
        + start_marker.len();
    let end_fragment = start_html + html_content.find(end_marker).unwrap_or(html_content.len());

    format!(
        "Version:1.0\r\nStartHTML:{start_html:0>10}\r\nEndHTML:{end_html:0>10}\r\nStartFragment:{start_fragment:0>10}\r\nEndFragment:{end_fragment:0>10}\r\n{html_content}"
    )
}

fn encode_cf_dib(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let pixel_count = (width as usize).saturating_mul(height as usize);
    if rgba.len() != pixel_count.saturating_mul(4) {
        return Err("RGBA buffer does not match width and height".to_owned());
    }

    let mut dib = Vec::with_capacity(40 + rgba.len());
    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&(width as i32).to_le_bytes());
    dib.extend_from_slice(&(height as i32).to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&(rgba.len() as u32).to_le_bytes());
    dib.extend_from_slice(&0i32.to_le_bytes());
    dib.extend_from_slice(&0i32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());

    for row in (0..height as usize).rev() {
        let start = row.saturating_mul(width as usize).saturating_mul(4);
        for pixel in rgba[start..start + (width as usize) * 4].chunks_exact(4) {
            dib.push(pixel[2]);
            dib.push(pixel[1]);
            dib.push(pixel[0]);
            dib.push(pixel[3]);
        }
    }
    Ok(dib)
}

fn encode_hdrop(paths: &[String]) -> Vec<u8> {
    const HEADER_SIZE: u32 = 20;
    let mut bytes = vec![0u8; HEADER_SIZE as usize];
    bytes[0..4].copy_from_slice(&HEADER_SIZE.to_le_bytes());
    bytes[16..20].copy_from_slice(&1i32.to_le_bytes());
    for path in paths {
        for unit in path.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

#[cfg(not(test))]
pub(crate) fn execute(plan: &PastePlan) -> Result<(), String> {
    execute_paste(plan, &mut Win32PasteExecutor)
}

#[cfg(not(test))]
struct Win32PasteExecutor;

#[cfg(not(test))]
impl PasteExecutor for Win32PasteExecutor {
    fn hide_window(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn restore_focus(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn apply_payload(&mut self, payload: &PastePayload) -> Result<(), String> {
        if !payload.files.is_empty() {
            return set_hdrop_files(&payload.files);
        }
        if let Some(image) = payload.image.as_deref() {
            return set_image(image);
        }
        match payload.html.as_deref() {
            Some(html) if !html.trim().is_empty() => {
                set_unicode_text_and_html(&payload.text, html)
            }
            _ => set_unicode_text(&payload.text),
        }
    }

    fn send_paste_keystroke(&mut self) -> Result<(), String> {
        send_ctrl_v()
    }
}

#[cfg(not(test))]
const CF_UNICODETEXT: u32 = 13;
#[cfg(not(test))]
const CF_DIB: u32 = 8;
#[cfg(not(test))]
const CF_HDROP: u32 = 15;
#[cfg(not(test))]
const VK_CONTROL: VIRTUAL_KEY = 0x11;
#[cfg(not(test))]
const VK_V: VIRTUAL_KEY = 0x56;

#[cfg(not(test))]
fn set_unicode_text(text: &str) -> Result<(), String> {
    with_open_clipboard(|clipboard| {
        empty_clipboard(clipboard)?;
        set_unicode_on_clipboard(text)
    })
}

#[cfg(not(test))]
fn set_unicode_text_and_html(text: &str, html: &str) -> Result<(), String> {
    with_open_clipboard(|clipboard| {
        empty_clipboard(clipboard)?;
        set_unicode_on_clipboard(text)?;
        set_html_on_clipboard(html)
    })
}

#[cfg(not(test))]
struct OpenClipboardGuard;

#[cfg(not(test))]
fn with_open_clipboard<T>(
    operation: impl FnOnce(&OpenClipboardGuard) -> Result<T, String>,
) -> Result<T, String> {
    unsafe {
        if OpenClipboard(ptr::null_mut()) == 0 {
            return Err("OpenClipboard failed".to_owned());
        }
    }
    let result = operation(&OpenClipboardGuard);
    unsafe {
        CloseClipboard();
    }
    result
}

#[cfg(not(test))]
fn empty_clipboard(_clipboard: &OpenClipboardGuard) -> Result<(), String> {
    unsafe {
        if EmptyClipboard() == 0 {
            return Err("EmptyClipboard failed".to_owned());
        }
    }
    Ok(())
}

#[cfg(not(test))]
fn set_unicode_on_clipboard(text: &str) -> Result<(), String> {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let bytes = wide.len().saturating_mul(std::mem::size_of::<u16>());
    let handle = alloc_moveable(bytes)?;
    unsafe {
        let locked = GlobalLock(handle);
        if locked.is_null() {
            return Err("GlobalLock failed".to_owned());
        }
        ptr::copy_nonoverlapping(wide.as_ptr(), locked.cast::<u16>(), wide.len());
        GlobalUnlock(handle);
        if SetClipboardData(CF_UNICODETEXT, handle).is_null() {
            return Err("SetClipboardData failed".to_owned());
        }
    }
    Ok(())
}

#[cfg(not(test))]
fn set_html_on_clipboard(html: &str) -> Result<(), String> {
    let encoded = encode_cf_html(html);
    let mut bytes = encoded.into_bytes();
    bytes.push(0);
    set_named_format_bytes("HTML Format", &bytes)
}

#[cfg(not(test))]
fn set_hdrop_files(paths: &[String]) -> Result<(), String> {
    with_open_clipboard(|clipboard| {
        empty_clipboard(clipboard)?;
        set_bytes_on_clipboard(CF_HDROP, &encode_hdrop(paths))
    })
}

#[cfg(not(test))]
fn set_image(source: &str) -> Result<(), String> {
    let original = read_image_source(source)?;
    let (width, height, rgba) = decode_image_pixels(&original)?;
    let dib = encode_cf_dib(width, height, &rgba)?;
    with_open_clipboard(|clipboard| {
        empty_clipboard(clipboard)?;
        set_bytes_on_clipboard(CF_DIB, &dib)?;
        if original.starts_with(b"\x89PNG") {
            let mut png = original.clone();
            png.push(0);
            set_named_format_bytes("PNG", &png)?;
        }
        if looks_like_file_path(source) {
            set_bytes_on_clipboard(CF_HDROP, &encode_hdrop(&[normalize_file_path(source)]))?;
        }
        Ok(())
    })
}

#[cfg(not(test))]
fn looks_like_file_path(source: &str) -> bool {
    let source = source.trim();
    !source.starts_with("data:image/")
}

fn normalize_file_path(source: &str) -> String {
    source
        .trim()
        .trim_start_matches("file:///")
        .trim_start_matches("file://")
        .to_owned()
}

#[cfg(not(test))]
fn read_image_source(source: &str) -> Result<Vec<u8>, String> {
    let source = source.trim();
    if let Some(rest) = source.strip_prefix("data:image/") {
        let encoded = rest.split(',').nth(1).ok_or_else(|| {
            "image data URL is missing base64 payload".to_owned()
        })?;
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
            .map_err(|error| format!("image data URL is not valid base64: {error}"))
    } else {
        std::fs::read(normalize_file_path(source))
            .map_err(|error| format!("failed to read image file: {error}"))
    }
}

#[cfg(not(test))]
fn decode_image_pixels(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let image = image::load_from_memory(bytes)
        .map_err(|error| format!("failed to decode image: {error}"))?
        .to_rgba8();
    let (width, height) = image.dimensions();
    Ok((width, height, image.into_raw()))
}

#[cfg(not(test))]
fn set_named_format_bytes(name: &str, bytes: &[u8]) -> Result<(), String> {
    let mut format_name: Vec<u16> = name.encode_utf16().collect();
    format_name.push(0);
    let format_id = unsafe { RegisterClipboardFormatW(format_name.as_ptr()) };
    if format_id == 0 {
        return Err(format!("RegisterClipboardFormatW failed for {name}"));
    }
    set_bytes_on_clipboard(format_id, bytes)
}

#[cfg(not(test))]
fn set_bytes_on_clipboard(format: u32, bytes: &[u8]) -> Result<(), String> {
    let handle = alloc_moveable(bytes.len())?;
    unsafe {
        let locked = GlobalLock(handle);
        if locked.is_null() {
            return Err("GlobalLock failed".to_owned());
        }
        ptr::copy_nonoverlapping(bytes.as_ptr(), locked.cast::<u8>(), bytes.len());
        GlobalUnlock(handle);
        if SetClipboardData(format, handle).is_null() {
            return Err("SetClipboardData failed".to_owned());
        }
    }
    Ok(())
}

#[cfg(not(test))]
fn alloc_moveable(bytes: usize) -> Result<*mut core::ffi::c_void, String> {
    let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) };
    if handle.is_null() {
        return Err("GlobalAlloc failed".to_owned());
    }
    Ok(handle)
}

#[cfg(not(test))]
fn send_ctrl_v() -> Result<(), String> {
    let inputs = [
        keyboard_input(VK_CONTROL, false),
        keyboard_input(VK_V, false),
        keyboard_input(VK_V, true),
        keyboard_input(VK_CONTROL, true),
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        return Err("SendInput failed for Ctrl+V".to_owned());
    }
    Ok(())
}

#[cfg(not(test))]
fn keyboard_input(virtual_key: VIRTUAL_KEY, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: 0,
                dwFlags: if key_up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{encode_cf_dib, encode_cf_html, encode_hdrop, normalize_file_path};

    #[test]
    fn encode_cf_html_wraps_a_fragment_and_keeps_fixed_width_offsets() {
        let encoded = encode_cf_html("<b>hi</b>");
        assert!(encoded.starts_with("Version:1.0\r\nStartHTML:"));
        assert!(!encoded.contains("StartHTML:0000000000"));
        assert!(encoded.contains("<!--StartFragment--><b>hi</b><!--EndFragment-->"));
    }

    #[test]
    fn encode_cf_html_keeps_an_already_wrapped_document() {
        let document = "<html><body><!--StartFragment--><i>kept</i><!--EndFragment--></body></html>";
        let encoded = encode_cf_html(document);
        assert!(encoded.contains(document));
        assert_eq!(encoded.matches("<!--StartFragment-->").count(), 1);
    }

    #[test]
    fn encode_cf_dib_converts_rgba_to_bottom_up_bgra() {
        let dib = encode_cf_dib(1, 1, &[255, 0, 0, 255]).unwrap();
        assert_eq!(&dib[0..4], 40u32.to_le_bytes());
        assert_eq!(&dib[40..], [0, 0, 255, 255]);
    }

    #[test]
    fn encode_hdrop_includes_wide_paths() {
        let encoded = encode_hdrop(&[r"C:\a.txt".to_owned()]);
        let utf16: Vec<u8> = r"C:\a.txt".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(&encoded[0..4], 20u32.to_le_bytes());
        assert_eq!(&encoded[16..20], 1i32.to_le_bytes());
        assert!(encoded.windows(utf16.len()).any(|window| window == utf16));
    }

    #[test]
    fn normalize_file_path_strips_file_url_prefix() {
        assert_eq!(normalize_file_path("file:///C:/shot.png"), "C:/shot.png");
    }
}
