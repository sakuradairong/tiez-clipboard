//! C ABI transport adapter for the WinUI 3 migration slice.
//!
//! Clipboard history behavior lives in the Tauri-independent `tiez-core`
//! crate. This library only owns environment selection, panic containment,
//! UTF-8/C string ownership, and the stable ABI consumed by C++/WinRT.

use serde::Serialize;
use std::cell::RefCell;
use std::env;
use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::Mutex;
use tiez_core::clipboard_history::{
    ClipboardHistory, HistoryContent, HistoryMutationResult, HistorySnapshot,
};

const ABI_VERSION: u32 = 3;
const DATABASE_ENV: &str = "TIEZ_WINUI_DB_PATH";

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

#[repr(C)]
pub struct TiezCoreHandle {
    history: Mutex<ClipboardHistory>,
}

impl TiezCoreHandle {
    fn new_from_environment() -> Result<Self, String> {
        let history = match env::var_os(DATABASE_ENV) {
            Some(value) if !value.is_empty() => ClipboardHistory::open_sqlite_read_only(value)
                .map_err(|error| format!("{DATABASE_ENV}: {error}"))?,
            _ => ClipboardHistory::synthetic(),
        };

        Ok(Self {
            history: Mutex::new(history),
        })
    }
}

#[derive(Serialize)]
struct SnapshotResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a HistorySnapshot,
}

#[derive(Serialize)]
struct MutationResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    mutation: &'a HistoryMutationResult,
}

fn snapshot_json(snapshot: &HistorySnapshot) -> Result<String, String> {
    serde_json::to_string(&SnapshotResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("failed to serialize clipboard history snapshot: {error}"))
}

fn content_json(content: &HistoryContent) -> Result<String, String> {
    serde_json::to_string(content)
        .map_err(|error| format!("failed to serialize clipboard history content: {error}"))
}

fn mutation_json(mutation: &HistoryMutationResult) -> Result<String, String> {
    serde_json::to_string(&MutationResponse {
        abi_version: ABI_VERSION,
        mutation,
    })
    .map_err(|error| format!("failed to serialize clipboard mutation result: {error}"))
}

fn apply_history_action(
    handle: &TiezCoreHandle,
    entry_id: i64,
    action: &str,
) -> Result<HistoryMutationResult, String> {
    let mut history = handle
        .history
        .lock()
        .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
    history
        .apply_action(entry_id, action)
        .map_err(|error| error.to_string())
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
        Ok(Box::into_raw(Box::new(
            TiezCoreHandle::new_from_environment()?,
        )))
    })
}

#[no_mangle]
/// # Safety
///
/// `handle` must be null or a live pointer returned by `tiez_core_create`.
/// A non-null handle must be transferred to this function exactly once.
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
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `query_utf8` may be null; otherwise it must remain a readable,
/// NUL-terminated UTF-8 string for the duration of this call.
pub unsafe extern "C" fn tiez_core_get_snapshot_json(
    handle: *mut TiezCoreHandle,
    query_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let query = optional_utf8(query_utf8)?;
        let history = handle
            .history
            .lock()
            .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
        let snapshot = history
            .snapshot(&query)
            .map_err(|error| error.to_string())?;
        into_owned_c_string(snapshot_json(&snapshot)?)
    })
}

#[no_mangle]
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_content_json(
    handle: *mut TiezCoreHandle,
    entry_id: i64,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let history = handle
            .history
            .lock()
            .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
        let content = history
            .content(entry_id)
            .map_err(|error| error.to_string())?;
        into_owned_c_string(content_json(&content)?)
    })
}

#[no_mangle]
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `action_utf8` must remain a readable, NUL-terminated UTF-8 string for the
/// duration of this call.
pub unsafe extern "C" fn tiez_core_apply_action(
    handle: *mut TiezCoreHandle,
    entry_id: i64,
    action_utf8: *const c_char,
) -> bool {
    with_ffi_result(false, || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let action = required_utf8(action_utf8, "action_utf8")?;
        apply_history_action(handle, entry_id, &action)?;
        Ok(true)
    })
}

#[no_mangle]
/// Apply an action and return its structured outcome as newly allocated JSON.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `action_utf8` must remain a readable, NUL-terminated UTF-8 string for the
/// duration of this call.
pub unsafe extern "C" fn tiez_core_apply_action_json(
    handle: *mut TiezCoreHandle,
    entry_id: i64,
    action_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let action = required_utf8(action_utf8, "action_utf8")?;
        let mutation = apply_history_action(handle, entry_id, &action)?;
        into_owned_c_string(mutation_json(&mutation)?)
    })
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
/// # Safety
///
/// `value` must be null or a pointer returned by this library through
/// `tiez_core_get_snapshot_json`, `tiez_core_get_content_json`,
/// `tiez_core_apply_action_json`, or `tiez_core_take_last_error`. A non-null
/// pointer must be transferred to this function exactly once.
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
    use tiez_core::clipboard_history::HistoryItem;

    #[test]
    fn snapshot_serializes_the_stable_abi_and_utf8() {
        let history = ClipboardHistory::in_memory(vec![HistoryItem {
            id: -1,
            content_type: "text".to_owned(),
            preview: "line one\n\"line two\" 中文".to_owned(),
            source_app: "TieZ".to_owned(),
            captured_at: "now".to_owned(),
            is_pinned: false,
            is_sensitive: false,
        }]);
        let snapshot = history.snapshot("中文").unwrap();

        let json = snapshot_json(&snapshot).unwrap();

        assert!(json.contains("\"abi_version\":3"));
        assert!(json.contains("\"adapter\":\"memory\""));
        assert!(json.contains("\"read_only\":false"));
        assert!(json.contains("line one\\n\\\"line two\\\" 中文"));
        assert!(json.contains("\"total\":1"));
        assert!(json.contains("\"query\":\"中文\""));
    }

    #[test]
    fn content_export_serializes_full_utf8_payload() {
        let handle = Box::into_raw(Box::new(TiezCoreHandle {
            history: Mutex::new(ClipboardHistory::in_memory(vec![HistoryItem {
                id: -7,
                content_type: "text".to_owned(),
                preview: "完整内容 🚀".to_owned(),
                source_app: "TieZ".to_owned(),
                captured_at: "now".to_owned(),
                is_pinned: false,
                is_sensitive: false,
            }])),
        }));

        unsafe {
            let value = tiez_core_get_content_json(handle, -7);
            assert!(!value.is_null());
            let json = CStr::from_ptr(value).to_str().unwrap();
            assert!(json.contains("\"id\":-7"));
            assert!(json.contains("\"content\":\"完整内容 🚀\""));
            assert!(json.contains("\"available\":true"));
            tiez_core_string_free(value);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn action_export_serializes_structured_mutation_result() {
        let handle = Box::into_raw(Box::new(TiezCoreHandle {
            history: Mutex::new(ClipboardHistory::synthetic()),
        }));
        let pin = CString::new("pin").unwrap();
        let delete = CString::new("delete").unwrap();

        unsafe {
            let pin_value = tiez_core_apply_action_json(handle, 102, pin.as_ptr());
            assert!(!pin_value.is_null());
            let pin_json = CStr::from_ptr(pin_value).to_str().unwrap();
            assert!(pin_json.contains("\"abi_version\":3"));
            assert!(pin_json.contains("\"adapter\":\"memory\""));
            assert!(pin_json.contains("\"action\":\"pin\""));
            assert!(pin_json.contains("\"requested_id\":102"));
            assert!(pin_json.contains("\"effective_id\":102"));
            assert!(pin_json.contains("\"replacement_id\":null"));
            assert!(pin_json.contains("\"removed\":false"));
            assert!(pin_json.contains("\"generation\":2"));
            tiez_core_string_free(pin_value);

            let delete_value = tiez_core_apply_action_json(handle, 101, delete.as_ptr());
            assert!(!delete_value.is_null());
            let delete_json = CStr::from_ptr(delete_value).to_str().unwrap();
            assert!(delete_json.contains("\"effective_id\":null"));
            assert!(delete_json.contains("\"removed\":true"));
            assert!(delete_json.contains("\"generation\":3"));
            tiez_core_string_free(delete_value);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn invalid_action_sets_a_retrievable_error() {
        let handle = Box::into_raw(Box::new(TiezCoreHandle {
            history: Mutex::new(ClipboardHistory::synthetic()),
        }));
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
