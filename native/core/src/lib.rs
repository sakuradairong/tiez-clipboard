//! Tauri-free prototype settings core. See ../README.md for the C ABI contract.
mod settings;
use serde::Deserialize;
use serde_json::{json, Value};
pub use settings::{CoreError, CoreResult, SettingsChange, SettingsService, SettingsSnapshot};
use std::collections::{BTreeMap, HashMap};
use std::ffi::{c_char, CString};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

const MAX_REQUEST_BYTES: usize = 64 * 1024;
#[derive(Default)]
struct Registry {
    next_handle: u64,
    services: HashMap<u64, SettingsService>,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Open {
        database_path: String,
    },
    Snapshot {
        handle: u64,
    },
    Update {
        handle: u64,
        expected_revision: u64,
        patch: BTreeMap<String, String>,
    },
    Close {
        handle: u64,
    },
}

fn dispatch(bytes: &[u8]) -> CoreResult<Value> {
    let request: Request = serde_json::from_slice(bytes)
        .map_err(|_| CoreError::new("invalid_request", "Expected a valid UTF-8 JSON command"))?;
    let mut registry = REGISTRY
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| CoreError::new("internal", "Core registry unavailable"))?;
    match request {
        Request::Open { database_path } => {
            if !registry.services.is_empty() {
                return Err(CoreError::new(
                    "already_open",
                    "Share the existing core between windows",
                ));
            }
            let service = SettingsService::open(Path::new(&database_path))?;
            let snapshot = service.snapshot()?;
            let handle = registry
                .next_handle
                .checked_add(1)
                .ok_or_else(|| CoreError::new("internal", "Core handle space exhausted"))?;
            registry.next_handle = handle;
            registry.services.insert(handle, service);
            Ok(json!({ "handle": handle, "snapshot": snapshot }))
        }
        Request::Snapshot { handle } => {
            let service = registry.services.get(&handle).ok_or_else(invalid_handle)?;
            Ok(json!({ "snapshot": service.snapshot()? }))
        }
        Request::Update {
            handle,
            expected_revision,
            patch,
        } => {
            let service = registry
                .services
                .get_mut(&handle)
                .ok_or_else(invalid_handle)?;
            Ok(
                serde_json::to_value(service.update(expected_revision, patch)?)
                    .expect("serializable settings"),
            )
        }
        Request::Close { handle } => {
            registry
                .services
                .remove(&handle)
                .ok_or_else(invalid_handle)?;
            Ok(json!({ "closed": true }))
        }
    }
}
fn invalid_handle() -> CoreError {
    CoreError::new("invalid_handle", "Core handle is closed or unknown")
}
fn response(result: CoreResult<Value>) -> *mut c_char {
    let value = match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(error) => json!({ "ok": false, "error": error }),
    };
    CString::new(value.to_string())
        .expect("JSON escapes NUL")
        .into_raw()
}

#[no_mangle]
pub extern "C" fn tiez_core_abi_version() -> u32 {
    1
}

/// Returns an owned NUL-terminated UTF-8 JSON response; free with tiez_core_free.
/// Calls are serialized. No native callbacks or UI thread affinity.
/// # Safety
/// For nonzero len, request must point to len readable bytes for this call.
#[no_mangle]
pub unsafe extern "C" fn tiez_core_request(request: *const u8, len: usize) -> *mut c_char {
    let result = std::panic::catch_unwind(|| {
        if request.is_null() || len == 0 || len > MAX_REQUEST_BYTES {
            return Err(CoreError::new(
                "invalid_request",
                "Request must contain 1..65536 UTF-8 bytes",
            ));
        }
        dispatch(unsafe { std::slice::from_raw_parts(request, len) })
    })
    .unwrap_or_else(|_| Err(CoreError::new("internal", "Core operation panicked")));
    response(result)
}

/// # Safety
/// buffer must be null or an unfreed pointer returned by tiez_core_request.
/// Call exactly once per non-null response, before unloading the library.
#[no_mangle]
pub unsafe extern "C" fn tiez_core_free(buffer: *mut c_char) {
    if !buffer.is_null() {
        drop(unsafe { CString::from_raw(buffer) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call(value: Value) -> Value {
        let bytes = serde_json::to_vec(&value).unwrap();
        unsafe {
            let pointer = tiez_core_request(bytes.as_ptr(), bytes.len());
            let result =
                serde_json::from_slice(std::ffi::CStr::from_ptr(pointer).to_bytes()).unwrap();
            tiez_core_free(pointer);
            result
        }
    }
    #[test]
    fn ffi_unicode_lifetime_validation_and_reopen() {
        let dir = std::env::temp_dir().join(format!("TieZ-native-接口-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("probe.db");
        assert_eq!(tiez_core_abi_version(), 1);
        let invalid = call(json!({"op":"snapshot", "handle":0}));
        assert_eq!(invalid["error"]["code"], "invalid_handle");
        unsafe {
            let pointer = tiez_core_request(std::ptr::null(), 0);
            let invalid: Value =
                serde_json::from_slice(std::ffi::CStr::from_ptr(pointer).to_bytes()).unwrap();
            assert_eq!(invalid["error"]["code"], "invalid_request");
            tiez_core_free(pointer);
            let pointer = tiez_core_request(b"x".as_ptr(), MAX_REQUEST_BYTES + 1);
            tiez_core_free(pointer);
            let pointer = tiez_core_request(b"not JSON".as_ptr(), 8);
            let invalid: Value =
                serde_json::from_slice(std::ffi::CStr::from_ptr(pointer).to_bytes()).unwrap();
            assert!(!invalid["ok"].as_bool().unwrap());
            tiez_core_free(pointer);
            tiez_core_free(std::ptr::null_mut());
        }
        let opened = call(json!({"op":"open", "database_path":path}));
        assert_eq!(opened["ok"], true, "{opened}");
        let handle = opened["result"]["handle"].as_u64().unwrap();
        assert_eq!(
            call(json!({"op":"open", "database_path":path}))["error"]["code"],
            "already_open"
        );
        let changed = call(
            json!({"op":"update","handle":handle,"expected_revision":0,"patch":{"app.language":"en"}}),
        );
        assert_eq!(changed["result"]["snapshot"]["revision"], 1);
        assert_eq!(call(json!({"op":"close", "handle":handle}))["ok"], true);
        assert_eq!(
            call(json!({"op":"close", "handle":handle}))["error"]["code"],
            "invalid_handle"
        );
        assert_eq!(
            call(json!({"op":"snapshot", "handle":handle}))["error"]["code"],
            "invalid_handle"
        );
        let reopened = call(json!({"op":"open", "database_path":path}));
        let next = reopened["result"]["handle"].as_u64().unwrap();
        assert_ne!(next, handle);
        assert_eq!(reopened["result"]["snapshot"]["revision"], 1);
        assert_eq!(call(json!({"op":"close", "handle":next}))["ok"], true);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
