//! Tauri-free Windows core. See native/README.md for the C ABI contract.
mod capture;
mod paste;
pub use capture::{CaptureService, CaptureStatus};
mod history;
pub use history::{HistoryQuery, HistoryService};
mod settings;
use serde::Deserialize;
use serde_json::{json, Value};
pub use settings::{CoreError, CoreResult, SettingsChange, SettingsService, SettingsSnapshot};
use std::collections::{BTreeMap, HashMap};
use std::ffi::{c_char, CString};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

const MAX_REQUEST_BYTES: usize = 64 * 1024;
fn default_true() -> bool {
    true
}
#[derive(Default)]
struct Registry {
    next_handle: u64,
    services: HashMap<u64, CoreRuntime>,
}
struct CoreRuntime {
    settings: SettingsService,
    history: Arc<Mutex<HistoryService>>,
    capture: capture::CaptureService,
    target: tiez_native_platform::target::TargetTracker,
    shell: tiez_native_platform::shell::ShellService,
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
    HistoryPage {
        handle: u64,
        query: HistoryQuery,
    },
    HistoryEntry {
        handle: u64,
        id: i64,
    },
    HistoryAdd {
        handle: u64,
        content_type: String,
        content: String,
        html_content: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
    },
    HistoryPin {
        handle: u64,
        id: i64,
        is_pinned: bool,
    },
    HistoryEdit {
        handle: u64,
        id: i64,
        content: String,
    },
    HistoryTags {
        handle: u64,
        id: i64,
        tags: Vec<String>,
    },
    HistoryDelete {
        handle: u64,
        id: i64,
    },
    HistoryChanges {
        handle: u64,
        after_revision: u64,
    },
    ClipboardCopy {
        handle: u64,
        id: i64,
        #[serde(default)]
        plain_text: bool,
        #[serde(default)]
        allow_sensitive: bool,
    },
    ClipboardPaste {
        handle: u64,
        id: i64,
        expected_generation: u64,
        #[serde(default)]
        plain_text: bool,
        #[serde(default)]
        allow_sensitive: bool,
    },
    ShellStart {
        handle: u64,
        show_tray: bool,
        #[serde(default = "default_true")]
        bind_main_hotkey: bool,
    },
    ShellPoll {
        handle: u64,
    },
    ShellStop {
        handle: u64,
    },
    TargetStart {
        handle: u64,
    },
    TargetStatus {
        handle: u64,
    },
    TargetStop {
        handle: u64,
    },
    TargetRestore {
        handle: u64,
        expected_generation: u64,
    },
    CaptureStart {
        handle: u64,
    },
    CaptureStop {
        handle: u64,
    },
    CaptureStatus {
        handle: u64,
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
            let history = Arc::new(Mutex::new(HistoryService::open(Path::new(&database_path))?));
            let service = SettingsService::open(Path::new(&database_path))?;
            let snapshot = service.snapshot()?;
            let capture = capture::CaptureService::new(
                Arc::clone(&history),
                Path::new(&database_path).parent().unwrap().to_path_buf(),
                &snapshot,
            );
            let handle = registry
                .next_handle
                .checked_add(1)
                .ok_or_else(|| CoreError::new("internal", "Core handle space exhausted"))?;
            registry.next_handle = handle;
            registry.services.insert(
                handle,
                CoreRuntime {
                    settings: service,
                    history,
                    capture,
                    target: Default::default(),
                    shell: Default::default(),
                },
            );
            Ok(json!({ "handle": handle, "snapshot": snapshot }))
        }
        Request::Snapshot { handle } => {
            let service = registry.services.get(&handle).ok_or_else(invalid_handle)?;
            Ok(json!({ "snapshot": service.settings.snapshot()? }))
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
            let shell = &service.shell;
            let (change, prepared) =
                service
                    .settings
                    .update_with(expected_revision, patch, |current, next| {
                        if current.values["app.hotkey"] != next.values["app.hotkey"]
                            && shell.status().running
                        {
                            shell
                                .prepare_hotkey(&next.values["app.hotkey"])
                                .map(Some)
                                .map_err(hotkey_error)
                        } else {
                            Ok(None)
                        }
                    })?;
            let effect_error = prepared
                .flatten()
                .and_then(|prepared| prepared.commit().err());
            service.capture.apply_settings(&change.snapshot);
            let mut result = serde_json::to_value(change).expect("serializable settings");
            result["side_effect_error"] = json!(effect_error);
            Ok(result)
        }
        Request::HistoryPage { handle, query } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .page(query)?)),
        Request::HistoryEntry { handle, id } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .entry(id)?)),
        Request::HistoryChanges {
            handle,
            after_revision,
        } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .changes(after_revision)?)),
        Request::HistoryAdd {
            handle,
            content_type,
            content,
            html_content,
            tags,
        } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .add_manual(content_type, content, html_content, tags)?)),
        Request::HistoryPin {
            handle,
            id,
            is_pinned,
        } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .pin(id, is_pinned)?)),
        Request::HistoryEdit {
            handle,
            id,
            content,
        } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .edit(id, content)?)),
        Request::HistoryTags { handle, id, tags } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .set_tags(id, tags)?)),
        Request::HistoryDelete { handle, id } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .delete(id)?)),
        Request::ClipboardCopy {
            handle,
            id,
            plain_text,
            allow_sensitive,
        } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .capture
            .copy_entry(id, plain_text, allow_sensitive)?)),
        Request::ClipboardPaste {
            handle,
            id,
            expected_generation,
            plain_text,
            allow_sensitive,
        } => {
            let service = registry.services.get(&handle).ok_or_else(invalid_handle)?;
            Ok(json!(paste::paste_entry(
                service,
                id,
                plain_text,
                allow_sensitive,
                expected_generation
            )?))
        }
        Request::ShellStart {
            handle,
            show_tray,
            bind_main_hotkey,
        } => {
            let service = registry
                .services
                .get_mut(&handle)
                .ok_or_else(invalid_handle)?;
            service.shell.start(show_tray).map_err(shell_error)?;
            if bind_main_hotkey {
                let snapshot = service.settings.snapshot()?;
                // Conflicts do not take away the tray/visible-window recovery path.
                if let Ok(prepared) = service.shell.prepare_hotkey(&snapshot.values["app.hotkey"]) {
                    let _ = prepared.commit();
                }
            }
            Ok(shell_status(service.shell.status()))
        }
        Request::ShellPoll { handle } => {
            let service = registry.services.get(&handle).ok_or_else(invalid_handle)?;
            use tiez_native_platform::shell::ShellAction;
            let actions: Vec<_> = service
                .shell
                .drain()
                .into_iter()
                .map(|action| match action {
                    ShellAction::ShowMain => "show_main",
                    ShellAction::OpenSettings => "open_settings",
                    ShellAction::Quit => "quit",
                    ShellAction::ToggleMain => "toggle_main",
                })
                .collect();
            Ok(json!({ "status": shell_status(service.shell.status()), "actions": actions }))
        }
        Request::ShellStop { handle } => {
            let service = registry
                .services
                .get_mut(&handle)
                .ok_or_else(invalid_handle)?;
            service.shell.stop().map_err(shell_error)?;
            Ok(shell_status(service.shell.status()))
        }
        Request::TargetStart { handle } => {
            let service = registry
                .services
                .get_mut(&handle)
                .ok_or_else(invalid_handle)?;
            Ok(target_status(service.target.start().map_err(target_error)?))
        }
        Request::TargetStatus { handle } => Ok(target_status(
            registry
                .services
                .get(&handle)
                .ok_or_else(invalid_handle)?
                .target
                .status(),
        )),
        Request::TargetStop { handle } => {
            let service = registry
                .services
                .get_mut(&handle)
                .ok_or_else(invalid_handle)?;
            service.target.stop().map_err(target_error)?;
            Ok(target_status(service.target.status()))
        }
        Request::TargetRestore {
            handle,
            expected_generation,
        } => {
            let service = registry.services.get(&handle).ok_or_else(invalid_handle)?;
            let ticket = service
                .target
                .ticket(expected_generation)
                .map_err(target_error)?;
            let result = service.target.restore(&ticket).map_err(target_error)?;
            Ok(json!({ "generation": result.generation, "restored": result.restored }))
        }
        Request::CaptureStart { handle } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .capture
            .start()?)),
        Request::CaptureStop { handle } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .capture
            .stop())),
        Request::CaptureStatus { handle } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .capture
            .status())),
        Request::Close { handle } => {
            let mut service = registry
                .services
                .remove(&handle)
                .ok_or_else(invalid_handle)?;
            // Removed from registry, no history/policy guards held while joining.
            let _ = service.shell.stop();
            let shell = shell_status(service.shell.status());
            let capture = service.capture.stop();
            let _ = service.target.stop();
            let target = target_status(service.target.status());
            Ok(json!({ "closed": true, "capture": capture, "target": target, "shell": shell }))
        }
    }
}
fn hotkey_error(code: &'static str) -> CoreError {
    CoreError::new(
        code,
        "Shortcut could not be prepared; previous settings and binding retained",
    )
}
fn shell_error(code: &'static str) -> CoreError {
    CoreError::new(
        code,
        "Native tray/message service unavailable; keep the main window accessible",
    )
}
fn shell_status(status: tiez_native_platform::shell::ShellStatus) -> Value {
    json!({ "running": status.running, "starts": status.starts,
        "tray_visible": status.tray_visible, "last_error": status.last_error,
        "main_hotkey_active": status.main_hotkey_active, "hotkey_error": status.hotkey_error })
}
fn target_error(code: &'static str) -> CoreError {
    CoreError::new(
        code,
        "Target window could not be verified or restored; no paste input was sent",
    )
}
fn target_status(status: tiez_native_platform::target::TargetStatus) -> Value {
    json!({ "running": status.running, "starts": status.starts, "generation": status.generation,
        "has_target": status.has_target, "application": status.application, "last_error": status.last_error })
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
        let added = call(
            json!({"op":"history_add","handle":handle,"content_type":"rich_text","content":" 中 \r\n","html_content":"<b>中</b>"}),
        );
        assert_eq!(added["ok"], true, "{added}");
        let id = added["result"]["id"].as_i64().unwrap();
        let page = call(json!({"op":"history_page","handle":handle,"query":{"limit":1}}));
        assert_eq!(page["result"]["items"][0]["id"], id);
        assert_eq!(
            call(json!({"op":"history_entry","handle":handle,"id":id}))["result"]["html_content"],
            "<b>中</b>"
        );
        assert_eq!(
            call(json!({"op":"history_pin","handle":handle,"id":id,"is_pinned":true}))["ok"],
            true
        );
        assert_eq!(
            call(json!({"op":"history_tags","handle":handle,"id":id,"tags":["密码"]}))["ok"],
            true
        );
        assert_eq!(
            call(json!({"op":"history_edit","handle":handle,"id":id,"content":" changed \n"}))
                ["ok"],
            true
        );
        assert_eq!(
            call(json!({"op":"history_changes","handle":handle,"after_revision":0}))["result"]
                ["revision"],
            4
        );

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
        assert_eq!(
            call(json!({"op":"history_entry","handle":next,"id":id}))["result"]["content"],
            " changed \n"
        );
        assert_eq!(
            call(json!({"op":"history_delete","handle":next,"id":id}))["ok"],
            true
        );
        assert_eq!(call(json!({"op":"close", "handle":next}))["ok"], true);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
