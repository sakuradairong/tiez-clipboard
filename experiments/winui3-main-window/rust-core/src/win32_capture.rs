//! Windows clipboard-format listener for the WinUI probe.
//!
//! Mirrors the production `listen_clipboard` contract: a message-only HWND
//! receives `WM_CLIPBOARDUPDATE`, and clipboard reads happen on a bounded
//! worker thread — never inside the window procedure.

use crate::TiezCoreInner;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

#[cfg(all(windows, not(test)))]
use crate::ingest_captured_snapshot;
#[cfg(all(windows, not(test)))]
use crate::win32_paste::{decode_cf_dib, decode_hdrop};
#[cfg(all(windows, not(test)))]
use std::hash::{Hash, Hasher};
#[cfg(all(windows, not(test)))]
use std::io::Cursor;
#[cfg(all(windows, not(test)))]
use std::ptr;
#[cfg(all(windows, not(test)))]
use std::sync::mpsc;
#[cfg(all(windows, not(test)))]
use std::thread;
#[cfg(all(windows, not(test)))]
use std::time::Duration;
#[cfg(all(windows, not(test)))]
use tiez_core::clipboard_capture::{classify_snapshot, ClipboardSnapshot};

#[cfg(all(windows, not(test)))]
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
#[cfg(all(windows, not(test)))]
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, IsClipboardFormatAvailable,
    OpenClipboard, RegisterClipboardFormatW, RemoveClipboardFormatListener,
};
#[cfg(all(windows, not(test)))]
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(all(windows, not(test)))]
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
#[cfg(all(windows, not(test)))]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowLongPtrW,
    PostMessageW, RegisterClassW, SetWindowLongPtrW, GWLP_USERDATA, HWND_MESSAGE, MSG,
    WM_CLIPBOARDUPDATE, WM_QUIT, WNDCLASSW,
};

#[cfg(all(windows, not(test)))]
const CF_UNICODETEXT: u32 = 13;
#[cfg(all(windows, not(test)))]
const CF_DIB: u32 = 8;
#[cfg(all(windows, not(test)))]
const CF_HDROP: u32 = 15;

pub struct Session {
    stop: Arc<AtomicBool>,
    #[allow(dead_code)]
    hwnd: Arc<AtomicIsize>,
    worker: Option<JoinHandle<()>>,
    listener: Option<JoinHandle<()>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        #[cfg(all(windows, not(test)))]
        {
            let hwnd = self.hwnd.load(Ordering::SeqCst) as HWND;
            if !hwnd.is_null() {
                unsafe {
                    PostMessageW(hwnd, WM_QUIT, 0, 0);
                }
            }
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
    }
}

pub fn start(inner: Arc<TiezCoreInner>) -> Result<Session, String> {
    #[cfg(all(windows, not(test)))]
    {
        start_windows(inner)
    }
    #[cfg(not(all(windows, not(test))))]
    {
        let _ = inner;
        Ok(Session {
            stop: Arc::new(AtomicBool::new(false)),
            hwnd: Arc::new(AtomicIsize::new(0)),
            worker: None,
            listener: None,
        })
    }
}

#[cfg(all(windows, not(test)))]
fn start_windows(inner: Arc<TiezCoreInner>) -> Result<Session, String> {
    if let Some(existing) = read_clipboard_snapshot() {
        if let Some(payload) = classify_snapshot(existing) {
            if let Ok(mut filter) = inner.capture.lock() {
                filter.prime_payload(&payload);
            }
        }
    }

    let stop = Arc::new(AtomicBool::new(false));
    let hwnd = Arc::new(AtomicIsize::new(0));
    let (notify_tx, notify_rx) = mpsc::sync_channel::<()>(1);

    let worker_inner = Arc::clone(&inner);
    let worker_stop = Arc::clone(&stop);
    let worker = thread::Builder::new()
        .name("tiez-winui-clipboard-worker".into())
        .spawn(move || {
            while notify_rx.recv().is_ok() {
                if worker_stop.load(Ordering::SeqCst) {
                    break;
                }
                thread::sleep(Duration::from_millis(30));
                let Some(snapshot) = read_clipboard_snapshot() else {
                    continue;
                };
                let _ = ingest_captured_snapshot(&worker_inner, snapshot);
            }
        })
        .map_err(|error| format!("failed to start clipboard worker: {error}"))?;

    let hwnd_for_listener = Arc::clone(&hwnd);
    let stop_for_listener = Arc::clone(&stop);
    let listener = thread::Builder::new()
        .name("tiez-winui-clipboard-listener".into())
        .spawn(move || unsafe {
            run_listener(notify_tx, hwnd_for_listener, stop_for_listener);
        })
        .map_err(|error| format!("failed to start clipboard listener: {error}"))?;

    Ok(Session {
        stop,
        hwnd,
        worker: Some(worker),
        listener: Some(listener),
    })
}

#[cfg(all(windows, not(test)))]
unsafe fn run_listener(
    notify_tx: mpsc::SyncSender<()>,
    hwnd_slot: Arc<AtomicIsize>,
    stop: Arc<AtomicBool>,
) {
    let instance = GetModuleHandleW(ptr::null());
    let class_name: Vec<u16> = "TieZWinUIClipboardListener\0".encode_utf16().collect();
    let class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: ptr::null_mut(),
        hCursor: ptr::null_mut(),
        hbrBackground: ptr::null_mut(),
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    let _ = RegisterClassW(&class);

    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        ptr::null(),
        0,
        0,
        0,
        0,
        0,
        HWND_MESSAGE,
        ptr::null_mut(),
        instance,
        ptr::null(),
    );
    if hwnd.is_null() {
        return;
    }
    hwnd_slot.store(hwnd as isize, Ordering::SeqCst);

    let boxed = Box::new(notify_tx);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(boxed) as isize);

    if AddClipboardFormatListener(hwnd) == 0 {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut mpsc::SyncSender<()>;
        if !ptr.is_null() {
            drop(Box::from_raw(ptr));
        }
        hwnd_slot.store(0, Ordering::SeqCst);
        return;
    }

    let mut msg = std::mem::zeroed::<MSG>();
    while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) != 0 {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        DispatchMessageW(&msg);
    }

    let _ = RemoveClipboardFormatListener(hwnd);
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut mpsc::SyncSender<()>;
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    if !ptr.is_null() {
        drop(Box::from_raw(ptr));
    }
    hwnd_slot.store(0, Ordering::SeqCst);
}

#[cfg(all(windows, not(test)))]
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CLIPBOARDUPDATE {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut mpsc::SyncSender<()>;
        if !ptr.is_null() {
            let _ = (*ptr).try_send(());
        }
        return 0;
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

#[cfg(all(windows, not(test)))]
pub(crate) fn read_clipboard_snapshot() -> Option<ClipboardSnapshot> {
    for _ in 0..5 {
        if let Some(snapshot) = unsafe { try_read_clipboard_snapshot() } {
            return Some(snapshot);
        }
        thread::sleep(Duration::from_millis(10));
    }
    None
}

#[cfg(all(windows, not(test)))]
pub(crate) fn read_plain_text_exact() -> Result<String, String> {
    read_clipboard_snapshot()
        .and_then(|snapshot| snapshot.text)
        .ok_or_else(|| "剪贴板中没有可发送的文本".to_owned())
}

#[cfg(not(all(windows, not(test))))]
pub(crate) fn read_plain_text_exact() -> Result<String, String> {
    Err("当前测试平台没有可用的 Windows 文本剪贴板".to_owned())
}

#[cfg(all(windows, not(test)))]
unsafe fn try_read_clipboard_snapshot() -> Option<ClipboardSnapshot> {
    if OpenClipboard(ptr::null_mut()) == 0 {
        return None;
    }
    let files = read_hdrop();
    let html = read_named_text("HTML Format");
    let text = read_unicode_text();
    let png = read_named_bytes("PNG");
    let dib = read_format_bytes(CF_DIB);
    CloseClipboard();

    let image = persist_image(png, dib);
    if files.is_empty() && html.is_none() && text.is_none() && image.is_none() {
        return None;
    }
    Some(ClipboardSnapshot {
        text,
        html,
        image,
        files,
    })
}

#[cfg(all(windows, not(test)))]
unsafe fn read_unicode_text() -> Option<String> {
    if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 {
        return None;
    }
    let handle = GetClipboardData(CF_UNICODETEXT);
    if handle.is_null() {
        return None;
    }
    let locked = GlobalLock(handle);
    if locked.is_null() {
        return None;
    }
    let result = {
        let ptr = locked as *const u16;
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
            if len > 8 * 1024 * 1024 {
                break;
            }
        }
        String::from_utf16(std::slice::from_raw_parts(ptr, len)).ok()
    };
    GlobalUnlock(handle);
    result.filter(|value| !value.is_empty())
}

#[cfg(all(windows, not(test)))]
unsafe fn read_hdrop() -> Vec<String> {
    if IsClipboardFormatAvailable(CF_HDROP) == 0 {
        return Vec::new();
    }
    let Some(bytes) = read_format_bytes(CF_HDROP) else {
        return Vec::new();
    };
    decode_hdrop(&bytes).unwrap_or_default()
}

#[cfg(all(windows, not(test)))]
unsafe fn read_named_text(name: &str) -> Option<String> {
    let bytes = read_named_bytes(name)?;
    let text = if bytes.len() >= 2 && bytes[1] == 0 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(all(windows, not(test)))]
unsafe fn read_named_bytes(name: &str) -> Option<Vec<u8>> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let format = RegisterClipboardFormatW(wide.as_ptr());
    if format == 0 || IsClipboardFormatAvailable(format) == 0 {
        return None;
    }
    read_format_bytes(format)
}

#[cfg(all(windows, not(test)))]
unsafe fn read_format_bytes(format: u32) -> Option<Vec<u8>> {
    let handle = GetClipboardData(format);
    if handle.is_null() {
        return None;
    }
    let size = GlobalSize(handle);
    if size == 0 {
        return None;
    }
    let locked = GlobalLock(handle);
    if locked.is_null() {
        return None;
    }
    let bytes = std::slice::from_raw_parts(locked as *const u8, size).to_vec();
    GlobalUnlock(handle);
    Some(bytes)
}

#[cfg(all(windows, not(test)))]
fn persist_image(png: Option<Vec<u8>>, dib: Option<Vec<u8>>) -> Option<String> {
    let png = png
        .filter(|bytes| bytes.starts_with(b"\x89PNG"))
        .or_else(|| {
            let dib = dib?;
            let (width, height, rgba) = decode_cf_dib(&dib).ok()?;
            encode_png(width, height, &rgba).ok()
        })?;
    persist_png(&png).ok()
}

#[cfg(all(windows, not(test)))]
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| "RGBA buffer does not match width and height".to_owned())?;
    let image = image::DynamicImage::ImageRgba8(buffer);
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|error| format!("failed to encode captured PNG: {error}"))?;
    Ok(bytes)
}

#[cfg(all(windows, not(test)))]
fn persist_png(png: &[u8]) -> Result<String, String> {
    let dir = std::env::temp_dir().join("tiez-winui-capture");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create capture directory: {error}"))?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    png.hash(&mut hasher);
    let path = dir.join(format!("{:016x}.png", hasher.finish()));
    std::fs::write(&path, png).map_err(|error| format!("failed to write captured PNG: {error}"))?;
    Ok(path.to_string_lossy().into_owned())
}
