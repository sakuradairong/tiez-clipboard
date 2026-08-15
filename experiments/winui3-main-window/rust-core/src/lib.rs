//! C ABI transport adapter for the WinUI 3 migration slice.
//!
//! Clipboard history behavior lives in the Tauri-independent `tiez-core`
//! crate. This library only owns environment selection, panic containment,
//! UTF-8/C string ownership, and the stable ABI consumed by C++/WinRT.

use serde::Serialize;
use std::cell::RefCell;
use std::env;
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, Mutex};
use tiez_core::backup::{
    apply_pending_restore, create_backup, inspect_backup, schedule_backup_restore, BackupInfo,
};
use tiez_core::clipboard_capture::{
    classify_snapshot, detect_content_type, CaptureFilter, CapturedPayload, ClipboardSnapshot,
};
use tiez_core::clipboard_history::{
    ClipboardHistory, HistoryContent, HistoryMutationResult, HistorySnapshot, PinnedOrderResult,
};
use tiez_core::cloud_sync_settings::{
    CloudSyncProbeResult, CloudSyncSettings, CloudSyncSettingsMutation, CloudSyncSettingsSnapshot,
    CloudSyncSettingsUpdate,
};
use tiez_core::content_opening::{prepare_open_content, OpenContentPlan};
use tiez_core::data_directory::resolve_data_directory;
use tiez_core::database_bootstrap::open_database_with_decrypt;
use tiez_core::image_analysis::{
    analyze_image_entry_from_database, get_image_analysis_from_database, ImageAnalysisResult,
};
use tiez_core::native_settings::{NativeSettingMutation, NativeSettings, NativeSettingsSnapshot};
use tiez_core::paste_coordinator::{plan_paste, PasteFormat, PastePayload};
use tiez_core::runtime_instance::DatabaseInstanceGuard;

mod win32_capture;
#[cfg(windows)]
mod win32_paste;

const ABI_VERSION: u32 = 11;
const DATABASE_ENV: &str = "TIEZ_WINUI_DB_PATH";
const DATABASE_READ_ONLY_ENV: &str = "TIEZ_WINUI_DB_READ_ONLY";
const PRODUCTION_DATA_ENV: &str = "TIEZ_WINUI_USE_PRODUCTION_DATA";
const SYNTHETIC_DATA_ENV: &str = "TIEZ_WINUI_USE_SYNTHETIC_DATA";
static DATABASE_INSTANCE_GUARD: Mutex<Option<DatabaseInstanceGuard>> = Mutex::new(None);

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy)]
struct ChangedSink {
    callback: extern "C" fn(*mut c_void, u64),
    user_data: usize,
}

unsafe impl Send for ChangedSink {}
unsafe impl Sync for ChangedSink {}

pub(crate) struct TiezCoreInner {
    history: Mutex<ClipboardHistory>,
    settings: Mutex<NativeSettings>,
    capture: Mutex<CaptureFilter>,
    changed: Mutex<Option<ChangedSink>>,
}

#[repr(C)]
pub struct TiezCoreHandle {
    inner: Arc<TiezCoreInner>,
    cloud_settings: Mutex<CloudSyncSettings>,
    session: Mutex<Option<win32_capture::Session>>,
}

impl TiezCoreHandle {
    #[cfg_attr(not(test), allow(dead_code))]
    fn wrap(history: ClipboardHistory) -> Self {
        Self::wrap_with_settings(history, NativeSettings::in_memory())
    }

    fn wrap_with_settings(history: ClipboardHistory, settings: NativeSettings) -> Self {
        Self::wrap_with_adapters(history, settings, CloudSyncSettings::in_memory())
    }

    fn wrap_with_adapters(
        history: ClipboardHistory,
        settings: NativeSettings,
        cloud_settings: CloudSyncSettings,
    ) -> Self {
        Self {
            inner: Arc::new(TiezCoreInner {
                history: Mutex::new(history),
                settings: Mutex::new(settings),
                capture: Mutex::new(CaptureFilter::new()),
                changed: Mutex::new(None),
            }),
            cloud_settings: Mutex::new(cloud_settings),
            session: Mutex::new(None),
        }
    }

    fn new_from_environment() -> Result<Self, String> {
        let use_production_data = env_flag(PRODUCTION_DATA_ENV)
            || (cfg!(feature = "production-default") && !env_flag(SYNTHETIC_DATA_ENV));
        let configured_database = match env::var_os(DATABASE_ENV) {
            Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
            _ if use_production_data => Some(production_database_path()?),
            _ => None,
        };
        let (history, settings, cloud_settings) = match configured_database {
            Some(value) => {
                let read_only = env_flag(DATABASE_READ_ONLY_ENV);
                ensure_database_instance_guard(&value)?;
                if !read_only {
                    prepare_writable_database(&value)?;
                }
                let history = ClipboardHistory::open_sqlite(&value, read_only)
                    .map_err(|error| format!("{}: {error}", value.display()))?;
                let settings = NativeSettings::open_sqlite(&value, read_only)
                    .map_err(|error| format!("{}: {error}", value.display()))?;
                let cloud_settings = CloudSyncSettings::open_sqlite(&value, read_only)
                    .map_err(|error| format!("{}: {error}", value.display()))?;
                (history, settings, cloud_settings)
            }
            _ => (
                ClipboardHistory::synthetic(),
                NativeSettings::in_memory(),
                CloudSyncSettings::in_memory(),
            ),
        };

        Ok(Self::wrap_with_adapters(history, settings, cloud_settings))
    }
}

fn production_database_path() -> Result<PathBuf, String> {
    let roaming_app_data = env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{PRODUCTION_DATA_ENV}: APPDATA is unavailable"))?;
    let default_app_dir = PathBuf::from(roaming_app_data).join("com.tiez");
    let executable_path = env::current_exe().ok();
    let data_dir = resolve_data_directory(&default_app_dir, executable_path.as_deref());
    Ok(data_dir.path.join("clipboard.db"))
}

fn prepare_writable_database(database_path: &Path) -> Result<(), String> {
    let data_dir = database_path
        .parent()
        .ok_or_else(|| format!("数据库路径缺少父目录：{}", database_path.display()))?;
    std::fs::create_dir_all(data_dir)
        .map_err(|error| format!("无法创建数据目录 {}：{error}", data_dir.display()))?;
    let restore =
        apply_pending_restore(data_dir).map_err(|error| format!("无法应用待恢复备份：{error}"))?;
    if restore.applied || restore.quarantined {
        eprintln!(">>> [RESTORE] {}", restore.message);
    }
    open_database_with_decrypt(database_path, tiez_core::encryption::decrypt_value)
        .map(|_| ())
        .map_err(|error| format!("无法初始化数据库 {}：{error}", database_path.display()))
}

fn ensure_database_instance_guard(database_path: &Path) -> Result<(), String> {
    let mut guard = DATABASE_INSTANCE_GUARD
        .lock()
        .map_err(|_| "database instance guard lock is poisoned".to_owned())?;
    if let Some(existing) = guard.as_ref() {
        return if existing.protects(database_path) {
            Ok(())
        } else {
            Err(format!(
                "the WinUI runtime already owns {}",
                existing.database_path().display()
            ))
        };
    }

    *guard = Some(
        DatabaseInstanceGuard::acquire(database_path)
            .map_err(|error| format!("{DATABASE_ENV}: {error}"))?,
    );
    Ok(())
}

fn env_flag(name: &str) -> bool {
    matches!(
        env::var(name).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE")
    )
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

#[derive(Serialize)]
struct PinnedOrderResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    result: &'a PinnedOrderResult,
}

#[derive(Serialize)]
struct SettingsResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a NativeSettingsSnapshot,
}

#[derive(Serialize)]
struct SettingMutationResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    mutation: &'a NativeSettingMutation,
}

#[derive(Serialize)]
struct CloudSyncSettingsResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a CloudSyncSettingsSnapshot,
}

#[derive(Serialize)]
struct CloudSyncSettingsMutationResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    mutation: &'a CloudSyncSettingsMutation,
}

#[derive(Serialize)]
struct CloudSyncProbeResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    result: &'a CloudSyncProbeResult,
}

#[derive(Serialize)]
struct OpenContentResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    plan: &'a OpenContentPlan,
}

#[derive(Serialize)]
struct BackupResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    info: &'a BackupInfo,
}

#[derive(Serialize)]
struct ImageAnalysisResponse<'a> {
    abi_version: u32,
    analysis: Option<&'a ImageAnalysisResult>,
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

fn pinned_order_json(result: &PinnedOrderResult) -> Result<String, String> {
    serde_json::to_string(&PinnedOrderResponse {
        abi_version: ABI_VERSION,
        result,
    })
    .map_err(|error| format!("failed to serialize pinned order result: {error}"))
}

fn settings_json(snapshot: &NativeSettingsSnapshot) -> Result<String, String> {
    serde_json::to_string(&SettingsResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("failed to serialize native settings: {error}"))
}

fn setting_mutation_json(mutation: &NativeSettingMutation) -> Result<String, String> {
    serde_json::to_string(&SettingMutationResponse {
        abi_version: ABI_VERSION,
        mutation,
    })
    .map_err(|error| format!("failed to serialize native setting mutation: {error}"))
}

fn cloud_sync_settings_json(snapshot: &CloudSyncSettingsSnapshot) -> Result<String, String> {
    serde_json::to_string(&CloudSyncSettingsResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("failed to serialize cloud-sync settings: {error}"))
}

fn cloud_sync_settings_mutation_json(
    mutation: &CloudSyncSettingsMutation,
) -> Result<String, String> {
    serde_json::to_string(&CloudSyncSettingsMutationResponse {
        abi_version: ABI_VERSION,
        mutation,
    })
    .map_err(|error| format!("failed to serialize cloud-sync settings mutation: {error}"))
}

fn cloud_sync_probe_json(result: &CloudSyncProbeResult) -> Result<String, String> {
    serde_json::to_string(&CloudSyncProbeResponse {
        abi_version: ABI_VERSION,
        result,
    })
    .map_err(|error| format!("failed to serialize cloud-sync probe result: {error}"))
}

fn open_content_json(plan: &OpenContentPlan) -> Result<String, String> {
    serde_json::to_string(&OpenContentResponse {
        abi_version: ABI_VERSION,
        plan,
    })
    .map_err(|error| format!("failed to serialize open-content plan: {error}"))
}

fn backup_json(info: &BackupInfo) -> Result<String, String> {
    serde_json::to_string(&BackupResponse {
        abi_version: ABI_VERSION,
        info,
    })
    .map_err(|error| format!("failed to serialize backup information: {error}"))
}

fn image_analysis_json(analysis: Option<&ImageAnalysisResult>) -> Result<String, String> {
    serde_json::to_string(&ImageAnalysisResponse {
        abi_version: ABI_VERSION,
        analysis,
    })
    .map_err(|error| format!("failed to serialize image analysis: {error}"))
}

fn owned_database_context(handle: &TiezCoreHandle) -> Result<(PathBuf, bool), String> {
    let database_path = {
        let guard = DATABASE_INSTANCE_GUARD
            .lock()
            .map_err(|_| "database instance guard lock is poisoned".to_owned())?;
        guard
            .as_ref()
            .map(|guard| guard.database_path().to_path_buf())
            .ok_or_else(|| "备份功能仅在 WinUI 生产数据模式下可用".to_owned())?
    };
    let read_only = handle
        .inner
        .history
        .lock()
        .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?
        .snapshot("")
        .map_err(|error| error.to_string())?
        .read_only;
    Ok((database_path, read_only))
}

fn prepare_history_open(handle: &TiezCoreHandle, entry_id: i64) -> Result<OpenContentPlan, String> {
    let content = {
        let history = handle
            .inner
            .history
            .lock()
            .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
        history
            .content(entry_id)
            .map_err(|error| error.to_string())?
    };
    let temporary_root = std::env::temp_dir().join("TieZ").join("open");
    prepare_open_content(&content, &temporary_root).map_err(|error| error.to_string())
}

fn apply_history_action(
    handle: &TiezCoreHandle,
    entry_id: i64,
    action: &str,
) -> Result<HistoryMutationResult, String> {
    let mut history = handle
        .inner
        .history
        .lock()
        .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;

    if matches!(
        action,
        "paste-plain" | "paste-rich" | "copy-plain" | "copy-rich"
    ) {
        let format = if action.ends_with("rich") {
            PasteFormat::Rich
        } else {
            PasteFormat::Plain
        };
        let content = history
            .content(entry_id)
            .map_err(|error| error.to_string())?;
        let mut plan = plan_paste(&content, format, false).map_err(|error| error.to_string())?;
        let copied = action.starts_with("copy-");
        if copied {
            plan = plan.into_clipboard_only();
        }
        execute_os_paste(&plan)?;
        if let Ok(mut filter) = handle.inner.capture.lock() {
            if let Some(payload) = payload_written_to_clipboard(&plan) {
                filter.note_payload(&payload);
            }
        }
        let mut mutation = history
            .apply_action(entry_id, action)
            .map_err(|error| error.to_string())?;
        mutation.message = format!(
            "{} item {entry_id} as {} ({})",
            if copied { "Copied" } else { "Pasted" },
            plan.payload.format.as_str(),
            paste_payload_summary(&plan.payload)
        );
        return Ok(mutation);
    }

    history
        .apply_action(entry_id, action)
        .map_err(|error| error.to_string())
}

fn update_history_tags(
    handle: &TiezCoreHandle,
    entry_id: i64,
    tags: Vec<String>,
) -> Result<HistoryMutationResult, String> {
    let mutation = {
        let mut history = handle
            .inner
            .history
            .lock()
            .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
        history
            .update_tags(entry_id, tags)
            .map_err(|error| error.to_string())?
    };
    notify_changed(&handle.inner, mutation.generation);
    Ok(mutation)
}

fn reorder_history_pins(
    handle: &TiezCoreHandle,
    ordered_ids: Vec<i64>,
) -> Result<PinnedOrderResult, String> {
    let result = {
        let mut history = handle
            .inner
            .history
            .lock()
            .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
        history
            .reorder_pinned(ordered_ids)
            .map_err(|error| error.to_string())?
    };
    notify_changed(&handle.inner, result.generation);
    Ok(result)
}

fn native_settings_snapshot(handle: &TiezCoreHandle) -> Result<NativeSettingsSnapshot, String> {
    handle
        .inner
        .settings
        .lock()
        .map_err(|_| "NativeSettings lock is poisoned".to_owned())?
        .snapshot()
        .map_err(|error| error.to_string())
}

fn update_native_setting(
    handle: &TiezCoreHandle,
    key: &str,
    value: &str,
) -> Result<NativeSettingMutation, String> {
    handle
        .inner
        .settings
        .lock()
        .map_err(|_| "NativeSettings lock is poisoned".to_owned())?
        .update(key, value)
        .map_err(|error| error.to_string())
}

fn cloud_sync_settings_snapshot(
    handle: &TiezCoreHandle,
) -> Result<CloudSyncSettingsSnapshot, String> {
    handle
        .cloud_settings
        .lock()
        .map_err(|_| "CloudSyncSettings lock is poisoned".to_owned())?
        .snapshot()
        .map_err(|error| error.to_string())
}

fn update_cloud_sync_settings(
    handle: &TiezCoreHandle,
    update: CloudSyncSettingsUpdate,
) -> Result<CloudSyncSettingsMutation, String> {
    handle
        .cloud_settings
        .lock()
        .map_err(|_| "CloudSyncSettings lock is poisoned".to_owned())?
        .update(update)
        .map_err(|error| error.to_string())
}

fn probe_cloud_sync(handle: &TiezCoreHandle) -> Result<CloudSyncProbeResult, String> {
    handle
        .cloud_settings
        .lock()
        .map_err(|_| "CloudSyncSettings lock is poisoned".to_owned())?
        .probe_webdav()
        .map_err(|error| error.to_string())
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn ingest_captured_text(
    inner: &TiezCoreInner,
    raw: &str,
) -> Result<Option<u64>, String> {
    ingest_captured_snapshot(
        inner,
        ClipboardSnapshot {
            text: Some(raw.to_owned()),
            ..ClipboardSnapshot::default()
        },
    )
}

pub(crate) fn ingest_captured_snapshot(
    inner: &TiezCoreInner,
    snapshot: ClipboardSnapshot,
) -> Result<Option<u64>, String> {
    let Some(mut payload) = classify_snapshot(snapshot) else {
        return Ok(None);
    };
    let capture_preferences = inner
        .settings
        .lock()
        .map_err(|_| "NativeSettings lock is poisoned".to_owned())?
        .capture_preferences()
        .map_err(|error| error.to_string())?;
    if matches!(payload, CapturedPayload::Files { .. }) && !capture_preferences.capture_files {
        return Ok(None);
    }
    if !capture_preferences.capture_rich_text {
        if let CapturedPayload::RichText { content, .. } = payload {
            let content_type = detect_content_type(&content);
            payload = CapturedPayload::Text {
                content,
                content_type,
            };
        }
    }
    let accepted = {
        let mut filter = inner
            .capture
            .lock()
            .map_err(|_| "CaptureFilter lock is poisoned".to_owned())?;
        match filter.accept_payload_with_dedup(Some(payload), capture_preferences.deduplicate) {
            Ok(payload) => payload,
            Err(_) => return Ok(None),
        }
    };
    let mutation = {
        let mut history = inner
            .history
            .lock()
            .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
        history
            .ingest(accepted, "Clipboard")
            .map_err(|error| error.to_string())?
    };
    notify_changed(inner, mutation.generation);
    Ok(Some(mutation.generation))
}

fn payload_written_to_clipboard(
    plan: &tiez_core::paste_coordinator::PastePlan,
) -> Option<CapturedPayload> {
    #[cfg(all(windows, not(test)))]
    if let Some(snapshot) = win32_capture::read_clipboard_snapshot() {
        return classify_snapshot(snapshot);
    }

    payload_from_paste_plan(plan)
}

fn payload_from_paste_plan(
    plan: &tiez_core::paste_coordinator::PastePlan,
) -> Option<CapturedPayload> {
    if !plan.payload.files.is_empty() {
        return Some(CapturedPayload::Files {
            paths: plan.payload.files.clone(),
        });
    }
    if let Some(image) = &plan.payload.image {
        return Some(CapturedPayload::Image {
            content: image.clone(),
        });
    }
    classify_snapshot(ClipboardSnapshot {
        text: Some(plan.payload.text.clone()),
        html: plan.payload.html.clone(),
        ..ClipboardSnapshot::default()
    })
}

fn notify_changed(inner: &TiezCoreInner, generation: u64) {
    let sink = inner.changed.lock().ok().and_then(|guard| *guard);
    if let Some(sink) = sink {
        (sink.callback)(sink.user_data as *mut c_void, generation);
    }
}

fn start_capture(handle: &TiezCoreHandle) -> Result<(), String> {
    let mut session = handle
        .session
        .lock()
        .map_err(|_| "capture session lock is poisoned".to_owned())?;
    if session.is_some() {
        return Ok(());
    }
    *session = Some(win32_capture::start(Arc::clone(&handle.inner))?);
    Ok(())
}

fn paste_payload_summary(payload: &PastePayload) -> String {
    if let Some(image) = &payload.image {
        format!("image {image}")
    } else if !payload.files.is_empty() {
        format!("{} files", payload.files.len())
    } else {
        format!(
            "{} chars{}",
            payload.text.chars().count(),
            if payload.html.is_some() {
                " + HTML"
            } else {
                ""
            }
        )
    }
}

fn execute_os_paste(plan: &tiez_core::paste_coordinator::PastePlan) -> Result<(), String> {
    #[cfg(all(windows, not(test)))]
    {
        crate::win32_paste::execute(plan)
    }
    #[cfg(not(all(windows, not(test))))]
    {
        use tiez_core::paste_coordinator::{execute_paste, RecordingPasteExecutor};
        let mut executor = RecordingPasteExecutor::default();
        execute_paste(plan, &mut executor)
    }
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
        // transferred back exactly once by this function. Drop the listener
        // before the inner Arc so the worker can finish using it.
        let boxed = unsafe { Box::from_raw(handle) };
        if let Ok(mut session) = boxed.session.lock() {
            session.take();
        }
        drop(boxed);
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
            .inner
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
            .inner
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
/// Return cached OCR/QR analysis for a production image entry, if available.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_image_analysis_json(
    handle: *mut TiezCoreHandle,
    entry_id: i64,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let (database_path, _) = owned_database_context(handle)?;
        let analysis = get_image_analysis_from_database(&database_path, entry_id)
            .map_err(|error| error.to_string())?;
        into_owned_c_string(image_analysis_json(analysis.as_ref())?)
    })
}

#[no_mangle]
/// Analyze one production image entry with Windows OCR and QR decoding.
/// Read-only and sensitive entries are analyzed in memory without persisting
/// recognized plaintext.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_analyze_image_json(
    handle: *mut TiezCoreHandle,
    entry_id: i64,
    force: bool,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let (database_path, read_only) = owned_database_context(handle)?;
        let analysis =
            analyze_image_entry_from_database(&database_path, entry_id, force, read_only)
                .map_err(|error| error.to_string())?;
        into_owned_c_string(image_analysis_json(Some(&analysis))?)
    })
}

#[no_mangle]
/// Resolve one entry into a validated URL or local-file launch plan.
///
/// The returned UTF-8 JSON string is newly allocated and must be released with
/// `tiez_core_string_free`. Sensitive or unavailable entries are rejected.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_prepare_open_content_json(
    handle: *mut TiezCoreHandle,
    entry_id: i64,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let plan = prepare_history_open(handle, entry_id)?;
        into_owned_c_string(open_content_json(&plan)?)
    })
}

#[no_mangle]
/// Create a validated TieZ backup and return its metadata as allocated JSON.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `destination_utf8` must remain a readable, NUL-terminated UTF-8 path for
/// the duration of this call.
pub unsafe extern "C" fn tiez_core_create_backup_json(
    handle: *mut TiezCoreHandle,
    destination_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let destination = PathBuf::from(required_utf8(destination_utf8, "destination_utf8")?);
        let (database_path, _) = owned_database_context(handle)?;
        let data_dir = database_path
            .parent()
            .ok_or_else(|| format!("数据库路径缺少父目录：{}", database_path.display()))?;
        let info = create_backup(
            &database_path,
            data_dir,
            &destination,
            env!("CARGO_PKG_VERSION"),
        )
        .map_err(|error| error.to_string())?;
        into_owned_c_string(backup_json(&info)?)
    })
}

#[no_mangle]
/// Validate a TieZ backup and return its metadata as allocated JSON.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `path_utf8` must remain a readable, NUL-terminated UTF-8 path for this call.
pub unsafe extern "C" fn tiez_core_inspect_backup_json(
    handle: *mut TiezCoreHandle,
    path_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let _handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let path = PathBuf::from(required_utf8(path_utf8, "path_utf8")?);
        let info = inspect_backup(&path).map_err(|error| error.to_string())?;
        into_owned_c_string(backup_json(&info)?)
    })
}

#[no_mangle]
/// Validate and stage a TieZ backup for restore before the next database open.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `path_utf8` must remain a readable, NUL-terminated UTF-8 path for this call.
pub unsafe extern "C" fn tiez_core_schedule_restore_json(
    handle: *mut TiezCoreHandle,
    path_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let path = PathBuf::from(required_utf8(path_utf8, "path_utf8")?);
        let (database_path, read_only) = owned_database_context(handle)?;
        if read_only {
            return Err("只读生产数据模式不能安排恢复".to_owned());
        }
        let data_dir = database_path
            .parent()
            .ok_or_else(|| format!("数据库路径缺少父目录：{}", database_path.display()))?;
        let info = schedule_backup_restore(data_dir, &path).map_err(|error| error.to_string())?;
        into_owned_c_string(backup_json(&info)?)
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
/// Replace an entry's tags and return its structured mutation outcome as JSON.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `tags_json_utf8` must be a readable, NUL-terminated UTF-8 JSON string
/// containing an array of strings for the duration of this call.
pub unsafe extern "C" fn tiez_core_update_tags_json(
    handle: *mut TiezCoreHandle,
    entry_id: i64,
    tags_json_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let tags_json = required_utf8(tags_json_utf8, "tags_json_utf8")?;
        let tags = serde_json::from_str::<Vec<String>>(&tags_json)
            .map_err(|error| format!("tags_json_utf8 must be a JSON string array: {error}"))?;
        let mutation = update_history_tags(handle, entry_id, tags)?;
        into_owned_c_string(mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Replace the complete top-to-bottom order of pinned entry IDs.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `ordered_ids_json_utf8` must be a readable, NUL-terminated UTF-8 JSON
/// string containing an array of positive integer IDs for this call.
pub unsafe extern "C" fn tiez_core_update_pinned_order_json(
    handle: *mut TiezCoreHandle,
    ordered_ids_json_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let ordered_ids_json = required_utf8(ordered_ids_json_utf8, "ordered_ids_json_utf8")?;
        let ordered_ids = serde_json::from_str::<Vec<i64>>(&ordered_ids_json).map_err(|error| {
            format!("ordered_ids_json_utf8 must be a JSON integer array: {error}")
        })?;
        let result = reorder_history_pins(handle, ordered_ids)?;
        into_owned_c_string(pinned_order_json(&result)?)
    })
}

#[no_mangle]
/// Return the allowlisted native settings as a newly allocated UTF-8 JSON string.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_settings_json(handle: *mut TiezCoreHandle) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let snapshot = native_settings_snapshot(handle)?;
        into_owned_c_string(settings_json(&snapshot)?)
    })
}

#[no_mangle]
/// Update one allowlisted native setting and return its structured result.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `key_utf8` and `value_utf8` must be readable, NUL-terminated UTF-8 strings.
pub unsafe extern "C" fn tiez_core_update_setting_json(
    handle: *mut TiezCoreHandle,
    key_utf8: *const c_char,
    value_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let key = required_utf8(key_utf8, "key_utf8")?;
        let value = required_utf8(value_utf8, "value_utf8")?;
        let mutation = update_native_setting(handle, &key, &value)?;
        into_owned_c_string(setting_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Return cloud-sync configuration without exposing stored credentials.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_cloud_sync_settings_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let snapshot = cloud_sync_settings_snapshot(handle)?;
        into_owned_c_string(cloud_sync_settings_json(&snapshot)?)
    })
}

#[no_mangle]
/// Transactionally update WebDAV settings. Passwords are write-only.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `request_json_utf8` must be a readable, NUL-terminated UTF-8 JSON object.
pub unsafe extern "C" fn tiez_core_update_cloud_sync_settings_json(
    handle: *mut TiezCoreHandle,
    request_json_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let request_json = required_utf8(request_json_utf8, "request_json_utf8")?;
        let update = serde_json::from_str::<CloudSyncSettingsUpdate>(&request_json)
            .map_err(|error| format!("request_json_utf8 must be cloud settings JSON: {error}"))?;
        let mutation = update_cloud_sync_settings(handle, update)?;
        into_owned_c_string(cloud_sync_settings_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Probe the configured WebDAV endpoint with a read-only `PROPFIND` request.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_probe_cloud_sync_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let result = probe_cloud_sync(handle)?;
        into_owned_c_string(cloud_sync_probe_json(&result)?)
    })
}

#[no_mangle]
/// Register a history-changed callback. Pass a null callback to clear it.
///
/// The callback may run on a background clipboard worker thread. The C++
/// host must marshal UI work onto its dispatcher.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `user_data` must remain valid until the callback is cleared or the handle
/// is destroyed.
pub unsafe extern "C" fn tiez_core_set_changed_callback(
    handle: *mut TiezCoreHandle,
    callback: Option<extern "C" fn(*mut c_void, u64)>,
    user_data: *mut c_void,
) {
    catch_ffi((), || {
        let handle = match unsafe { handle.as_ref() } {
            Some(handle) => handle,
            None => {
                set_last_error("handle must not be null");
                return;
            }
        };
        match handle.inner.changed.lock() {
            Ok(mut slot) => {
                *slot = callback.map(|callback| ChangedSink {
                    callback,
                    user_data: user_data as usize,
                });
            }
            Err(_) => set_last_error("changed-callback lock is poisoned"),
        }
    });
}

#[no_mangle]
/// Start live Unicode clipboard capture. Startup content is primed, not ingested.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_start_capture(handle: *mut TiezCoreHandle) -> bool {
    with_ffi_result(false, || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        start_capture(handle)?;
        Ok(true)
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
/// `tiez_core_get_image_analysis_json`, `tiez_core_analyze_image_json`,
/// `tiez_core_prepare_open_content_json`,
/// `tiez_core_create_backup_json`, `tiez_core_inspect_backup_json`,
/// `tiez_core_schedule_restore_json`, `tiez_core_apply_action_json`,
/// `tiez_core_update_tags_json`, or
/// `tiez_core_update_pinned_order_json`, `tiez_core_get_settings_json`,
/// `tiez_core_update_setting_json`, `tiez_core_get_cloud_sync_settings_json`,
/// `tiez_core_update_cloud_sync_settings_json`,
/// `tiez_core_probe_cloud_sync_json`, or `tiez_core_take_last_error`. A
/// non-null pointer must be transferred to this function exactly once.
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
    use std::time::{SystemTime, UNIX_EPOCH};
    use tiez_core::clipboard_history::HistoryItem;

    fn temporary_database_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("tiez-winui-bootstrap-{name}-{unique}"))
    }

    #[test]
    fn writable_database_bootstrap_creates_a_usable_production_schema() {
        let root = temporary_database_root("new");
        let database_path = root.join("clipboard.db");

        prepare_writable_database(&database_path).unwrap();
        let history = ClipboardHistory::open_sqlite_read_write(&database_path).unwrap();
        assert_eq!(history.snapshot("").unwrap().adapter, "sqlite");

        drop(history);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writable_database_bootstrap_quarantines_an_invalid_pending_restore() {
        let root = temporary_database_root("pending-restore");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(tiez_core::backup::PENDING_BACKUP_NAME),
            b"pending",
        )
        .unwrap();
        let database_path = root.join("clipboard.db");

        prepare_writable_database(&database_path).unwrap();

        assert!(database_path.exists());
        assert!(!root.join(tiez_core::backup::PENDING_BACKUP_NAME).exists());
        assert!(std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("restore-failed-")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn snapshot_serializes_the_stable_abi_and_utf8() {
        let history = ClipboardHistory::in_memory(vec![HistoryItem {
            id: -1,
            content_type: "text".to_owned(),
            preview: "line one\n\"line two\" 中文".to_owned(),
            source_app: "TieZ".to_owned(),
            captured_at: "now".to_owned(),
            is_pinned: false,
            tags: vec!["中文".to_owned()],
            is_sensitive: false,
        }]);
        let snapshot = history.snapshot("中文").unwrap();

        let json = snapshot_json(&snapshot).unwrap();

        assert!(json.contains("\"abi_version\":11"));
        assert!(json.contains("\"adapter\":\"memory\""));
        assert!(json.contains("\"read_only\":false"));
        assert!(json.contains("line one\\n\\\"line two\\\" 中文"));
        assert!(json.contains("\"total\":1"));
        assert!(json.contains("\"query\":\"中文\""));
    }

    #[test]
    fn content_export_serializes_full_utf8_payload() {
        let handle = Box::into_raw(Box::new(TiezCoreHandle::wrap(ClipboardHistory::in_memory(
            vec![HistoryItem {
                id: -7,
                content_type: "text".to_owned(),
                preview: "完整内容 🚀".to_owned(),
                source_app: "TieZ".to_owned(),
                captured_at: "now".to_owned(),
                is_pinned: false,
                tags: Vec::new(),
                is_sensitive: false,
            }],
        ))));

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
    fn image_analysis_export_keeps_abi_and_camel_case_contract() {
        let analysis = ImageAnalysisResult {
            text: "识别文字".to_owned(),
            qr_codes: vec!["https://example.com/pay".to_owned()],
            language: Some("zh-CN".to_owned()),
            analyzed_at: 42,
            cached: false,
            persisted: true,
            ocr_available: true,
            ocr_error: None,
        };

        let json = image_analysis_json(Some(&analysis)).unwrap();
        let empty = image_analysis_json(None).unwrap();

        assert!(json.contains("\"abi_version\":11"));
        assert!(json.contains("\"analysis\":{\"text\":\"识别文字\""));
        assert!(json.contains("\"qrCodes\":[\"https://example.com/pay\"]"));
        assert!(json.contains("\"analyzedAt\":42"));
        assert!(json.contains("\"ocrAvailable\":true"));
        assert!(empty.contains("\"analysis\":null"));
    }

    #[test]
    fn open_content_export_returns_a_validated_plan_and_rejects_sensitive_entries() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));

        unsafe {
            let value = tiez_core_prepare_open_content_json(handle, 103);
            assert!(!value.is_null());
            let json = CStr::from_ptr(value).to_str().unwrap();
            assert!(json.contains("\"abi_version\":11"));
            assert!(json.contains("\"kind\":\"url\""));
            assert!(json.contains("https://github.com/jimuzhe/tiez-clipboard/issues/154"));
            assert!(json.contains("\"requires_confirmation\":false"));
            tiez_core_string_free(value);

            assert!(tiez_core_prepare_open_content_json(handle, 107).is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("受隐私保护"));
            tiez_core_string_free(error);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn backup_exports_round_trip_production_data_and_startup_restore() {
        let root = temporary_database_root("backup-round-trip");
        let data_dir = root.join("data");
        let database_path = data_dir.join("clipboard.db");
        let destination = root.join("daily.tiez-backup");
        prepare_writable_database(&database_path).unwrap();
        ensure_database_instance_guard(&database_path).unwrap();
        let history = ClipboardHistory::open_sqlite_read_write(&database_path).unwrap();
        let settings = NativeSettings::open_sqlite(&database_path, false).unwrap();
        let handle = Box::into_raw(Box::new(TiezCoreHandle::wrap_with_settings(
            history, settings,
        )));
        let destination = CString::new(destination.to_string_lossy().as_bytes()).unwrap();

        unsafe {
            let created = tiez_core_create_backup_json(handle, destination.as_ptr());
            assert!(!created.is_null());
            let created_json = CStr::from_ptr(created).to_str().unwrap();
            assert!(created_json.contains("\"abi_version\":11"));
            assert!(created_json.contains("\"entryCount\":0"));
            tiez_core_string_free(created);

            let inspected = tiez_core_inspect_backup_json(handle, destination.as_ptr());
            assert!(!inspected.is_null());
            assert!(CStr::from_ptr(inspected)
                .to_str()
                .unwrap()
                .contains("\"fileCount\":1"));
            tiez_core_string_free(inspected);

            let scheduled = tiez_core_schedule_restore_json(handle, destination.as_ptr());
            assert!(!scheduled.is_null());
            assert!(CStr::from_ptr(scheduled)
                .to_str()
                .unwrap()
                .contains("\"formatVersion\":1"));
            tiez_core_string_free(scheduled);
            tiez_core_destroy(handle);
        }

        DATABASE_INSTANCE_GUARD.lock().unwrap().take();
        assert!(data_dir
            .join(tiez_core::backup::PENDING_BACKUP_NAME)
            .is_file());
        prepare_writable_database(&database_path).unwrap();
        assert!(!data_dir
            .join(tiez_core::backup::PENDING_BACKUP_NAME)
            .exists());
        assert!(std::fs::read_dir(&data_dir)
            .unwrap()
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("restore-rollback-")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn action_export_serializes_structured_mutation_result() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let pin = CString::new("pin").unwrap();
        let delete = CString::new("delete").unwrap();

        unsafe {
            let pin_value = tiez_core_apply_action_json(handle, 102, pin.as_ptr());
            assert!(!pin_value.is_null());
            let pin_json = CStr::from_ptr(pin_value).to_str().unwrap();
            assert!(pin_json.contains("\"abi_version\":11"));
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
    fn tag_export_accepts_utf8_json_and_returns_structured_mutation() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let tags = CString::new("[\"工作\",\"密码\",\"工作\"]").unwrap();

        unsafe {
            let value = tiez_core_update_tags_json(handle, 102, tags.as_ptr());
            assert!(!value.is_null());
            let json = CStr::from_ptr(value).to_str().unwrap();
            assert!(json.contains("\"abi_version\":11"));
            assert!(json.contains("\"action\":\"update-tags\""));
            assert!(json.contains("\"requested_id\":102"));
            assert!(json.contains("\"effective_id\":102"));
            assert!(json.contains("\"replacement_id\":null"));
            assert!(json.contains("\"generation\":2"));
            tiez_core_string_free(value);

            let query = CString::new("工作").unwrap();
            let snapshot = tiez_core_get_snapshot_json(handle, query.as_ptr());
            assert!(!snapshot.is_null());
            let snapshot_json = CStr::from_ptr(snapshot).to_str().unwrap();
            assert!(snapshot_json.contains("\"tags\":[\"工作\",\"密码\"]"));
            assert!(snapshot_json.contains("\"is_sensitive\":true"));
            tiez_core_string_free(snapshot);

            let invalid = CString::new("{\"tag\":\"工作\"}").unwrap();
            assert!(tiez_core_update_tags_json(handle, 102, invalid.as_ptr()).is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("JSON string array"));
            tiez_core_string_free(error);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn native_settings_export_is_allowlisted_and_mutable() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let compact_key = CString::new("app.compact_mode").unwrap();
        let true_value = CString::new("true").unwrap();

        unsafe {
            let initial = tiez_core_get_settings_json(handle);
            assert!(!initial.is_null());
            let initial_json = CStr::from_ptr(initial).to_str().unwrap();
            assert!(initial_json.contains("\"abi_version\":11"));
            assert!(initial_json.contains("\"app.compact_mode\":\"false\""));
            assert!(!initial_json.contains("mqtt_password"));
            tiez_core_string_free(initial);

            let mutation =
                tiez_core_update_setting_json(handle, compact_key.as_ptr(), true_value.as_ptr());
            assert!(!mutation.is_null());
            let mutation_json = CStr::from_ptr(mutation).to_str().unwrap();
            assert!(mutation_json.contains("\"key\":\"app.compact_mode\""));
            assert!(mutation_json.contains("\"value\":\"true\""));
            assert!(mutation_json.contains("\"generation\":2"));
            tiez_core_string_free(mutation);

            let updated = tiez_core_get_settings_json(handle);
            assert!(!updated.is_null());
            assert!(CStr::from_ptr(updated)
                .to_str()
                .unwrap()
                .contains("\"app.compact_mode\":\"true\""));
            tiez_core_string_free(updated);

            let secret_key = CString::new("mqtt_password").unwrap();
            assert!(tiez_core_update_setting_json(
                handle,
                secret_key.as_ptr(),
                true_value.as_ptr(),
            )
            .is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("not exposed to native frontends"));
            tiez_core_string_free(error);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn cloud_sync_exports_keep_passwords_write_only() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let update = CString::new(
            r#"{
                "enabled":true,
                "auto_sync":true,
                "webdav_url":"https://dav.example.test/root",
                "webdav_username":"中文用户",
                "webdav_password":"must-not-cross-boundary",
                "clear_password":false,
                "webdav_base_path":"tiez-sync",
                "interval_secs":120,
                "snapshot_interval_min":720,
                "content_prefs":{"text":true,"image":true,"file_path":false,"emoji":true}
            }"#,
        )
        .unwrap();

        unsafe {
            let initial = tiez_core_get_cloud_sync_settings_json(handle);
            assert!(!initial.is_null());
            let initial_json = CStr::from_ptr(initial).to_str().unwrap();
            assert!(initial_json.contains("\"abi_version\":11"));
            assert!(initial_json.contains("\"password_configured\":false"));
            assert!(!initial_json.contains("webdav_password"));
            tiez_core_string_free(initial);

            let mutation = tiez_core_update_cloud_sync_settings_json(handle, update.as_ptr());
            assert!(!mutation.is_null());
            let mutation_json = CStr::from_ptr(mutation).to_str().unwrap();
            assert!(mutation_json.contains("\"password_configured\":true"));
            assert!(mutation_json.contains("\"webdav_username\":\"中文用户\""));
            assert!(mutation_json.contains("\"file_path\":false"));
            assert!(!mutation_json.contains("must-not-cross-boundary"));
            assert!(!mutation_json.contains("webdav_password"));
            tiez_core_string_free(mutation);

            let snapshot = tiez_core_get_cloud_sync_settings_json(handle);
            assert!(!snapshot.is_null());
            let snapshot_json = CStr::from_ptr(snapshot).to_str().unwrap();
            assert!(snapshot_json.contains("\"password_configured\":true"));
            assert!(!snapshot_json.contains("must-not-cross-boundary"));
            tiez_core_string_free(snapshot);

            let invalid = CString::new(
                r#"{
                    "enabled":true,
                    "auto_sync":true,
                    "webdav_url":"http://dav.example.test/root",
                    "webdav_username":"",
                    "clear_password":false,
                    "webdav_base_path":"tiez-sync",
                    "interval_secs":120,
                    "snapshot_interval_min":720,
                    "content_prefs":{"text":true,"image":true,"file_path":true,"emoji":true}
                }"#,
            )
            .unwrap();
            assert!(tiez_core_update_cloud_sync_settings_json(handle, invalid.as_ptr()).is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("must use HTTPS"));
            tiez_core_string_free(error);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn pinned_order_export_requires_all_pinned_ids_and_preserves_requested_order() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let pin = CString::new("pin").unwrap();

        unsafe {
            let value = tiez_core_apply_action_json(handle, 102, pin.as_ptr());
            assert!(!value.is_null());
            tiez_core_string_free(value);
            let value = tiez_core_apply_action_json(handle, 103, pin.as_ptr());
            assert!(!value.is_null());
            tiez_core_string_free(value);

            let order = CString::new("[103,101,102]").unwrap();
            let value = tiez_core_update_pinned_order_json(handle, order.as_ptr());
            assert!(!value.is_null());
            let json = CStr::from_ptr(value).to_str().unwrap();
            assert!(json.contains("\"abi_version\":11"));
            assert!(json.contains("\"action\":\"reorder-pinned\""));
            assert!(json.contains("\"ordered_ids\":[103,101,102]"));
            assert!(json.contains("\"generation\":4"));
            tiez_core_string_free(value);

            let snapshot = tiez_core_get_snapshot_json(handle, ptr::null());
            assert!(!snapshot.is_null());
            let snapshot_json = CStr::from_ptr(snapshot).to_str().unwrap();
            let first = snapshot_json.find("\"id\":103").unwrap();
            let second = snapshot_json.find("\"id\":101").unwrap();
            let third = snapshot_json.find("\"id\":102").unwrap();
            assert!(first < second && second < third);
            tiez_core_string_free(snapshot);

            let incomplete = CString::new("[103,101]").unwrap();
            assert!(tiez_core_update_pinned_order_json(handle, incomplete.as_ptr()).is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("refresh before reordering"));
            tiez_core_string_free(error);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn paste_action_plans_payload_instead_of_logging_only() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let action = CString::new("paste-plain").unwrap();

        unsafe {
            let value = tiez_core_apply_action_json(handle, 101, action.as_ptr());
            assert!(!value.is_null());
            let json = CStr::from_ptr(value).to_str().unwrap();
            assert!(json.contains("\"action\":\"paste-plain\""));
            assert!(json.contains("Pasted item 101 as plain"));
            assert!(!json.contains(" + HTML"));
            assert!(json.contains("\"replacement_id\":null"));
            tiez_core_string_free(value);

            let rich = CString::new("paste-rich").unwrap();
            let rich_value = tiez_core_apply_action_json(handle, 101, rich.as_ptr());
            assert!(!rich_value.is_null());
            let rich_json = CStr::from_ptr(rich_value).to_str().unwrap();
            assert!(rich_json.contains("Pasted item 101 as rich"));
            assert!(rich_json.contains(" + HTML"));
            tiez_core_string_free(rich_value);

            let copy = CString::new("copy-plain").unwrap();
            let copy_value = tiez_core_apply_action_json(handle, 101, copy.as_ptr());
            assert!(!copy_value.is_null());
            let copy_json = CStr::from_ptr(copy_value).to_str().unwrap();
            assert!(copy_json.contains("Copied item 101 as plain"));
            assert!(!copy_json.contains("Pasted item 101 as plain"));
            tiez_core_string_free(copy_value);

            assert!(tiez_core_apply_action_json(handle, 105, action.as_ptr()).is_null());
            let image_error = tiez_core_take_last_error();
            assert!(!image_error.is_null());
            assert!(CStr::from_ptr(image_error)
                .to_str()
                .unwrap()
                .contains("image"));
            tiez_core_string_free(image_error);

            assert!(tiez_core_apply_action_json(handle, 107, action.as_ptr()).is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("not available to paste"));
            tiez_core_string_free(error);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn invalid_action_sets_a_retrievable_error() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
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

    #[test]
    fn ingest_notifies_changed_callback_and_skips_duplicates() {
        static LAST_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        extern "C" fn on_changed(_: *mut c_void, generation: u64) {
            LAST_GENERATION.store(generation, std::sync::atomic::Ordering::SeqCst);
        }

        LAST_GENERATION.store(0, std::sync::atomic::Ordering::SeqCst);
        let handle = Box::into_raw(Box::new(TiezCoreHandle::wrap(ClipboardHistory::in_memory(
            vec![],
        ))));
        unsafe {
            tiez_core_set_changed_callback(handle, Some(on_changed), ptr::null_mut());
            assert!(tiez_core_start_capture(handle));
            let first = ingest_captured_text(&(*handle).inner, "hello from notepad").unwrap();
            assert_eq!(first, Some(2));
            assert_eq!(LAST_GENERATION.load(std::sync::atomic::Ordering::SeqCst), 2);
            assert!(ingest_captured_text(&(*handle).inner, "hello from notepad")
                .unwrap()
                .is_none());
            assert_eq!(LAST_GENERATION.load(std::sync::atomic::Ordering::SeqCst), 2);
            let key = CString::new("app.deduplicate").unwrap();
            let disabled = CString::new("false").unwrap();
            let mutation = tiez_core_update_setting_json(handle, key.as_ptr(), disabled.as_ptr());
            assert!(!mutation.is_null());
            tiez_core_string_free(mutation);
            assert_eq!(
                ingest_captured_text(&(*handle).inner, "hello from notepad").unwrap(),
                Some(3)
            );
            let snapshot = tiez_core_get_snapshot_json(handle, ptr::null());
            let json = CStr::from_ptr(snapshot).to_str().unwrap();
            assert!(json.contains("hello from notepad"));
            tiez_core_string_free(snapshot);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn ingest_classifies_html_files_and_urls() {
        let handle = Box::into_raw(Box::new(TiezCoreHandle::wrap(ClipboardHistory::in_memory(
            vec![],
        ))));
        unsafe {
            let true_value = CString::new("true").unwrap();
            for key in ["app.capture_files", "app.capture_rich_text"] {
                let key = CString::new(key).unwrap();
                let mutation =
                    tiez_core_update_setting_json(handle, key.as_ptr(), true_value.as_ptr());
                assert!(!mutation.is_null());
                tiez_core_string_free(mutation);
            }

            ingest_captured_snapshot(
                &(*handle).inner,
                ClipboardSnapshot {
                    text: Some("hello".into()),
                    html: Some("<!--StartFragment--><b>hello</b><!--EndFragment-->".into()),
                    ..ClipboardSnapshot::default()
                },
            )
            .unwrap();
            ingest_captured_snapshot(
                &(*handle).inner,
                ClipboardSnapshot {
                    files: vec![r"C:\tmp\notes.md".into()],
                    ..ClipboardSnapshot::default()
                },
            )
            .unwrap();
            ingest_captured_text(&(*handle).inner, "https://example.com").unwrap();

            let snapshot = tiez_core_get_snapshot_json(handle, ptr::null());
            let json = CStr::from_ptr(snapshot).to_str().unwrap();
            assert!(json.contains("\"content_type\":\"url\""));
            assert!(json.contains("\"content_type\":\"file\""));
            assert!(json.contains("\"content_type\":\"rich_text\""));
            tiez_core_string_free(snapshot);

            let content = tiez_core_get_content_json(handle, 1);
            let content_json = CStr::from_ptr(content).to_str().unwrap();
            assert!(content_json.contains("<b>hello</b>"));
            tiez_core_string_free(content);
            tiez_core_destroy(handle);
        }
    }
}
