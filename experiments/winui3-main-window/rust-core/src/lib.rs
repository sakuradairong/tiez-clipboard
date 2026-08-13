//! Throwaway Rust core used by the WinUI 3 main-window experiment.
//!
//! The interface deliberately stays C-only. It tests the seam that a future
//! WinUI adapter would use without coupling the experiment to Tauri types.

use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::Mutex;

const ABI_VERSION: u32 = 1;

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClipboardItem {
    id: i64,
    content_type: &'static str,
    preview: String,
    source_app: &'static str,
    captured_at: &'static str,
    is_pinned: bool,
}

#[derive(Debug)]
struct CoreState {
    generation: u64,
    last_action: String,
    items: Vec<ClipboardItem>,
}

#[repr(C)]
pub struct TiezCoreHandle {
    state: Mutex<CoreState>,
}

impl TiezCoreHandle {
    fn new() -> Self {
        Self {
            state: Mutex::new(CoreState {
                generation: 1,
                last_action: "Rust core ready".to_owned(),
                items: sample_items(),
            }),
        }
    }
}

fn sample_items() -> Vec<ClipboardItem> {
    vec![
        ClipboardItem {
            id: 101,
            content_type: "text",
            preview: "WinUI 3 main-window probe is connected to Rust through a C ABI."
                .to_owned(),
            source_app: "Visual Studio",
            captured_at: "Just now",
            is_pinned: true,
        },
        ClipboardItem {
            id: 102,
            content_type: "code",
            preview: "tiez_core_get_snapshot_json(handle, query);".to_owned(),
            source_app: "Windows Terminal",
            captured_at: "1 minute ago",
            is_pinned: false,
        },
        ClipboardItem {
            id: 103,
            content_type: "url",
            preview: "https://github.com/jimuzhe/tiez-clipboard/issues/154".to_owned(),
            source_app: "Microsoft Edge",
            captured_at: "3 minutes ago",
            is_pinned: false,
        },
        ClipboardItem {
            id: 104,
            content_type: "text",
            preview: "中文、emoji 🚀 和 UTF-8 必须完整穿过 Rust/C++ 边界。".to_owned(),
            source_app: "TieZ",
            captured_at: "5 minutes ago",
            is_pinned: false,
        },
        ClipboardItem {
            id: 105,
            content_type: "image",
            preview: "Image preview placeholder · 1920 × 1080".to_owned(),
            source_app: "Snipping Tool",
            captured_at: "8 minutes ago",
            is_pinned: false,
        },
        ClipboardItem {
            id: 106,
            content_type: "files",
            preview: "release-notes.md\nTieZ-setup.exe".to_owned(),
            source_app: "File Explorer",
            captured_at: "12 minutes ago",
            is_pinned: false,
        },
    ]
}

fn set_last_error(message: impl Into<String>) {
    let message = message.into().replace('\0', "\\0");
    let value = CString::new(message).expect("NUL bytes were replaced");
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(value));
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| {
        slot.borrow_mut().take();
    });
}

fn required_utf8(value: *const c_char, field: &str) -> Result<String, String> {
    if value.is_null() {
        return Err(format!("{field} must not be null"));
    }

    // SAFETY: the C interface requires a valid NUL-terminated pointer. The
    // null case is rejected above and invalid UTF-8 is reported as an error.
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .map(str::to_owned)
        .map_err(|error| format!("{field} must be valid UTF-8: {error}"))
}

fn optional_utf8(value: *const c_char) -> Result<String, String> {
    if value.is_null() {
        return Ok(String::new());
    }

    // SAFETY: a non-null pointer supplied through the C interface must point
    // to a NUL-terminated string for the duration of this call.
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .map(str::to_owned)
        .map_err(|error| format!("query_utf8 must be valid UTF-8: {error}"))
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');

    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            value if value <= '\u{001f}' => {
                use std::fmt::Write;
                write!(output, "\\u{:04x}", value as u32).expect("writing to String cannot fail");
            }
            value => output.push(value),
        }
    }

    output.push('"');
    output
}

fn snapshot_json(state: &CoreState, query: &str) -> String {
    let normalized_query = query.trim().to_lowercase();
    let visible_items: Vec<_> = state
        .items
        .iter()
        .filter(|item| {
            normalized_query.is_empty()
                || item.preview.to_lowercase().contains(&normalized_query)
                || item.source_app.to_lowercase().contains(&normalized_query)
                || item.content_type.contains(&normalized_query)
        })
        .collect();

    let mut output = format!(
        "{{\"abi_version\":{ABI_VERSION},\"generation\":{},\"total\":{},\"query\":{},\"last_action\":{},\"items\":[",
        state.generation,
        visible_items.len(),
        json_string(query),
        json_string(&state.last_action),
    );

    for (index, item) in visible_items.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }

        output.push_str(&format!(
            "{{\"id\":{},\"content_type\":{},\"preview\":{},\"source_app\":{},\"captured_at\":{},\"is_pinned\":{}}}",
            item.id,
            json_string(item.content_type),
            json_string(&item.preview),
            json_string(item.source_app),
            json_string(item.captured_at),
            item.is_pinned,
        ));
    }

    output.push_str("]}");
    output
}

fn into_owned_c_string(value: String) -> Result<*mut c_char, String> {
    CString::new(value)
        .map(CString::into_raw)
        .map_err(|_| "response unexpectedly contained an interior NUL byte".to_owned())
}

fn catch_ffi<T>(fallback: T, operation: impl FnOnce() -> T) -> T {
    clear_last_error();

    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(value) => value,
        Err(_) => {
            set_last_error("Rust core panicked across the C ABI");
            fallback
        }
    }
}

fn with_ffi_result<T>(fallback: T, operation: impl FnOnce() -> Result<T, String>) -> T {
    clear_last_error();

    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            set_last_error(error);
            fallback
        }
        Err(_) => {
            set_last_error("Rust core panicked across the C ABI");
            fallback
        }
    }
}

#[no_mangle]
pub extern "C" fn tiez_core_abi_version() -> u32 {
    ABI_VERSION
}

#[no_mangle]
pub extern "C" fn tiez_core_create() -> *mut TiezCoreHandle {
    with_ffi_result(ptr::null_mut(), || {
        Ok(Box::into_raw(Box::new(TiezCoreHandle::new())))
    })
}

#[no_mangle]
pub unsafe extern "C" fn tiez_core_destroy(handle: *mut TiezCoreHandle) {
    catch_ffi((), || {
        if handle.is_null() {
            return;
        }

        // SAFETY: handles are produced by tiez_core_create and ownership is
        // transferred back exactly once by this function.
        drop(unsafe { Box::from_raw(handle) });
    });
}

#[no_mangle]
pub unsafe extern "C" fn tiez_core_get_snapshot_json(
    handle: *mut TiezCoreHandle,
    query_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle = unsafe { handle.as_ref() }
            .ok_or_else(|| "handle must not be null".to_owned())?;
        let query = optional_utf8(query_utf8)?;
        let state = handle
            .state
            .lock()
            .map_err(|_| "Rust core state lock is poisoned".to_owned())?;

        into_owned_c_string(snapshot_json(&state, &query))
    })
}

#[no_mangle]
pub unsafe extern "C" fn tiez_core_apply_action(
    handle: *mut TiezCoreHandle,
    entry_id: i64,
    action_utf8: *const c_char,
) -> bool {
    with_ffi_result(false, || {
        let handle = unsafe { handle.as_ref() }
            .ok_or_else(|| "handle must not be null".to_owned())?;
        let action = required_utf8(action_utf8, "action_utf8")?;
        let mut state = handle
            .state
            .lock()
            .map_err(|_| "Rust core state lock is poisoned".to_owned())?;

        match action.as_str() {
            "pin" => {
                let item = state
                    .items
                    .iter_mut()
                    .find(|item| item.id == entry_id)
                    .ok_or_else(|| format!("clipboard entry {entry_id} was not found"))?;
                item.is_pinned = !item.is_pinned;
                let is_pinned = item.is_pinned;
                state.last_action = format!(
                    "Entry {entry_id} {}",
                    if is_pinned { "pinned" } else { "unpinned" }
                );
            }
            "delete" => {
                let previous_len = state.items.len();
                state.items.retain(|item| item.id != entry_id);
                if previous_len == state.items.len() {
                    return Err(format!("clipboard entry {entry_id} was not found"));
                }
                state.last_action = format!("Entry {entry_id} deleted");
            }
            "paste-plain" => {
                ensure_item_exists(&state, entry_id)?;
                state.last_action = format!("Plain-text paste requested for entry {entry_id}");
            }
            "paste-rich" => {
                ensure_item_exists(&state, entry_id)?;
                state.last_action = format!("Rich paste requested for entry {entry_id}");
            }
            _ => return Err(format!("unsupported action: {action}")),
        }

        state.generation += 1;
        Ok(true)
    })
}

fn ensure_item_exists(state: &CoreState, entry_id: i64) -> Result<(), String> {
    state
        .items
        .iter()
        .any(|item| item.id == entry_id)
        .then_some(())
        .ok_or_else(|| format!("clipboard entry {entry_id} was not found"))
}

#[no_mangle]
pub extern "C" fn tiez_core_take_last_error() -> *mut c_char {
    match catch_unwind(AssertUnwindSafe(|| {
        LAST_ERROR.with(|slot| {
            slot.borrow_mut()
                .take()
                .map(CString::into_raw)
                .unwrap_or(ptr::null_mut())
        })
    })) {
        Ok(value) => value,
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn tiez_core_string_free(value: *mut c_char) {
    catch_ffi((), || {
        if value.is_null() {
            return;
        }

        // SAFETY: strings returned by this library are allocated with
        // CString::into_raw and must be released exactly once here.
        drop(unsafe { CString::from_raw(value) });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_filters_and_escapes_items() {
        let state = CoreState {
            generation: 7,
            last_action: "ready".to_owned(),
            items: vec![ClipboardItem {
                id: -1,
                content_type: "text",
                preview: "line one\n\"line two\" 中文".to_owned(),
                source_app: "TieZ",
                captured_at: "now",
                is_pinned: false,
            }],
        };

        let json = snapshot_json(&state, "中文");

        assert!(json.contains("\"generation\":7"));
        assert!(json.contains("line one\\n\\\"line two\\\" 中文"));
        assert!(json.contains("\"total\":1"));
        assert!(snapshot_json(&state, "missing").contains("\"total\":0"));
    }

    #[test]
    fn actions_update_the_generation_and_snapshot() {
        let handle = tiez_core_create();
        assert!(!handle.is_null());

        let pin = CString::new("pin").unwrap();
        let query = CString::new("").unwrap();

        unsafe {
            assert!(tiez_core_apply_action(handle, 102, pin.as_ptr()));
            let snapshot = tiez_core_get_snapshot_json(handle, query.as_ptr());
            assert!(!snapshot.is_null());
            let snapshot_text = CStr::from_ptr(snapshot).to_str().unwrap().to_owned();
            assert!(snapshot_text.contains("\"generation\":2"));
            assert!(snapshot_text.contains("\"id\":102"));
            assert!(snapshot_text.contains("\"is_pinned\":true"));
            tiez_core_string_free(snapshot);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn invalid_action_sets_a_retrievable_error() {
        let handle = tiez_core_create();
        let action = CString::new("explode").unwrap();

        unsafe {
            assert!(!tiez_core_apply_action(handle, 101, action.as_ptr()));
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert_eq!(
                CStr::from_ptr(error).to_str().unwrap(),
                "unsupported action: explode"
            );
            tiez_core_string_free(error);
            tiez_core_destroy(handle);
        }
    }
}
