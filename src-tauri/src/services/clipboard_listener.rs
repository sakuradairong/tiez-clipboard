use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::{hash::Hash, time::Duration};
#[cfg(target_os = "windows")]
use windows::core::PCWSTR;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
#[cfg(target_os = "windows")]
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, RemoveClipboardFormatListener,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowLongPtrW,
    RegisterClassW, SetWindowLongPtrW, GWLP_USERDATA, HWND_MESSAGE, MSG, WM_CLIPBOARDUPDATE,
    WNDCLASSW,
};

pub fn listen_clipboard(callback: Arc<dyn Fn() + Send + Sync + 'static>) {
    #[cfg(target_os = "windows")]
    {
        // Never perform clipboard reads from the window procedure itself.
        // Some delayed-rendering providers (notably 32-bit WPS) re-enter their
        // own message loop while serving clipboard formats and can crash when
        // the listener blocks WM_CLIPBOARDUPDATE. A bounded worker also merges
        // bursts of format-update notifications into one processing pass.
        let (notify_tx, notify_rx) = std::sync::mpsc::sync_channel::<()>(1);
        std::thread::spawn(move || {
            while notify_rx.recv().is_ok() {
                callback();
            }
        });
        let notifier: Arc<dyn Fn() + Send + Sync + 'static> = Arc::new(move || {
            let _ = notify_tx.try_send(());
        });

        std::thread::spawn(move || {
            unsafe {
                let instance =
                    windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap();
                let window_class = "TieZClipboardListener";
                let window_class_w: Vec<u16> = window_class
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();

                let wnd_class = WNDCLASSW {
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: instance.into(),
                    lpszClassName: PCWSTR(window_class_w.as_ptr()),
                    ..Default::default()
                };

                RegisterClassW(&wnd_class);

                let hwnd = match CreateWindowExW(
                    Default::default(),
                    PCWSTR(window_class_w.as_ptr()),
                    PCWSTR(std::ptr::null()),
                    Default::default(),
                    0,
                    0,
                    0,
                    0,
                    Some(HWND_MESSAGE), // Use HWND_MESSAGE for invisible message-only window
                    None,
                    Some(HINSTANCE(instance.0)),
                    None,
                ) {
                    Ok(hwnd) => hwnd,
                    Err(e) => {
                        eprintln!(
                            "[ERROR] Failed to create clipboard listener window: {:?}",
                            e
                        );
                        return;
                    }
                };

                // Wrap callback in a Box to store in window user data
                let boxed_callback = Box::new(notifier);
                let ptr = Box::into_raw(boxed_callback);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);

                if let Err(e) = AddClipboardFormatListener(hwnd) {
                    eprintln!("[ERROR] Failed to add clipboard listener: {:?}", e);
                    let _ = Box::from_raw(ptr);
                    return;
                }

                println!(">>> [CLIPBOARD] Windows event-driven listener started.");

                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    DispatchMessageW(&msg);
                }

                let _ = RemoveClipboardFormatListener(hwnd);
                // Cleanup callback
                let _ = Box::from_raw(ptr);
            }
        });
    }

    #[cfg(target_os = "macos")]
    std::thread::spawn(move || {
        use objc2::rc::autoreleasepool;
        use objc2_app_kit::NSPasteboard;

        let pasteboard = autoreleasepool(|_| NSPasteboard::generalPasteboard());
        let mut last_change_count = autoreleasepool(|_| pasteboard.changeCount());
        println!(">>> [CLIPBOARD] macOS change-count listener started.");

        loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let change_count = autoreleasepool(|_| pasteboard.changeCount());
            if change_count != last_change_count {
                last_change_count = change_count;
                callback();
            }
        }
    });

    #[cfg(target_os = "linux")]
    std::thread::spawn(move || {
        use clipboard_rs::{ClipboardHandler, ClipboardWatcher, ClipboardWatcherContext};

        struct CallbackHandler {
            callback: Arc<dyn Fn() + Send + Sync + 'static>,
        }

        impl ClipboardHandler for CallbackHandler {
            fn on_clipboard_change(&mut self) {
                (self.callback)();
            }
        }

        let watcher_callback = callback.clone();
        let watcher_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut watcher = ClipboardWatcherContext::<CallbackHandler>::new()
                .expect("failed to initialize X11 clipboard watcher");
            watcher.add_handler(CallbackHandler {
                callback: watcher_callback,
            });
            println!(">>> [CLIPBOARD] Linux XFixes listener started.");
            watcher.start_watch();
        }));

        if watcher_result.is_err() {
            eprintln!(
                "[WARN] XFixes clipboard listener unavailable; using content polling fallback"
            );
            poll_linux_clipboard(callback);
        }
    });
}

#[cfg(target_os = "linux")]
fn poll_linux_clipboard(callback: Arc<dyn Fn() + Send + Sync + 'static>) {
    let mut clipboard = None;
    let mut last_fingerprint = None;

    loop {
        if clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(mut value) => {
                    last_fingerprint = linux_clipboard_fingerprint(&mut value);
                    clipboard = Some(value);
                }
                Err(error) => {
                    eprintln!("[WARN] Clipboard initialization failed: {error}");
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }
            }
        }

        std::thread::sleep(Duration::from_millis(300));
        let current = clipboard.as_mut().and_then(linux_clipboard_fingerprint);

        if let Some(current_fingerprint) = current {
            if last_fingerprint.is_some() && last_fingerprint != Some(current_fingerprint) {
                callback();
            }
            last_fingerprint = Some(current_fingerprint);
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_clipboard_fingerprint(clipboard: &mut arboard::Clipboard) -> Option<u64> {
    use std::hash::Hasher;

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut found = false;

    if let Ok(files) = clipboard.get().file_list() {
        "files".hash(&mut hasher);
        files.hash(&mut hasher);
        found = true;
    }
    {
        use clipboard_rs::{Clipboard as _, ClipboardContext};
        if let Ok(context) = ClipboardContext::new() {
            if let Ok(html) = context.get_html() {
                "html".hash(&mut hasher);
                html.hash(&mut hasher);
                found = true;
            }
        }
    }
    if let Ok(text) = clipboard.get_text() {
        "text".hash(&mut hasher);
        text.hash(&mut hasher);
        found = true;
    }
    if let Ok(image) = clipboard.get_image() {
        "image".hash(&mut hasher);
        image.width.hash(&mut hasher);
        image.height.hash(&mut hasher);
        image.bytes.hash(&mut hasher);
        found = true;
    }

    found.then(|| hasher.finish())
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let callback = &*(ptr as *const Arc<dyn Fn() + Send + Sync + 'static>);
                callback();
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
