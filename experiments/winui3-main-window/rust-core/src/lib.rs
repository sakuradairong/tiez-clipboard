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
use tiez_core::ai::{
    AiActionResult, AiProbeResult, AiSettings, AiSettingsMutation, AiSettingsSnapshot,
    AiSettingsUpdate,
};
use tiez_core::backup::{
    apply_pending_restore, create_backup, inspect_backup, schedule_backup_restore, BackupInfo,
};
use tiez_core::clipboard_capture::{
    classify_snapshot, detect_content_type, CaptureFilter, CapturedPayload, ClipboardSnapshot,
};
use tiez_core::clipboard_history::{
    ClipboardHistory, HistoryContent, HistoryItem, HistoryMutationResult, HistorySnapshot,
    PinnedOrderResult,
};
use tiez_core::clipboard_relay::{RelayFetchResult, RelaySendResult};
use tiez_core::cloud_sync_settings::{
    CloudSyncProbeResult, CloudSyncSettings, CloudSyncSettingsMutation, CloudSyncSettingsSnapshot,
    CloudSyncSettingsUpdate,
};
use tiez_core::content_identity::is_text_type;
use tiez_core::content_opening::{prepare_open_content, OpenContentPlan};
use tiez_core::data_directory::resolve_data_directory;
use tiez_core::database_bootstrap::open_database_with_decrypt;
use tiez_core::emoji_favorites::{EmojiFavorites, EmojiFavoritesMutation, EmojiFavoritesSnapshot};
use tiez_core::file_transfer::{FileTransferPreferences, FileTransferPreferencesUpdate};
use tiez_core::image_analysis::{
    analyze_image_entry_from_database, get_image_analysis_from_database, ImageAnalysisResult,
};
use tiez_core::native_settings::{NativeSettingMutation, NativeSettings, NativeSettingsSnapshot};
use tiez_core::paste_coordinator::{plan_paste, PasteFormat, PastePayload};
use tiez_core::runtime_instance::DatabaseInstanceGuard;
use tiez_core::tag_catalog::{
    is_protected_tag, normalized_tag_name, TagCatalog, TagCatalogMutation, TagCatalogSnapshot,
    TagDeletePlan, TagEntriesSnapshot, TagRenamePlan,
};

mod clipboard_relay_service;
mod cloud_sync_service;
mod file_transfer_service;
mod paste_hotkey_service;
mod search_hotkey_service;
mod sequential_paste_service;
mod win32_capture;
#[cfg(windows)]
mod win32_paste;

use clipboard_relay_service::{
    NativeClipboardRelay, NativeRelayHotkeyMutation, NativeRelayHotkeySnapshot,
    NativeRelayKeyMutation, NativeRelaySnapshot,
};
use cloud_sync_service::{NativeCloudSyncService, NativeCloudSyncStatus};
use file_transfer_service::{FileTransferSnapshot, NativeFileTransferService, ReceivedTransfer};
use paste_hotkey_service::{
    NativePasteHotkeyMutation, NativePasteHotkeySnapshot, NativePasteHotkeys,
};
use search_hotkey_service::{
    NativeSearchHotkey, NativeSearchHotkeyMutation, NativeSearchHotkeySnapshot,
};
use sequential_paste_service::{
    NativeSequentialPaste, NativeSequentialPasteMutation, NativeSequentialPasteSnapshot,
};

const ABI_VERSION: u32 = 22;
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
    sequential_paste: Mutex<NativeSequentialPaste>,
    changed: Mutex<Option<ChangedSink>>,
}

#[repr(C)]
pub struct TiezCoreHandle {
    inner: Arc<TiezCoreInner>,
    emoji_favorites: Mutex<EmojiFavorites>,
    tag_catalog: Mutex<TagCatalog>,
    ai_settings: Mutex<AiSettings>,
    cloud_settings: Mutex<CloudSyncSettings>,
    cloud_sync: Arc<NativeCloudSyncService>,
    clipboard_relay: NativeClipboardRelay,
    paste_hotkeys: NativePasteHotkeys,
    search_hotkey: NativeSearchHotkey,
    file_transfer: NativeFileTransferService,
    session: Mutex<Option<win32_capture::Session>>,
}

impl TiezCoreHandle {
    #[cfg_attr(not(test), allow(dead_code))]
    fn wrap(history: ClipboardHistory) -> Self {
        Self::wrap_with_settings(history, NativeSettings::in_memory())
    }

    fn wrap_with_settings(history: ClipboardHistory, settings: NativeSettings) -> Self {
        Self::wrap_with_adapters(
            history,
            settings,
            AiSettings::in_memory(),
            CloudSyncSettings::in_memory(),
        )
    }

    fn wrap_with_adapters(
        history: ClipboardHistory,
        settings: NativeSettings,
        ai_settings: AiSettings,
        cloud_settings: CloudSyncSettings,
    ) -> Self {
        let inner = Arc::new(TiezCoreInner {
            history: Mutex::new(history),
            settings: Mutex::new(settings),
            capture: Mutex::new(CaptureFilter::new()),
            sequential_paste: Mutex::new(NativeSequentialPaste::unavailable()),
            changed: Mutex::new(None),
        });
        let file_transfer = NativeFileTransferService::new(
            FileTransferPreferences::in_memory(default_file_transfer_directory()),
            file_transfer_receiver(Arc::clone(&inner)),
        );
        Self {
            inner,
            emoji_favorites: Mutex::new(EmojiFavorites::in_memory()),
            tag_catalog: Mutex::new(TagCatalog::in_memory()),
            ai_settings: Mutex::new(ai_settings),
            cloud_settings: Mutex::new(cloud_settings),
            cloud_sync: Arc::new(NativeCloudSyncService::unavailable()),
            clipboard_relay: NativeClipboardRelay::unavailable(),
            paste_hotkeys: NativePasteHotkeys::unavailable(),
            search_hotkey: NativeSearchHotkey::unavailable(),
            file_transfer,
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
        let read_only = configured_database.is_some() && env_flag(DATABASE_READ_ONLY_ENV);
        let (history, settings, ai_settings, cloud_settings) = match configured_database.as_ref() {
            Some(value) => {
                ensure_database_instance_guard(&value)?;
                if !read_only {
                    prepare_writable_database(&value)?;
                }
                let history = ClipboardHistory::open_sqlite(&value, read_only)
                    .map_err(|error| format!("{}: {error}", value.display()))?;
                let settings = NativeSettings::open_sqlite(&value, read_only)
                    .map_err(|error| format!("{}: {error}", value.display()))?;
                let ai_settings = AiSettings::open_sqlite(&value, read_only)
                    .map_err(|error| format!("{}: {error}", value.display()))?;
                let cloud_settings = CloudSyncSettings::open_sqlite(&value, read_only)
                    .map_err(|error| format!("{}: {error}", value.display()))?;
                (history, settings, ai_settings, cloud_settings)
            }
            _ => (
                ClipboardHistory::synthetic(),
                NativeSettings::in_memory(),
                AiSettings::in_memory(),
                CloudSyncSettings::in_memory(),
            ),
        };

        let mut handle = Self::wrap_with_adapters(history, settings, ai_settings, cloud_settings);
        if let Some(database_path) = configured_database {
            let data_dir = database_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            handle.emoji_favorites = Mutex::new(
                EmojiFavorites::open_sqlite(&database_path, &data_dir, read_only)
                    .map_err(|error| format!("{}: {error}", database_path.display()))?,
            );
            handle.tag_catalog = Mutex::new(
                TagCatalog::open_sqlite(&database_path, read_only)
                    .map_err(|error| format!("{}: {error}", database_path.display()))?,
            );
            let inner = Arc::clone(&handle.inner);
            handle.cloud_sync = Arc::new(NativeCloudSyncService::new(
                &database_path,
                data_dir.clone(),
                read_only,
                move || {
                    let generation = inner
                        .history
                        .lock()
                        .ok()
                        .and_then(|history| history.snapshot("").ok())
                        .map(|snapshot| snapshot.generation);
                    if let Some(generation) = generation {
                        notify_changed(&inner, generation);
                    }
                },
            ));
            handle.clipboard_relay =
                NativeClipboardRelay::new(&database_path, data_dir.clone(), read_only);
            handle.paste_hotkeys = NativePasteHotkeys::new(&database_path, read_only);
            handle.search_hotkey = NativeSearchHotkey::new(&database_path, read_only);
            *handle
                .inner
                .sequential_paste
                .lock()
                .map_err(|_| "SequentialPaste lock is poisoned".to_owned())? =
                NativeSequentialPaste::new(&database_path, read_only);
            handle.file_transfer = NativeFileTransferService::new(
                FileTransferPreferences::open_sqlite(
                    &database_path,
                    default_file_transfer_directory(),
                    read_only,
                )
                .map_err(|error| format!("{}: {error}", database_path.display()))?,
                file_transfer_receiver(Arc::clone(&handle.inner)),
            );
            let should_start_file_transfer = handle
                .file_transfer
                .snapshot()
                .map(|snapshot| snapshot.preferences.enabled && !snapshot.preferences.read_only)
                .unwrap_or(false);
            if should_start_file_transfer {
                if let Err(error) = handle.file_transfer.start() {
                    eprintln!(">>> [FILE TRANSFER] {error}");
                }
            }
        }
        Ok(handle)
    }
}

fn default_file_transfer_directory() -> PathBuf {
    env::var_os("USERPROFILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join("Downloads").join("TieZ"))
        .unwrap_or_else(|| env::temp_dir().join("TieZ").join("接收"))
}

fn file_transfer_receiver(
    inner: Arc<TiezCoreInner>,
) -> impl Fn(ReceivedTransfer) -> Result<(), String> + Send + Sync + 'static {
    move |received| {
        let mutation = {
            let mut history = inner
                .history
                .lock()
                .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
            match received {
                ReceivedTransfer::Text {
                    content,
                    sender_name,
                } => history
                    .ingest_text(content, sender_name)
                    .map_err(|error| error.to_string())?,
                ReceivedTransfer::File { path, sender_name } => history
                    .ingest(
                        CapturedPayload::Files {
                            paths: vec![path.to_string_lossy().into_owned()],
                        },
                        sender_name,
                    )
                    .map_err(|error| error.to_string())?,
            }
        };
        notify_changed(&inner, mutation.generation);
        Ok(())
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
struct SearchHotkeySnapshotResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a NativeSearchHotkeySnapshot,
}

#[derive(Serialize)]
struct SearchHotkeyMutationResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    mutation: &'a NativeSearchHotkeyMutation,
}

#[derive(Serialize)]
struct PasteHotkeySnapshotResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a NativePasteHotkeySnapshot,
}

#[derive(Serialize)]
struct PasteHotkeyMutationResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    mutation: &'a NativePasteHotkeyMutation,
}

#[derive(Serialize)]
struct SequentialPasteSnapshotResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a NativeSequentialPasteSnapshot,
}

#[derive(Serialize)]
struct SequentialPasteMutationResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    mutation: &'a NativeSequentialPasteMutation,
}

#[derive(Serialize)]
struct SequentialPasteActionResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    mutation: &'a HistoryMutationResult,
    queued_items: usize,
    queue_finished: bool,
}

#[derive(Serialize)]
struct EmojiFavoritesResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a EmojiFavoritesSnapshot,
}

#[derive(Serialize)]
struct EmojiFavoritesMutationResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    mutation: &'a EmojiFavoritesMutation,
}

#[derive(Serialize)]
struct TagCatalogResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a TagCatalogSnapshot,
}

#[derive(Serialize)]
struct TagEntriesResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a TagEntriesSnapshot,
}

#[derive(Serialize)]
struct TagCatalogMutationResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    mutation: &'a TagCatalogMutation,
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
struct CloudSyncStatusResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    status: &'a NativeCloudSyncStatus,
}

#[derive(Serialize)]
struct RelaySnapshotResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a NativeRelaySnapshot,
}

#[derive(Serialize)]
struct RelayKeyMutationResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    mutation: &'a NativeRelayKeyMutation,
}

#[derive(Serialize)]
struct RelayHotkeySnapshotResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a NativeRelayHotkeySnapshot,
}

#[derive(Serialize)]
struct RelayHotkeyMutationResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    mutation: &'a NativeRelayHotkeyMutation,
}

#[derive(Serialize)]
struct RelaySendResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    result: &'a RelaySendResult,
}

#[derive(Serialize)]
struct RelayFetchResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    result: &'a RelayFetchResult,
}

#[derive(Serialize)]
struct FileTransferResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a FileTransferSnapshot,
}

#[derive(Serialize)]
struct AiSettingsResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    snapshot: &'a AiSettingsSnapshot,
}

#[derive(Serialize)]
struct AiSettingsMutationResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    mutation: &'a AiSettingsMutation,
}

#[derive(Serialize)]
struct AiProbeResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    result: &'a AiProbeResult,
}

#[derive(Serialize)]
struct AiActionResponse<'a> {
    abi_version: u32,
    #[serde(flatten)]
    result: &'a AiActionResult,
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

fn search_hotkey_snapshot_json(snapshot: &NativeSearchHotkeySnapshot) -> Result<String, String> {
    serde_json::to_string(&SearchHotkeySnapshotResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("无法序列化搜索快捷键状态：{error}"))
}

fn search_hotkey_mutation_json(mutation: &NativeSearchHotkeyMutation) -> Result<String, String> {
    serde_json::to_string(&SearchHotkeyMutationResponse {
        abi_version: ABI_VERSION,
        mutation,
    })
    .map_err(|error| format!("无法序列化搜索快捷键结果：{error}"))
}

fn paste_hotkey_snapshot_json(snapshot: &NativePasteHotkeySnapshot) -> Result<String, String> {
    serde_json::to_string(&PasteHotkeySnapshotResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("无法序列化粘贴快捷键状态：{error}"))
}

fn paste_hotkey_mutation_json(mutation: &NativePasteHotkeyMutation) -> Result<String, String> {
    serde_json::to_string(&PasteHotkeyMutationResponse {
        abi_version: ABI_VERSION,
        mutation,
    })
    .map_err(|error| format!("无法序列化粘贴快捷键结果：{error}"))
}

fn sequential_paste_snapshot_json(
    snapshot: &NativeSequentialPasteSnapshot,
) -> Result<String, String> {
    serde_json::to_string(&SequentialPasteSnapshotResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("无法序列化顺序粘贴状态：{error}"))
}

fn sequential_paste_mutation_json(
    mutation: &NativeSequentialPasteMutation,
) -> Result<String, String> {
    serde_json::to_string(&SequentialPasteMutationResponse {
        abi_version: ABI_VERSION,
        mutation,
    })
    .map_err(|error| format!("无法序列化顺序粘贴设置结果：{error}"))
}

fn sequential_paste_action_json(
    mutation: &HistoryMutationResult,
    queued_items: usize,
) -> Result<String, String> {
    serde_json::to_string(&SequentialPasteActionResponse {
        abi_version: ABI_VERSION,
        mutation,
        queued_items,
        queue_finished: queued_items == 0,
    })
    .map_err(|error| format!("无法序列化顺序粘贴结果：{error}"))
}

fn emoji_favorites_json(snapshot: &EmojiFavoritesSnapshot) -> Result<String, String> {
    serde_json::to_string(&EmojiFavoritesResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("failed to serialize Emoji favorites: {error}"))
}

fn emoji_favorites_mutation_json(mutation: &EmojiFavoritesMutation) -> Result<String, String> {
    serde_json::to_string(&EmojiFavoritesMutationResponse {
        abi_version: ABI_VERSION,
        mutation,
    })
    .map_err(|error| format!("failed to serialize Emoji favorites mutation: {error}"))
}

fn tag_catalog_json(snapshot: &TagCatalogSnapshot) -> Result<String, String> {
    serde_json::to_string(&TagCatalogResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("failed to serialize tag catalog: {error}"))
}

fn tag_entries_json(snapshot: &TagEntriesSnapshot) -> Result<String, String> {
    serde_json::to_string(&TagEntriesResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("failed to serialize tag entries: {error}"))
}

fn tag_catalog_mutation_json(mutation: &TagCatalogMutation) -> Result<String, String> {
    serde_json::to_string(&TagCatalogMutationResponse {
        abi_version: ABI_VERSION,
        mutation,
    })
    .map_err(|error| format!("failed to serialize tag catalog mutation: {error}"))
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

fn cloud_sync_status_json(status: &NativeCloudSyncStatus) -> Result<String, String> {
    serde_json::to_string(&CloudSyncStatusResponse {
        abi_version: ABI_VERSION,
        status,
    })
    .map_err(|error| format!("failed to serialize cloud-sync status: {error}"))
}

fn relay_snapshot_json(snapshot: &NativeRelaySnapshot) -> Result<String, String> {
    serde_json::to_string(&RelaySnapshotResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("无法序列化剪贴板接力状态：{error}"))
}

fn relay_key_mutation_json(mutation: &NativeRelayKeyMutation) -> Result<String, String> {
    serde_json::to_string(&RelayKeyMutationResponse {
        abi_version: ABI_VERSION,
        mutation,
    })
    .map_err(|error| format!("无法序列化剪贴板接力密钥结果：{error}"))
}

fn relay_hotkey_snapshot_json(snapshot: &NativeRelayHotkeySnapshot) -> Result<String, String> {
    serde_json::to_string(&RelayHotkeySnapshotResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("无法序列化剪贴板接力快捷键状态：{error}"))
}

fn relay_hotkey_mutation_json(mutation: &NativeRelayHotkeyMutation) -> Result<String, String> {
    serde_json::to_string(&RelayHotkeyMutationResponse {
        abi_version: ABI_VERSION,
        mutation,
    })
    .map_err(|error| format!("无法序列化剪贴板接力快捷键结果：{error}"))
}

fn relay_send_json(result: &RelaySendResult) -> Result<String, String> {
    serde_json::to_string(&RelaySendResponse {
        abi_version: ABI_VERSION,
        result,
    })
    .map_err(|error| format!("无法序列化剪贴板接力发送结果：{error}"))
}

fn relay_fetch_json(result: &RelayFetchResult) -> Result<String, String> {
    serde_json::to_string(&RelayFetchResponse {
        abi_version: ABI_VERSION,
        result,
    })
    .map_err(|error| format!("无法序列化剪贴板接力接收结果：{error}"))
}

fn file_transfer_json(snapshot: &FileTransferSnapshot) -> Result<String, String> {
    serde_json::to_string(&FileTransferResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("failed to serialize file-transfer snapshot: {error}"))
}

fn ai_settings_json(snapshot: &AiSettingsSnapshot) -> Result<String, String> {
    serde_json::to_string(&AiSettingsResponse {
        abi_version: ABI_VERSION,
        snapshot,
    })
    .map_err(|error| format!("failed to serialize AI settings: {error}"))
}

fn ai_settings_mutation_json(mutation: &AiSettingsMutation) -> Result<String, String> {
    serde_json::to_string(&AiSettingsMutationResponse {
        abi_version: ABI_VERSION,
        mutation,
    })
    .map_err(|error| format!("failed to serialize AI settings mutation: {error}"))
}

fn ai_probe_json(result: &AiProbeResult) -> Result<String, String> {
    serde_json::to_string(&AiProbeResponse {
        abi_version: ABI_VERSION,
        result,
    })
    .map_err(|error| format!("failed to serialize AI probe result: {error}"))
}

fn ai_action_json(result: &AiActionResult) -> Result<String, String> {
    serde_json::to_string(&AiActionResponse {
        abi_version: ABI_VERSION,
        result,
    })
    .map_err(|error| format!("failed to serialize AI action result: {error}"))
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

    let mutation = history
        .apply_action(entry_id, action)
        .map_err(|error| error.to_string())?;
    drop(history);
    if matches!(action, "pin" | "delete" | "clear") {
        request_cloud_sync_for_history_mutation(handle, &mutation);
    }
    Ok(mutation)
}

fn request_cloud_sync_after_change(handle: &TiezCoreHandle) {
    if let Err(error) = handle.cloud_sync.request_change() {
        eprintln!(">>> [CLOUD SYNC] Unable to request automatic sync: {error}");
    }
}

fn request_cloud_sync_for_history_mutation(
    handle: &TiezCoreHandle,
    mutation: &HistoryMutationResult,
) {
    let persisted = mutation.requested_id > 0
        || mutation.effective_id.is_some_and(|entry_id| entry_id > 0)
        || mutation.replacement_id.is_some_and(|entry_id| entry_id > 0)
        || (mutation.action == "clear" && mutation.removed);
    if persisted {
        request_cloud_sync_after_change(handle);
    }
}

fn paste_latest_history(
    handle: &TiezCoreHandle,
    kind: &str,
) -> Result<HistoryMutationResult, String> {
    let (text_only, action, label) = match kind.trim().to_ascii_lowercase().as_str() {
        "rich" => (false, "paste-rich", "富文本"),
        "plain" => (true, "paste-plain", "纯文本"),
        _ => return Err("粘贴类型必须是 rich 或 plain".to_owned()),
    };
    let latest = {
        let history = handle
            .inner
            .history
            .lock()
            .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
        history
            .snapshot("")
            .map_err(|error| error.to_string())?
            .items
            .into_iter()
            .next()
            .ok_or_else(|| "剪贴板历史为空，没有可粘贴的记录".to_owned())?
    };
    if text_only && !is_text_type(&latest.content_type) {
        return Err(format!(
            "最新记录是 {}，纯文本快捷键只粘贴文本、代码、链接或富文本",
            latest.content_type
        ));
    }

    let delete_after = handle.paste_hotkeys.delete_after_paste()?;
    let protected = latest.is_pinned || !latest.tags.is_empty();
    let mut mutation = apply_history_action(handle, latest.id, action)?;
    if delete_after && !protected {
        let mut deleted = handle
            .inner
            .history
            .lock()
            .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?
            .apply_action(latest.id, "delete")
            .map_err(|error| error.to_string())?;
        deleted.action = action.to_owned();
        deleted.message = format!("{}；粘贴后已删除未保护记录", mutation.message);
        request_cloud_sync_for_history_mutation(handle, &deleted);
        return Ok(deleted);
    }
    mutation.message = format!("已通过{label}快捷键粘贴最新记录；{}", mutation.message);
    Ok(mutation)
}

fn paste_next_sequential(
    handle: &TiezCoreHandle,
) -> Result<(HistoryMutationResult, usize), String> {
    let entry_id = {
        let mut sequential = handle
            .inner
            .sequential_paste
            .lock()
            .map_err(|_| "SequentialPaste lock is poisoned".to_owned())?;
        let snapshot = sequential.snapshot()?;
        if !snapshot.available {
            return Err(snapshot
                .unavailable_reason
                .unwrap_or_else(|| "顺序粘贴当前不可用".to_owned()));
        }
        if snapshot.read_only {
            return Err("当前数据库为只读，顺序粘贴已停用".to_owned());
        }
        if !snapshot.enabled {
            return Err("请先在设置中开启顺序粘贴模式".to_owned());
        }
        sequential
            .pop_next()
            .ok_or_else(|| "顺序粘贴队列为空；请先复制需要依次粘贴的内容".to_owned())?
    };

    let attempt = (|| -> Result<HistoryMutationResult, String> {
        let (content, protected) = {
            let history = handle
                .inner
                .history
                .lock()
                .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
            let item = history
                .snapshot("")
                .map_err(|error| error.to_string())?
                .items
                .into_iter()
                .find(|item| item.id == entry_id)
                .ok_or_else(|| format!("顺序粘贴记录 {entry_id} 已不可用"))?;
            let content = history
                .content(entry_id)
                .map_err(|error| error.to_string())?;
            (content, item.is_pinned || !item.tags.is_empty())
        };
        let action = if content.content_type == "rich_text" && content.html_content.is_some() {
            "paste-rich"
        } else {
            "paste-plain"
        };
        let delete_after = handle.paste_hotkeys.delete_after_paste()?;
        let mut mutation = apply_history_action(handle, entry_id, action)?;
        if delete_after && !protected {
            let deletion = handle
                .inner
                .history
                .lock()
                .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?
                .apply_action(entry_id, "delete");
            match deletion {
                Ok(mut deleted) => {
                    deleted.action = action.to_owned();
                    deleted.message = format!("{}；顺序粘贴后已删除未保护记录", mutation.message);
                    request_cloud_sync_for_history_mutation(handle, &deleted);
                    return Ok(deleted);
                }
                Err(error) => {
                    mutation.message = format!(
                        "{}；顺序粘贴成功，但自动删除失败：{error}",
                        mutation.message
                    );
                    return Ok(mutation);
                }
            }
        }
        mutation.message = format!("已按复制顺序粘贴记录；{}", mutation.message);
        Ok(mutation)
    })();

    let mutation = match attempt {
        Ok(mutation) => mutation,
        Err(error) => {
            if let Ok(mut sequential) = handle.inner.sequential_paste.lock() {
                sequential.requeue_front(entry_id);
            }
            return Err(error);
        }
    };
    let queued_items = {
        let mut sequential = handle
            .inner
            .sequential_paste
            .lock()
            .map_err(|_| "SequentialPaste lock is poisoned".to_owned())?;
        sequential.mark_pasted();
        sequential.snapshot()?.queued_items
    };
    Ok((mutation, queued_items))
}

fn paste_transient_text(handle: &TiezCoreHandle, text: &str) -> Result<(), String> {
    let content = HistoryContent {
        id: 0,
        content_type: "text".to_owned(),
        content: text.to_owned(),
        html_content: None,
        available: true,
        is_sensitive: false,
        unavailable_reason: None,
    };
    let plan =
        plan_paste(&content, PasteFormat::Plain, false).map_err(|error| error.to_string())?;
    execute_os_paste(&plan)?;
    if let Ok(mut filter) = handle.inner.capture.lock() {
        if let Some(payload) = payload_written_to_clipboard(&plan) {
            filter.note_payload(&payload);
        }
    }
    Ok(())
}

fn paste_emoji_favorite(handle: &TiezCoreHandle, favorite_path: &str) -> Result<(), String> {
    let favorite_path = handle
        .emoji_favorites
        .lock()
        .map_err(|_| "EmojiFavorites lock is poisoned".to_owned())?
        .favorite_path_for_paste(favorite_path)
        .map_err(|error| error.to_string())?;
    let content = HistoryContent {
        id: 0,
        content_type: "image".to_owned(),
        content: favorite_path,
        html_content: None,
        available: true,
        is_sensitive: false,
        unavailable_reason: None,
    };
    let plan =
        plan_paste(&content, PasteFormat::Plain, false).map_err(|error| error.to_string())?;
    execute_os_paste(&plan)?;
    if let Ok(mut filter) = handle.inner.capture.lock() {
        if let Some(payload) = payload_written_to_clipboard(&plan) {
            filter.note_payload(&payload);
        }
    }
    Ok(())
}

fn emoji_favorites_snapshot(handle: &TiezCoreHandle) -> Result<EmojiFavoritesSnapshot, String> {
    handle
        .emoji_favorites
        .lock()
        .map_err(|_| "EmojiFavorites lock is poisoned".to_owned())?
        .snapshot()
        .map_err(|error| error.to_string())
}

fn import_emoji_favorite(
    handle: &TiezCoreHandle,
    source_path: &str,
) -> Result<EmojiFavoritesMutation, String> {
    let mutation = handle
        .emoji_favorites
        .lock()
        .map_err(|_| "EmojiFavorites lock is poisoned".to_owned())?
        .import_file(source_path)
        .map_err(|error| error.to_string())?;
    if mutation.changed {
        request_cloud_sync_after_change(handle);
    }
    Ok(mutation)
}

fn remove_emoji_favorite(
    handle: &TiezCoreHandle,
    favorite_path: &str,
) -> Result<EmojiFavoritesMutation, String> {
    let mutation = handle
        .emoji_favorites
        .lock()
        .map_err(|_| "EmojiFavorites lock is poisoned".to_owned())?
        .remove(favorite_path)
        .map_err(|error| error.to_string())?;
    if mutation.changed {
        request_cloud_sync_after_change(handle);
    }
    Ok(mutation)
}

fn current_history_items(handle: &TiezCoreHandle) -> Result<Vec<HistoryItem>, String> {
    handle
        .inner
        .history
        .lock()
        .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?
        .snapshot("")
        .map(|snapshot| snapshot.items)
        .map_err(|error| error.to_string())
}

fn tag_catalog_snapshot(handle: &TiezCoreHandle) -> Result<TagCatalogSnapshot, String> {
    let history_items = current_history_items(handle)?;
    handle
        .tag_catalog
        .lock()
        .map_err(|_| "TagCatalog lock is poisoned".to_owned())?
        .snapshot(&history_items)
        .map_err(|error| error.to_string())
}

fn tag_entries_snapshot(handle: &TiezCoreHandle, tag: &str) -> Result<TagEntriesSnapshot, String> {
    let history_items = current_history_items(handle)?;
    handle
        .tag_catalog
        .lock()
        .map_err(|_| "TagCatalog lock is poisoned".to_owned())?
        .entries(tag, &history_items)
        .map_err(|error| error.to_string())
}

fn create_tag(handle: &TiezCoreHandle, name: &str) -> Result<TagCatalogMutation, String> {
    handle
        .tag_catalog
        .lock()
        .map_err(|_| "TagCatalog lock is poisoned".to_owned())?
        .create(name)
        .map_err(|error| error.to_string())
}

fn set_tag_color(
    handle: &TiezCoreHandle,
    name: &str,
    color: Option<&str>,
) -> Result<TagCatalogMutation, String> {
    handle
        .tag_catalog
        .lock()
        .map_err(|_| "TagCatalog lock is poisoned".to_owned())?
        .set_color(name, color)
        .map_err(|error| error.to_string())
}

fn apply_tag_rename_plan(handle: &TiezCoreHandle, plan: &TagRenamePlan) -> Result<usize, String> {
    let mut applied = 0usize;
    let mut last_generation = None;
    let mut failure = None;
    {
        let mut history = handle
            .inner
            .history
            .lock()
            .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
        for entry in &plan.entries {
            match history.update_tags(entry.id, entry.tags.clone()) {
                Ok(mutation) => {
                    applied += 1;
                    last_generation = Some(mutation.generation);
                }
                Err(error) => {
                    failure = Some(error.to_string());
                    break;
                }
            }
        }
    }
    if let Some(generation) = last_generation {
        notify_changed(&handle.inner, generation);
    }
    if applied > 0 {
        request_cloud_sync_after_change(handle);
    }
    if let Some(error) = failure {
        return Err(format!(
            "标签重命名已安全更新 {applied}/{} 条记录；其余记录未改动：{error}",
            plan.entries.len()
        ));
    }
    Ok(applied)
}

fn rename_tag(
    handle: &TiezCoreHandle,
    old_name: &str,
    new_name: &str,
) -> Result<TagCatalogMutation, String> {
    let history_items = current_history_items(handle)?;
    let plan = handle
        .tag_catalog
        .lock()
        .map_err(|_| "TagCatalog lock is poisoned".to_owned())?
        .rename_plan(old_name, new_name, &history_items)
        .map_err(|error| error.to_string())?;
    apply_tag_rename_plan(handle, &plan)?;
    handle
        .tag_catalog
        .lock()
        .map_err(|_| "TagCatalog lock is poisoned".to_owned())?
        .finish_rename(&plan)
        .map_err(|error| error.to_string())
}

fn apply_tag_delete_plan(handle: &TiezCoreHandle, plan: &TagDeletePlan) -> Result<usize, String> {
    let mut applied = 0usize;
    let mut last_generation = None;
    let mut failure = None;
    {
        let mut history = handle
            .inner
            .history
            .lock()
            .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
        for entry_id in &plan.entry_ids {
            match history.apply_action(*entry_id, "delete") {
                Ok(mutation) => {
                    applied += 1;
                    last_generation = Some(mutation.generation);
                }
                Err(error) => {
                    failure = Some(error.to_string());
                    break;
                }
            }
        }
    }
    if let Some(generation) = last_generation {
        notify_changed(&handle.inner, generation);
    }
    if applied > 0 {
        request_cloud_sync_after_change(handle);
    }
    if let Some(error) = failure {
        return Err(format!(
            "标签删除已安全删除 {applied}/{} 条记录；其余记录未改动：{error}",
            plan.entry_ids.len()
        ));
    }
    Ok(applied)
}

fn delete_tag(handle: &TiezCoreHandle, name: &str) -> Result<TagCatalogMutation, String> {
    let history_items = current_history_items(handle)?;
    let plan = handle
        .tag_catalog
        .lock()
        .map_err(|_| "TagCatalog lock is poisoned".to_owned())?
        .delete_plan(name, &history_items)
        .map_err(|error| error.to_string())?;
    apply_tag_delete_plan(handle, &plan)?;
    handle
        .tag_catalog
        .lock()
        .map_err(|_| "TagCatalog lock is poisoned".to_owned())?
        .finish_delete(&plan)
        .map_err(|error| error.to_string())
}

fn create_tagged_text(
    handle: &TiezCoreHandle,
    tag: &str,
    content: &str,
) -> Result<HistoryMutationResult, String> {
    if content.trim().is_empty() {
        return Err("手动文本不能为空".to_owned());
    }
    let tag = normalized_tag_name(tag).map_err(|error| error.to_string())?;
    if is_protected_tag(&tag) {
        return Err(
            "不能直接向内置敏感标签添加手动文本；请先保存到普通标签，再在记录详情中安全添加敏感标签"
                .to_owned(),
        );
    }
    create_tag(handle, &tag)?;
    let entry_id = {
        let mut history = handle
            .inner
            .history
            .lock()
            .map_err(|_| "ClipboardHistory lock is poisoned".to_owned())?;
        history
            .ingest_text(content.to_owned(), "TieZ 手动")
            .map_err(|error| error.to_string())?
            .effective_id
            .ok_or_else(|| "手动文本没有生成可用记录".to_owned())?
    };
    let history_items = current_history_items(handle)?;
    let mut tags = handle
        .tag_catalog
        .lock()
        .map_err(|_| "TagCatalog lock is poisoned".to_owned())?
        .tags_for_entry(entry_id, &history_items)
        .map_err(|error| error.to_string())?;
    if !tags.iter().any(|existing| existing == &tag) {
        tags.push(tag);
    }
    let mut mutation = update_history_tags(handle, entry_id, tags)?;
    mutation.action = "create-tagged-text".to_owned();
    mutation.message = "已添加带标签的手动文本".to_owned();
    Ok(mutation)
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
    request_cloud_sync_for_history_mutation(handle, &mutation);
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
    request_cloud_sync_after_change(handle);
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

fn ai_settings_snapshot(handle: &TiezCoreHandle) -> Result<AiSettingsSnapshot, String> {
    handle
        .ai_settings
        .lock()
        .map_err(|_| "AiSettings lock is poisoned".to_owned())?
        .snapshot()
        .map_err(|error| error.to_string())
}

fn update_ai_settings(
    handle: &TiezCoreHandle,
    update: AiSettingsUpdate,
) -> Result<AiSettingsMutation, String> {
    handle
        .ai_settings
        .lock()
        .map_err(|_| "AiSettings lock is poisoned".to_owned())?
        .update(update)
        .map_err(|error| error.to_string())
}

fn probe_ai_profile(handle: &TiezCoreHandle, profile_id: &str) -> Result<AiProbeResult, String> {
    handle
        .ai_settings
        .lock()
        .map_err(|_| "AiSettings lock is poisoned".to_owned())?
        .probe_profile(profile_id)
        .map_err(|error| error.to_string())
}

fn run_history_ai_action(
    handle: &TiezCoreHandle,
    entry_id: i64,
    action: &str,
) -> Result<AiActionResult, String> {
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
    if content.is_sensitive {
        return Err("敏感剪贴板内容禁止发送到 AI 服务".to_owned());
    }
    if !content.available {
        return Err("此剪贴板内容当前不可用于 AI".to_owned());
    }
    if !is_text_type(&content.content_type) {
        return Err("AI 助手当前只处理文本、富文本、链接和代码记录".to_owned());
    }
    handle
        .ai_settings
        .lock()
        .map_err(|_| "AiSettings lock is poisoned".to_owned())?
        .run_action(action, &content.content)
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
    ingest_captured_snapshot_result(inner, snapshot)
        .map(|result| result.map(|(generation, _)| generation))
}

pub(crate) fn ingest_captured_snapshot_result(
    inner: &TiezCoreInner,
    snapshot: ClipboardSnapshot,
) -> Result<Option<(u64, bool)>, String> {
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
    if let Some(entry_id) = mutation.effective_id {
        match inner.sequential_paste.lock() {
            Ok(mut sequential) => {
                if let Err(error) = sequential.record_capture(entry_id) {
                    eprintln!(">>> [SEQUENTIAL PASTE] Unable to enqueue capture: {error}");
                }
            }
            Err(_) => eprintln!(">>> [SEQUENTIAL PASTE] Queue lock is poisoned"),
        }
    }
    let generation = mutation.generation;
    let persisted = mutation.effective_id.is_some_and(|entry_id| entry_id > 0);
    notify_changed(inner, generation);
    Ok(Some((generation, persisted)))
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
    let cloud_sync = Arc::clone(&handle.cloud_sync);
    let request_cloud_sync = Arc::new(move || {
        if let Err(error) = cloud_sync.request_change() {
            eprintln!(">>> [CLOUD SYNC] Unable to distribute captured item: {error}");
        }
    });
    *session = Some(win32_capture::start(
        Arc::clone(&handle.inner),
        request_cloud_sync,
    )?);
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

fn read_relay_clipboard_text() -> Result<String, String> {
    win32_capture::read_plain_text_exact()
}

fn copy_relay_text(handle: &TiezCoreHandle, text: &str) -> Result<(), String> {
    #[cfg(all(windows, not(test)))]
    {
        if let Ok(mut filter) = handle.inner.capture.lock() {
            filter.note_self_write(text);
        }
        crate::win32_paste::copy_text(text)
    }
    #[cfg(not(all(windows, not(test))))]
    {
        let _ = (handle, text);
        Err("当前测试平台没有可用的 Windows 文本剪贴板".to_owned())
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
        // and cloud worker before the inner Arc or DLL can be unloaded.
        let boxed = unsafe { Box::from_raw(handle) };
        boxed.file_transfer.stop();
        boxed.cloud_sync.stop();
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
/// Paste arbitrary UTF-8 text without creating a clipboard-history row.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `text_utf8` must remain a readable, NUL-terminated UTF-8 string for this
/// call.
pub unsafe extern "C" fn tiez_core_paste_text(
    handle: *mut TiezCoreHandle,
    text_utf8: *const c_char,
) -> bool {
    with_ffi_result(false, || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let text = required_utf8(text_utf8, "text_utf8")?;
        paste_transient_text(handle, &text)?;
        Ok(true)
    })
}

#[no_mangle]
/// Return image Emoji favorites using the existing SQLite setting and data
/// directory contract.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_emoji_favorites_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let snapshot = emoji_favorites_snapshot(handle)?;
        into_owned_c_string(emoji_favorites_json(&snapshot)?)
    })
}

#[no_mangle]
/// Import a local image into the managed Emoji favorites directory.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `source_path_utf8` must remain a readable, NUL-terminated UTF-8 path for
/// this call.
pub unsafe extern "C" fn tiez_core_import_emoji_favorite_json(
    handle: *mut TiezCoreHandle,
    source_path_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let source_path = required_utf8(source_path_utf8, "source_path_utf8")?;
        let mutation = import_emoji_favorite(handle, &source_path)?;
        into_owned_c_string(emoji_favorites_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Remove one image from the ordered Emoji favorites list. Managed files are
/// removed safely; external paths are never deleted.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `favorite_path_utf8` must remain a readable, NUL-terminated UTF-8 path for
/// this call.
pub unsafe extern "C" fn tiez_core_remove_emoji_favorite_json(
    handle: *mut TiezCoreHandle,
    favorite_path_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let favorite_path = required_utf8(favorite_path_utf8, "favorite_path_utf8")?;
        let mutation = remove_emoji_favorite(handle, &favorite_path)?;
        into_owned_c_string(emoji_favorites_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Paste a currently registered image Emoji favorite without creating a
/// clipboard-history row.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `favorite_path_utf8` must remain a readable, NUL-terminated UTF-8 path for
/// this call.
pub unsafe extern "C" fn tiez_core_paste_emoji_favorite(
    handle: *mut TiezCoreHandle,
    favorite_path_utf8: *const c_char,
) -> bool {
    with_ffi_result(false, || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let favorite_path = required_utf8(favorite_path_utf8, "favorite_path_utf8")?;
        paste_emoji_favorite(handle, &favorite_path)?;
        Ok(true)
    })
}

#[no_mangle]
/// Return saved and in-use tags with counts, colors, and protected status.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_tag_catalog_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let snapshot = tag_catalog_snapshot(handle)?;
        into_owned_c_string(tag_catalog_json(&snapshot)?)
    })
}

#[no_mangle]
/// Return metadata-only entries that use one exact tag.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `tag_utf8` must remain a readable, NUL-terminated UTF-8 string for this
/// call.
pub unsafe extern "C" fn tiez_core_get_tag_entries_json(
    handle: *mut TiezCoreHandle,
    tag_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let tag = required_utf8(tag_utf8, "tag_utf8")?;
        let snapshot = tag_entries_snapshot(handle, &tag)?;
        into_owned_c_string(tag_entries_json(&snapshot)?)
    })
}

#[no_mangle]
/// Create a saved tag without creating a clipboard entry.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `name_utf8` must remain a readable, NUL-terminated UTF-8 string for this
/// call.
pub unsafe extern "C" fn tiez_core_create_tag_json(
    handle: *mut TiezCoreHandle,
    name_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let name = required_utf8(name_utf8, "name_utf8")?;
        let mutation = create_tag(handle, &name)?;
        into_owned_c_string(tag_catalog_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Rename one non-protected tag on every entry through the shared history
/// mutation path, then merge its saved metadata.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// Both name pointers must remain readable, NUL-terminated UTF-8 strings for
/// this call.
pub unsafe extern "C" fn tiez_core_rename_tag_json(
    handle: *mut TiezCoreHandle,
    old_name_utf8: *const c_char,
    new_name_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let old_name = required_utf8(old_name_utf8, "old_name_utf8")?;
        let new_name = required_utf8(new_name_utf8, "new_name_utf8")?;
        let mutation = rename_tag(handle, &old_name, &new_name)?;
        into_owned_c_string(tag_catalog_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Permanently delete every entry using one non-protected tag, then remove its
/// saved metadata.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// `name_utf8` must remain a readable, NUL-terminated UTF-8 string for this
/// call.
pub unsafe extern "C" fn tiez_core_delete_tag_json(
    handle: *mut TiezCoreHandle,
    name_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let name = required_utf8(name_utf8, "name_utf8")?;
        let mutation = delete_tag(handle, &name)?;
        into_owned_c_string(tag_catalog_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Set or clear one tag's `#RRGGBB` display color.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// Both pointers must remain readable, NUL-terminated UTF-8 strings for this
/// call. An empty color clears the custom color.
pub unsafe extern "C" fn tiez_core_set_tag_color_json(
    handle: *mut TiezCoreHandle,
    name_utf8: *const c_char,
    color_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let name = required_utf8(name_utf8, "name_utf8")?;
        let color = required_utf8(color_utf8, "color_utf8")?;
        let color = (!color.trim().is_empty()).then_some(color.as_str());
        let mutation = set_tag_color(handle, &name, color)?;
        into_owned_c_string(tag_catalog_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Add a manual UTF-8 text entry and assign one tag through the shared history
/// ingestion and secure tag-update paths.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
/// Both pointers must remain readable, NUL-terminated UTF-8 strings for this
/// call.
pub unsafe extern "C" fn tiez_core_create_tagged_text_json(
    handle: *mut TiezCoreHandle,
    tag_utf8: *const c_char,
    content_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let tag = required_utf8(tag_utf8, "tag_utf8")?;
        let content = required_utf8(content_utf8, "content_utf8")?;
        let mutation = create_tagged_text(handle, &tag, &content)?;
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
/// Return the Tauri-compatible global search hotkey without other settings.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_search_hotkey_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let snapshot = handle.search_hotkey.snapshot()?;
        into_owned_c_string(search_hotkey_snapshot_json(&snapshot)?)
    })
}

#[no_mangle]
/// Persist the search hotkey after the native host has registered it.
///
/// # Safety
/// `handle` and `value_utf8` must be valid readable pointers.
pub unsafe extern "C" fn tiez_core_update_search_hotkey_json(
    handle: *mut TiezCoreHandle,
    value_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let value = required_utf8(value_utf8, "value_utf8")?;
        let mutation = handle.search_hotkey.update(&value)?;
        into_owned_c_string(search_hotkey_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Return only the Tauri-compatible rich/plain latest-paste shortcut settings.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_paste_hotkeys_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let snapshot = handle.paste_hotkeys.snapshot()?;
        into_owned_c_string(paste_hotkey_snapshot_json(&snapshot)?)
    })
}

#[no_mangle]
/// Persist one paste shortcut after the native host has registered it.
///
/// # Safety
/// `handle`, `kind_utf8`, and `value_utf8` must be valid readable pointers.
pub unsafe extern "C" fn tiez_core_update_paste_hotkey_json(
    handle: *mut TiezCoreHandle,
    kind_utf8: *const c_char,
    value_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let kind = required_utf8(kind_utf8, "kind_utf8")?;
        let value = required_utf8(value_utf8, "value_utf8")?;
        let mutation = handle.paste_hotkeys.update(&kind, &value)?;
        into_owned_c_string(paste_hotkey_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Paste the newest history entry as rich or plain content.
///
/// # Safety
/// `handle` must be live and `kind_utf8` must be readable UTF-8.
pub unsafe extern "C" fn tiez_core_paste_latest_json(
    handle: *mut TiezCoreHandle,
    kind_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let kind = required_utf8(kind_utf8, "kind_utf8")?;
        let mutation = paste_latest_history(handle, &kind)?;
        into_owned_c_string(mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Return exact sequential-paste settings and the current FIFO length.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_sequential_paste_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let snapshot = handle
            .inner
            .sequential_paste
            .lock()
            .map_err(|_| "SequentialPaste lock is poisoned".to_owned())?
            .snapshot()?;
        into_owned_c_string(sequential_paste_snapshot_json(&snapshot)?)
    })
}

#[no_mangle]
/// Persist `hotkey` or `enabled` after the native host applies registration.
///
/// # Safety
/// `handle`, `field_utf8`, and `value_utf8` must be valid readable pointers.
pub unsafe extern "C" fn tiez_core_update_sequential_paste_json(
    handle: *mut TiezCoreHandle,
    field_utf8: *const c_char,
    value_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let field = required_utf8(field_utf8, "field_utf8")?;
        let value = required_utf8(value_utf8, "value_utf8")?;
        let mutation = handle
            .inner
            .sequential_paste
            .lock()
            .map_err(|_| "SequentialPaste lock is poisoned".to_owned())?
            .update(&field, &value)?;
        into_owned_c_string(sequential_paste_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Paste and consume the next captured entry from the sequential FIFO.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_paste_next_sequential_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let (mutation, queued_items) = paste_next_sequential(handle)?;
        into_owned_c_string(sequential_paste_action_json(&mutation, queued_items)?)
    })
}

#[no_mangle]
/// Return AI settings and profile summaries without API keys.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_ai_settings_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let snapshot = ai_settings_snapshot(handle)?;
        into_owned_c_string(ai_settings_json(&snapshot)?)
    })
}

#[no_mangle]
/// Transactionally update AI settings. API keys are write-only across the ABI.
///
/// # Safety
/// `handle` must be live and `request_json_utf8` must be readable UTF-8 JSON.
pub unsafe extern "C" fn tiez_core_update_ai_settings_json(
    handle: *mut TiezCoreHandle,
    request_json_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let request_json = required_utf8(request_json_utf8, "request_json_utf8")?;
        let update = serde_json::from_str::<AiSettingsUpdate>(&request_json)
            .map_err(|error| format!("AI 设置 JSON 无效：{error}"))?;
        let mutation = update_ai_settings(handle, update)?;
        into_owned_c_string(ai_settings_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Run a minimal request against one saved profile without exposing its key.
/// This blocking call must be invoked from a non-UI thread by native hosts.
///
/// # Safety
/// `handle` must be live and `profile_id_utf8` must be readable UTF-8.
pub unsafe extern "C" fn tiez_core_probe_ai_profile_json(
    handle: *mut TiezCoreHandle,
    profile_id_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let profile_id = required_utf8(profile_id_utf8, "profile_id_utf8")?;
        let result = probe_ai_profile(handle, &profile_id)?;
        into_owned_c_string(ai_probe_json(&result)?)
    })
}

#[no_mangle]
/// Send one non-sensitive text-like history entry to the assigned AI profile.
/// The original entry is never modified. This blocking call must run off the UI thread.
///
/// # Safety
/// `handle` must be live and `action_utf8` must be readable UTF-8.
pub unsafe extern "C" fn tiez_core_run_ai_action_json(
    handle: *mut TiezCoreHandle,
    entry_id: i64,
    action_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let action = required_utf8(action_utf8, "action_utf8")?;
        let result = run_history_ai_action(handle, entry_id, &action)?;
        into_owned_c_string(ai_action_json(&result)?)
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
/// Return sanitized background synchronization status as allocated JSON.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_cloud_sync_status_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        into_owned_c_string(cloud_sync_status_json(&handle.cloud_sync.status())?)
    })
}

#[no_mangle]
/// Start the WinUI-owned background synchronization worker or reload its
/// schedule after settings change.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_start_cloud_sync(handle: *mut TiezCoreHandle) -> bool {
    with_ffi_result(false, || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        handle.cloud_sync.start()?;
        Ok(true)
    })
}

#[no_mangle]
/// Request an immediate pass and force publication of a full snapshot.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_request_cloud_sync(handle: *mut TiezCoreHandle) -> bool {
    with_ffi_result(false, || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        handle.cloud_sync.request_now()?;
        Ok(true)
    })
}

#[no_mangle]
/// Stop and join the background synchronization worker.
///
/// # Safety
///
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_stop_cloud_sync(handle: *mut TiezCoreHandle) {
    catch_ffi((), || {
        if let Some(handle) = unsafe { handle.as_ref() } {
            handle.cloud_sync.stop();
        } else {
            set_last_error("handle must not be null");
        }
    });
}

#[no_mangle]
/// Return sanitized clipboard-relay readiness without exposing credentials.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_relay_status_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let snapshot = handle.clipboard_relay.snapshot()?;
        into_owned_c_string(relay_snapshot_json(&snapshot)?)
    })
}

#[no_mangle]
/// Validate and store a write-only relay key in the operating-system vault.
///
/// # Safety
/// `handle` must be live and `shared_key_utf8` must be readable UTF-8.
pub unsafe extern "C" fn tiez_core_set_relay_shared_key_json(
    handle: *mut TiezCoreHandle,
    shared_key_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let shared_key = required_utf8(shared_key_utf8, "shared_key_utf8")?;
        let mutation = handle.clipboard_relay.set_key(&shared_key)?;
        into_owned_c_string(relay_key_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Generate and store a relay key, returning it exactly once for pairing.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_generate_relay_shared_key_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let mutation = handle.clipboard_relay.generate_key()?;
        if let Some(generated_key) = mutation.generated_key.as_deref() {
            if let Ok(mut filter) = handle.inner.capture.lock() {
                filter.note_self_write(generated_key);
            }
        }
        into_owned_c_string(relay_key_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Clear the relay key from the operating-system vault.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_clear_relay_shared_key_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let mutation = handle.clipboard_relay.clear_key()?;
        into_owned_c_string(relay_key_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Return the Tauri-compatible relay send/fetch hotkeys without credentials.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_relay_hotkeys_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let snapshot = handle.clipboard_relay.hotkey_snapshot()?;
        into_owned_c_string(relay_hotkey_snapshot_json(&snapshot)?)
    })
}

#[no_mangle]
/// Persist one relay hotkey after the native host has registered it.
///
/// # Safety
/// `handle`, `key_utf8`, and `value_utf8` must be valid readable pointers.
pub unsafe extern "C" fn tiez_core_update_relay_hotkey_json(
    handle: *mut TiezCoreHandle,
    key_utf8: *const c_char,
    value_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let key = required_utf8(key_utf8, "key_utf8")?;
        let value = required_utf8(value_utf8, "value_utf8")?;
        let mutation = handle.clipboard_relay.update_hotkey(&key, &value)?;
        into_owned_c_string(relay_hotkey_mutation_json(&mutation)?)
    })
}

#[no_mangle]
/// Encrypt and publish the exact current Windows clipboard text.
/// This blocking call must be invoked off the native UI thread.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_send_relay_clipboard_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let text = read_relay_clipboard_text()?;
        let result = handle.clipboard_relay.send(&text)?;
        into_owned_c_string(relay_send_json(&result)?)
    })
}

#[no_mangle]
/// Fetch the newest eligible relay message, copy it to Windows clipboard, and
/// durably record the authenticated receipt before publishing its ACK.
/// This blocking call must be invoked off the native UI thread.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_fetch_relay_clipboard_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let result = handle
            .clipboard_relay
            .fetch(|text| copy_relay_text(handle, text))?;
        into_owned_c_string(relay_fetch_json(&result)?)
    })
}

#[no_mangle]
/// Returns pairing status, compatible preferences, devices, and capped chat history.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_get_file_transfer_json(
    handle: *mut TiezCoreHandle,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let snapshot = handle.file_transfer.snapshot()?;
        into_owned_c_string(file_transfer_json(&snapshot)?)
    })
}

#[no_mangle]
/// Updates validated transfer settings and applies an explicit enabled toggle.
///
/// # Safety
/// `handle` must be live and `request_json_utf8` must be readable UTF-8 JSON.
pub unsafe extern "C" fn tiez_core_update_file_transfer_json(
    handle: *mut TiezCoreHandle,
    request_json_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let request_json = required_utf8(request_json_utf8, "request_json_utf8")?;
        let update = serde_json::from_str::<FileTransferPreferencesUpdate>(&request_json)
            .map_err(|error| format!("文件传输设置 JSON 无效：{error}"))?;
        let snapshot = handle.file_transfer.update_preferences(update)?;
        into_owned_c_string(file_transfer_json(&snapshot)?)
    })
}

#[no_mangle]
/// Starts the authenticated LAN server using saved preferences.
///
/// # Safety
/// `handle` must point to a live handle returned by `tiez_core_create`.
pub unsafe extern "C" fn tiez_core_start_file_transfer(handle: *mut TiezCoreHandle) -> bool {
    with_ffi_result(false, || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        handle.file_transfer.start()?;
        Ok(true)
    })
}

#[no_mangle]
/// Stops and joins the LAN server before returning.
///
/// # Safety
/// `handle` must be null or point to a live handle.
pub unsafe extern "C" fn tiez_core_stop_file_transfer(handle: *mut TiezCoreHandle) {
    catch_ffi((), || {
        if let Some(handle) = unsafe { handle.as_ref() } {
            handle.file_transfer.stop();
        }
    });
}

#[no_mangle]
/// Adds a PC-to-mobile text message and returns the refreshed snapshot.
///
/// # Safety
/// `handle` must be live and `text_utf8` must be readable UTF-8.
pub unsafe extern "C" fn tiez_core_send_transfer_text_json(
    handle: *mut TiezCoreHandle,
    text_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let text = required_utf8(text_utf8, "text_utf8")?;
        let snapshot = handle.file_transfer.send_text(&text)?;
        into_owned_c_string(file_transfer_json(&snapshot)?)
    })
}

#[no_mangle]
/// Registers local files as authenticated streaming downloads.
///
/// # Safety
/// `handle` must be live and `paths_json_utf8` must be a readable JSON string array.
pub unsafe extern "C" fn tiez_core_share_transfer_files_json(
    handle: *mut TiezCoreHandle,
    paths_json_utf8: *const c_char,
) -> *mut c_char {
    with_ffi_result(ptr::null_mut(), || {
        let handle =
            unsafe { handle.as_ref() }.ok_or_else(|| "handle must not be null".to_owned())?;
        let paths_json = required_utf8(paths_json_utf8, "paths_json_utf8")?;
        let paths = serde_json::from_str::<Vec<String>>(&paths_json)
            .map_err(|error| format!("共享文件路径 JSON 无效：{error}"))?
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let snapshot = handle.file_transfer.share_files(paths)?;
        into_owned_c_string(file_transfer_json(&snapshot)?)
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
/// `tiez_core_update_setting_json`, `tiez_core_get_search_hotkey_json`,
/// `tiez_core_update_search_hotkey_json`, `tiez_core_get_paste_hotkeys_json`,
/// `tiez_core_update_paste_hotkey_json`, `tiez_core_paste_latest_json`,
/// `tiez_core_get_sequential_paste_json`,
/// `tiez_core_update_sequential_paste_json`,
/// `tiez_core_paste_next_sequential_json`,
/// `tiez_core_get_cloud_sync_settings_json`,
/// `tiez_core_update_cloud_sync_settings_json`,
/// `tiez_core_probe_cloud_sync_json`, `tiez_core_get_cloud_sync_status_json`,
/// or `tiez_core_take_last_error`. A
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

        assert!(json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
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

        assert!(json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
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
            assert!(json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
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
            assert!(created_json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
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
            assert!(pin_json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
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
    fn clear_action_export_preserves_protected_entries() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let clear = CString::new("clear").unwrap();

        unsafe {
            let value = tiez_core_apply_action_json(handle, 0, clear.as_ptr());
            assert!(!value.is_null());
            let json = CStr::from_ptr(value).to_str().unwrap();
            assert!(json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
            assert!(json.contains("\"action\":\"clear\""));
            assert!(json.contains("\"requested_id\":0"));
            assert!(json.contains("\"removed\":true"));
            assert!(json.contains("\"generation\":2"));
            tiez_core_string_free(value);

            let query = CString::new("").unwrap();
            let snapshot_value = tiez_core_get_snapshot_json(handle, query.as_ptr());
            assert!(!snapshot_value.is_null());
            let snapshot = CStr::from_ptr(snapshot_value).to_str().unwrap();
            let root = serde_json::from_str::<serde_json::Value>(snapshot).unwrap();
            let ids = root["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["id"].as_i64().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(ids, vec![101, 102, 104, 107]);
            tiez_core_string_free(snapshot_value);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn transient_text_paste_export_accepts_utf8_and_rejects_empty_text() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let emoji = CString::new("🫶🚀").unwrap();
        let empty = CString::new("").unwrap();

        unsafe {
            assert!(tiez_core_paste_text(handle, emoji.as_ptr()));
            assert!(!tiez_core_paste_text(handle, empty.as_ptr()));

            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("no pasteable text"));
            tiez_core_string_free(error);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn emoji_favorite_exports_import_list_paste_and_remove_an_image() {
        let root = temporary_database_root("emoji-favorite-abi");
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("中文收藏.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([80, 160, 240, 255]))
            .save(&source)
            .unwrap();
        let source_utf8 = CString::new(source.to_string_lossy().as_bytes()).unwrap();
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));

        unsafe {
            let imported = tiez_core_import_emoji_favorite_json(handle, source_utf8.as_ptr());
            assert!(!imported.is_null());
            let imported_json = CStr::from_ptr(imported).to_str().unwrap();
            assert!(imported_json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
            assert!(imported_json.contains("\"action\":\"add\""));
            assert!(imported_json.contains("\"changed\":true"));
            assert!(imported_json.contains("中文收藏.png"));
            tiez_core_string_free(imported);

            let snapshot = tiez_core_get_emoji_favorites_json(handle);
            assert!(!snapshot.is_null());
            let snapshot_json = CStr::from_ptr(snapshot).to_str().unwrap();
            assert!(snapshot_json.contains("\"adapter\":\"memory\""));
            assert!(snapshot_json.contains("\"read_only\":false"));
            assert!(snapshot_json.contains("中文收藏.png"));
            tiez_core_string_free(snapshot);

            if !tiez_core_paste_emoji_favorite(handle, source_utf8.as_ptr()) {
                let error = tiez_core_take_last_error();
                let message = if error.is_null() {
                    "missing last error".to_owned()
                } else {
                    let message = CStr::from_ptr(error).to_string_lossy().into_owned();
                    tiez_core_string_free(error);
                    message
                };
                panic!("favorite paste failed: {message}");
            }

            let removed = tiez_core_remove_emoji_favorite_json(handle, source_utf8.as_ptr());
            assert!(!removed.is_null());
            let removed_json = CStr::from_ptr(removed).to_str().unwrap();
            assert!(removed_json.contains("\"action\":\"remove\""));
            assert!(removed_json.contains("\"items\":[]"));
            tiez_core_string_free(removed);
            assert!(source.is_file());

            assert!(!tiez_core_paste_emoji_favorite(
                handle,
                source_utf8.as_ptr()
            ));
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("不在 Emoji 收藏列表"));
            tiez_core_string_free(error);
            tiez_core_destroy(handle);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tag_catalog_exports_create_color_add_rename_and_delete_workflow() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let created_name = CString::new("待办").unwrap();
        let renamed_name = CString::new("项目").unwrap();
        let color = CString::new("#12abEF").unwrap();
        let content = CString::new("  保留空格的手动记录  ").unwrap();
        let protected = CString::new("password").unwrap();

        unsafe {
            let initial = tiez_core_get_tag_catalog_json(handle);
            assert!(!initial.is_null());
            let initial_json = CStr::from_ptr(initial).to_str().unwrap();
            assert!(initial_json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
            assert!(initial_json.contains("\"name\":\"迁移\""));
            assert!(initial_json.contains("\"name\":\"password\""));
            assert!(initial_json.contains("\"protected\":true"));
            tiez_core_string_free(initial);

            let created = tiez_core_create_tag_json(handle, created_name.as_ptr());
            assert!(!created.is_null());
            assert!(CStr::from_ptr(created)
                .to_str()
                .unwrap()
                .contains("\"action\":\"create\""));
            tiez_core_string_free(created);

            let colored =
                tiez_core_set_tag_color_json(handle, created_name.as_ptr(), color.as_ptr());
            assert!(!colored.is_null());
            assert!(CStr::from_ptr(colored)
                .to_str()
                .unwrap()
                .contains("\"color\":\"#12ABEF\""));
            tiez_core_string_free(colored);

            let added =
                tiez_core_create_tagged_text_json(handle, created_name.as_ptr(), content.as_ptr());
            assert!(!added.is_null());
            assert!(CStr::from_ptr(added)
                .to_str()
                .unwrap()
                .contains("\"action\":\"create-tagged-text\""));
            tiez_core_string_free(added);

            let entries = tiez_core_get_tag_entries_json(handle, created_name.as_ptr());
            assert!(!entries.is_null());
            let entries_json = CStr::from_ptr(entries).to_str().unwrap();
            assert!(entries_json.contains("\"tag\":\"待办\""));
            assert!(entries_json.contains("保留空格的手动记录"));
            assert!(entries_json.contains("\"total\":1"));
            tiez_core_string_free(entries);

            let renamed =
                tiez_core_rename_tag_json(handle, created_name.as_ptr(), renamed_name.as_ptr());
            assert!(!renamed.is_null());
            let renamed_json = CStr::from_ptr(renamed).to_str().unwrap();
            assert!(renamed_json.contains("\"action\":\"rename\""));
            assert!(renamed_json.contains("\"new_name\":\"项目\""));
            assert!(renamed_json.contains("\"affected\":1"));
            tiez_core_string_free(renamed);

            let project_entries = tiez_core_get_tag_entries_json(handle, renamed_name.as_ptr());
            assert!(!project_entries.is_null());
            assert!(CStr::from_ptr(project_entries)
                .to_str()
                .unwrap()
                .contains("\"total\":1"));
            tiez_core_string_free(project_entries);

            let deleted = tiez_core_delete_tag_json(handle, renamed_name.as_ptr());
            assert!(!deleted.is_null());
            let deleted_json = CStr::from_ptr(deleted).to_str().unwrap();
            assert!(deleted_json.contains("\"action\":\"delete\""));
            assert!(deleted_json.contains("\"affected\":1"));
            tiez_core_string_free(deleted);

            assert!(tiez_core_delete_tag_json(handle, protected.as_ptr()).is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("内置敏感标签不能删除"));
            tiez_core_string_free(error);

            let protected_content = CString::new("must not be stored in plaintext").unwrap();
            assert!(tiez_core_create_tagged_text_json(
                handle,
                protected.as_ptr(),
                protected_content.as_ptr(),
            )
            .is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("不能直接向内置敏感标签添加手动文本"));
            tiez_core_string_free(error);
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
            assert!(json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
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
        let hotkey_key = CString::new("app.hotkey").unwrap();
        let hotkey_value = CString::new("Ctrl+Shift+F23").unwrap();

        unsafe {
            let initial = tiez_core_get_settings_json(handle);
            assert!(!initial.is_null());
            let initial_json = CStr::from_ptr(initial).to_str().unwrap();
            assert!(initial_json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
            assert!(initial_json.contains("\"app.compact_mode\":\"false\""));
            assert!(initial_json.contains("\"app.hotkey\":\"Alt+C\""));
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

            let hotkey_mutation =
                tiez_core_update_setting_json(handle, hotkey_key.as_ptr(), hotkey_value.as_ptr());
            assert!(!hotkey_mutation.is_null());
            let hotkey_json = CStr::from_ptr(hotkey_mutation).to_str().unwrap();
            assert!(hotkey_json.contains("\"key\":\"app.hotkey\""));
            assert!(hotkey_json.contains("\"value\":\"Ctrl+Shift+F23\""));
            assert!(hotkey_json.contains("\"generation\":3"));
            tiez_core_string_free(hotkey_mutation);

            let updated = tiez_core_get_settings_json(handle);
            assert!(!updated.is_null());
            let updated_json = CStr::from_ptr(updated).to_str().unwrap();
            assert!(updated_json.contains("\"app.compact_mode\":\"true\""));
            assert!(updated_json.contains("\"app.hotkey\":\"Ctrl+Shift+F23\""));
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
    fn search_hotkey_export_is_sanitized_and_unavailable_without_production_data() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let value = CString::new("Ctrl+Alt+F").unwrap();

        unsafe {
            let snapshot = tiez_core_get_search_hotkey_json(handle);
            assert!(!snapshot.is_null());
            let json = CStr::from_ptr(snapshot).to_str().unwrap();
            assert!(json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
            assert!(json.contains("\"available\":false"));
            assert!(json.contains("\"hotkey\":\"\""));
            assert!(!json.contains("password"));
            tiez_core_string_free(snapshot);

            assert!(tiez_core_update_search_hotkey_json(handle, value.as_ptr()).is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("生产数据模式"));
            tiez_core_string_free(error);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn paste_hotkey_export_is_sanitized_and_latest_plain_rejects_non_text() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let plain = CString::new("plain").unwrap();
        let value = CString::new("Ctrl+Alt+P").unwrap();

        unsafe {
            let snapshot = tiez_core_get_paste_hotkeys_json(handle);
            assert!(!snapshot.is_null());
            let json = CStr::from_ptr(snapshot).to_str().unwrap();
            assert!(json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
            assert!(json.contains("\"available\":false"));
            assert!(json.contains("\"rich_hotkey\":\"\""));
            assert!(json.contains("\"plain_hotkey\":\"\""));
            assert!(!json.contains("mqtt_password"));
            tiez_core_string_free(snapshot);

            assert!(
                tiez_core_update_paste_hotkey_json(handle, plain.as_ptr(), value.as_ptr(),)
                    .is_null()
            );
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("生产数据模式"));
            tiez_core_string_free(error);

            let pasted = tiez_core_paste_latest_json(handle, plain.as_ptr());
            assert!(!pasted.is_null());
            let pasted_json = CStr::from_ptr(pasted).to_str().unwrap();
            assert!(pasted_json.contains("\"requested_id\":101"));
            assert!(pasted_json.contains("\"action\":\"paste-plain\""));
            tiez_core_string_free(pasted);
            tiez_core_destroy(handle);
        }

        let image_history = ClipboardHistory::in_memory(vec![HistoryItem {
            id: 1,
            content_type: "image".to_owned(),
            preview: "not-a-real-image".to_owned(),
            source_app: "Snipping Tool".to_owned(),
            captured_at: "Just now".to_owned(),
            is_pinned: false,
            tags: Vec::new(),
            is_sensitive: false,
        }]);
        let handle = Box::into_raw(Box::new(TiezCoreHandle::wrap(image_history)));
        unsafe {
            assert!(tiez_core_paste_latest_json(handle, plain.as_ptr()).is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("只粘贴文本"));
            tiez_core_string_free(error);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn latest_paste_honors_delete_after_paste_but_preserves_protected_entries() {
        let root = temporary_database_root("paste-hotkey-delete");
        std::fs::create_dir_all(&root).unwrap();
        let settings_path = root.join("settings.db");
        rusqlite::Connection::open(&settings_path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO settings (key, value) VALUES
                    ('app.delete_after_paste', 'true');",
            )
            .unwrap();
        let plain = CString::new("plain").unwrap();

        let unprotected = ClipboardHistory::in_memory(vec![HistoryItem {
            id: 1,
            content_type: "text".to_owned(),
            preview: "remove me after paste".to_owned(),
            source_app: "Notepad".to_owned(),
            captured_at: "Just now".to_owned(),
            is_pinned: false,
            tags: Vec::new(),
            is_sensitive: false,
        }]);
        let mut handle = TiezCoreHandle::wrap(unprotected);
        handle.paste_hotkeys = NativePasteHotkeys::new(&settings_path, false);
        let handle = Box::into_raw(Box::new(handle));
        unsafe {
            let value = tiez_core_paste_latest_json(handle, plain.as_ptr());
            assert!(!value.is_null());
            let json = CStr::from_ptr(value).to_str().unwrap();
            assert!(json.contains("\"removed\":true"));
            assert!(json.contains("粘贴后已删除未保护记录"));
            tiez_core_string_free(value);
            let snapshot = tiez_core_get_snapshot_json(handle, ptr::null());
            assert!(!snapshot.is_null());
            assert!(CStr::from_ptr(snapshot)
                .to_str()
                .unwrap()
                .contains("\"total\":0"));
            tiez_core_string_free(snapshot);
            tiez_core_destroy(handle);
        }

        let mut protected_handle = TiezCoreHandle::wrap(ClipboardHistory::synthetic());
        protected_handle.paste_hotkeys = NativePasteHotkeys::new(&settings_path, false);
        let protected_handle = Box::into_raw(Box::new(protected_handle));
        unsafe {
            let value = tiez_core_paste_latest_json(protected_handle, plain.as_ptr());
            assert!(!value.is_null());
            let json = CStr::from_ptr(value).to_str().unwrap();
            assert!(json.contains("\"requested_id\":101"));
            assert!(json.contains("\"removed\":false"));
            tiez_core_string_free(value);
            tiez_core_destroy(protected_handle);
        }

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sequential_paste_exports_fifo_delete_policy_and_failure_requeue() {
        let root = temporary_database_root("sequential-paste");
        std::fs::create_dir_all(&root).unwrap();
        let settings_path = root.join("settings.db");
        rusqlite::Connection::open(&settings_path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO settings (key, value) VALUES
                    ('app.sequential_mode', 'true'),
                    ('app.sequential_hotkey', 'Alt+V'),
                    ('app.delete_after_paste', 'true'),
                    ('mqtt_password', 'must-not-leak');",
            )
            .unwrap();
        let history = ClipboardHistory::in_memory(vec![
            HistoryItem {
                id: 1,
                content_type: "text".to_owned(),
                preview: "first".to_owned(),
                source_app: "Notepad".to_owned(),
                captured_at: "Just now".to_owned(),
                is_pinned: false,
                tags: Vec::new(),
                is_sensitive: false,
            },
            HistoryItem {
                id: 2,
                content_type: "text".to_owned(),
                preview: "second protected".to_owned(),
                source_app: "Notepad".to_owned(),
                captured_at: "Just now".to_owned(),
                is_pinned: false,
                tags: vec!["work".to_owned()],
                is_sensitive: false,
            },
        ]);
        let mut handle = TiezCoreHandle::wrap(history);
        handle.paste_hotkeys = NativePasteHotkeys::new(&settings_path, false);
        {
            let mut sequential = handle.inner.sequential_paste.lock().unwrap();
            *sequential = NativeSequentialPaste::new(&settings_path, false);
            sequential.record_capture(1).unwrap();
            sequential.record_capture(2).unwrap();
        }
        let handle = Box::into_raw(Box::new(handle));

        unsafe {
            let snapshot = tiez_core_get_sequential_paste_json(handle);
            assert!(!snapshot.is_null());
            let json = CStr::from_ptr(snapshot).to_str().unwrap();
            assert!(json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
            assert!(json.contains("\"enabled\":true"));
            assert!(json.contains("\"hotkey\":\"Alt+V\""));
            assert!(json.contains("\"queued_items\":2"));
            assert!(!json.contains("must-not-leak"));
            tiez_core_string_free(snapshot);

            let first = tiez_core_paste_next_sequential_json(handle);
            assert!(!first.is_null());
            let json = CStr::from_ptr(first).to_str().unwrap();
            assert!(json.contains("\"requested_id\":1"));
            assert!(json.contains("\"removed\":true"));
            assert!(json.contains("\"queued_items\":1"));
            assert!(json.contains("顺序粘贴后已删除未保护记录"));
            tiez_core_string_free(first);

            let second = tiez_core_paste_next_sequential_json(handle);
            assert!(!second.is_null());
            let json = CStr::from_ptr(second).to_str().unwrap();
            assert!(json.contains("\"requested_id\":2"));
            assert!(json.contains("\"removed\":false"));
            assert!(json.contains("\"queue_finished\":true"));
            tiez_core_string_free(second);

            let handle_ref = &*handle;
            {
                let mut sequential = handle_ref.inner.sequential_paste.lock().unwrap();
                sequential.record_capture(2).unwrap();
            }
            {
                let mut history = handle_ref.inner.history.lock().unwrap();
                history
                    .update_tags(2, vec!["sensitive".to_owned()])
                    .unwrap();
            }
            assert!(tiez_core_paste_next_sequential_json(handle).is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            tiez_core_string_free(error);
            assert_eq!(
                handle_ref
                    .inner
                    .sequential_paste
                    .lock()
                    .unwrap()
                    .queued_ids(),
                vec![2]
            );
            tiez_core_destroy(handle);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn captured_entries_reset_and_refill_the_sequential_fifo_after_paste() {
        let root = temporary_database_root("sequential-capture");
        std::fs::create_dir_all(&root).unwrap();
        let settings_path = root.join("settings.db");
        rusqlite::Connection::open(&settings_path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO settings (key, value) VALUES
                    ('app.sequential_mode', 'true'),
                    ('app.sequential_hotkey', 'Alt+V');",
            )
            .unwrap();
        let handle = TiezCoreHandle::wrap(ClipboardHistory::in_memory(Vec::new()));
        {
            let mut sequential = handle.inner.sequential_paste.lock().unwrap();
            *sequential = NativeSequentialPaste::new(&settings_path, false);
        }
        ingest_captured_text(&handle.inner, "first copied").unwrap();
        ingest_captured_text(&handle.inner, "second copied").unwrap();
        assert_eq!(
            handle.inner.sequential_paste.lock().unwrap().queued_ids(),
            vec![1, 2]
        );
        handle.inner.sequential_paste.lock().unwrap().mark_pasted();
        ingest_captured_text(&handle.inner, "new copy sequence").unwrap();
        assert_eq!(
            handle.inner.sequential_paste.lock().unwrap().queued_ids(),
            vec![3]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ai_exports_keep_keys_write_only_and_reject_unsafe_history() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let update = CString::new(
            r#"{
                "enabled":true,
                "profiles":[{
                    "id":"primary",
                    "base_url":"https://ai.example.test/v1",
                    "model":"中文模型",
                    "enable_thinking":false,
                    "api_key":"must-not-cross-boundary"
                }],
                "assigned_profile_task":"primary",
                "assigned_profile_mouthpiece":"primary",
                "assigned_profile_translate":"primary",
                "target_lang":"auto_zh_en",
                "thinking_budget":2048
            }"#,
        )
        .unwrap();

        unsafe {
            let mutation = tiez_core_update_ai_settings_json(handle, update.as_ptr());
            assert!(!mutation.is_null());
            let mutation_json = CStr::from_ptr(mutation).to_str().unwrap();
            assert!(mutation_json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
            assert!(mutation_json.contains("\"model\":\"中文模型\""));
            assert!(mutation_json.contains("\"api_key_configured\":true"));
            assert!(!mutation_json.contains("must-not-cross-boundary"));
            assert!(!mutation_json.contains("\"api_key\""));
            tiez_core_string_free(mutation);

            let snapshot = tiez_core_get_ai_settings_json(handle);
            assert!(!snapshot.is_null());
            let snapshot_json = CStr::from_ptr(snapshot).to_str().unwrap();
            assert!(snapshot_json.contains("\"assigned_profile_task\":\"primary\""));
            assert!(!snapshot_json.contains("must-not-cross-boundary"));
            tiez_core_string_free(snapshot);

            let action = CString::new("task").unwrap();
            assert!(tiez_core_run_ai_action_json(handle, 105, action.as_ptr()).is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("只处理文本"));
            tiez_core_string_free(error);

            assert!(tiez_core_run_ai_action_json(handle, 107, action.as_ptr()).is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("敏感剪贴板内容禁止"));
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
            assert!(initial_json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
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
    fn cloud_sync_status_export_is_sanitized_and_reports_unavailable_mode() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));

        unsafe {
            let value = tiez_core_get_cloud_sync_status_json(handle);
            assert!(!value.is_null());
            let json = CStr::from_ptr(value).to_str().unwrap();
            assert!(json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
            assert!(json.contains("\"state\":\"unavailable\""));
            assert!(json.contains("\"service_started\":false"));
            assert!(json.contains("\"settings_revision\":0"));
            assert!(!json.contains("password"));
            tiez_core_string_free(value);

            assert!(!tiez_core_start_cloud_sync(handle));
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("生产数据模式"));
            tiez_core_string_free(error);
            tiez_core_destroy(handle);
        }
    }

    #[test]
    fn relay_status_is_sanitized_and_unavailable_without_production_data() {
        let handle = Box::into_raw(Box::new(
            TiezCoreHandle::wrap(ClipboardHistory::synthetic()),
        ));
        let key = CString::new("01".repeat(32)).unwrap();
        let send_key = CString::new("app.relay_send_hotkey").unwrap();
        let send_value = CString::new("Ctrl+Alt+S").unwrap();

        unsafe {
            let value = tiez_core_get_relay_status_json(handle);
            assert!(!value.is_null());
            let json = CStr::from_ptr(value).to_str().unwrap();
            assert!(json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
            assert!(json.contains("\"available\":false"));
            assert!(json.contains("\"key_configured\":false"));
            assert!(!json.contains("shared_key"));
            assert!(!json.contains("password"));
            tiez_core_string_free(value);

            let value = tiez_core_get_relay_hotkeys_json(handle);
            assert!(!value.is_null());
            let json = CStr::from_ptr(value).to_str().unwrap();
            assert!(json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
            assert!(json.contains("\"available\":false"));
            assert!(json.contains("\"send_hotkey\":\"\""));
            assert!(json.contains("\"fetch_hotkey\":\"\""));
            assert!(!json.contains("password"));
            tiez_core_string_free(value);

            assert!(tiez_core_update_relay_hotkey_json(
                handle,
                send_key.as_ptr(),
                send_value.as_ptr()
            )
            .is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("生产数据模式"));
            tiez_core_string_free(error);

            assert!(tiez_core_set_relay_shared_key_json(handle, key.as_ptr()).is_null());
            let error = tiez_core_take_last_error();
            assert!(!error.is_null());
            assert!(CStr::from_ptr(error)
                .to_str()
                .unwrap()
                .contains("生产数据模式"));
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
            assert!(json.contains(&format!("\"abi_version\":{ABI_VERSION}")));
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
