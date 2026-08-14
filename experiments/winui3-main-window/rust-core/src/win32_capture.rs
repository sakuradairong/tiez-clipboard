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
use crate::ingest_captured_text;
#[cfg(all(windows, not(test)))]
use std::ptr;
#[cfg(all(windows, not(test)))]
use std::sync::mpsc;
#[cfg(all(windows, not(test)))]
use std::thread;
#[cfg(all(windows, not(test)))]
use std::time::Duration;

#[cfg(all(windows, not(test)))]
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
#[cfg(all(windows, not(test)))]
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, OpenClipboard,
    RemoveClipboardFormatListener,
};
#[cfg(all(windows, not(test)))]
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(all(windows, not(test)))]
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
#[cfg(all(windows, not(test)))]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowLongPtrW,
    PostMessageW, RegisterClassW, SetWindowLongPtrW, GWLP_USERDATA, HWND_MESSAGE, MSG,
    WM_CLIPBOARDUPDATE, WM_QUIT, WNDCLASSW,
};

#[cfg(all(windows, not(test)))]
const CF_UNICODETEXT: u32 = 13;

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
    if let Some(existing) = read_unicode_text() {
        if let Ok(mut filter) = inner.capture.lock() {
            filter.prime(&existing);
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
                let Some(raw) = read_unicode_text() else {
                    continue;
                };
                let _ = ingest_captured_text(&worker_inner, &raw);
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
fn read_unicode_text() -> Option<String> {
    for _ in 0..5 {
        let text = unsafe { try_read_unicode_text() };
        if text.is_some() {
            return text;
        }
        thread::sleep(Duration::from_millis(10));
    }
    None
}

#[cfg(all(windows, not(test)))]
unsafe fn try_read_unicode_text() -> Option<String> {
    if OpenClipboard(ptr::null_mut()) == 0 {
        return None;
    }
    let handle = GetClipboardData(CF_UNICODETEXT);
    if handle.is_null() {
        CloseClipboard();
        return None;
    }
    let locked = GlobalLock(handle);
    if locked.is_null() {
        CloseClipboard();
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
    CloseClipboard();
    result
}
