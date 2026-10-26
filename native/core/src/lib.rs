//! Tauri-free Windows core. See native/README.md for the C ABI contract.
mod app_catalog;
mod app_icons;
mod capture;
mod cleanup_preview;
mod large_history;
pub mod external_open;
mod paste;
pub use capture::{CaptureService, CaptureStatus};
mod history;
pub use history::{HistoryQuery, HistoryService};
mod settings;
use serde::Deserialize;
use serde_json::{json, Value};
pub use settings::{CoreError, CoreResult, SettingsChange, SettingsService, SettingsSnapshot};
use std::collections::{BTreeMap, HashMap, HashSet};
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
    closing: HashSet<u64>,
}
struct CoreRuntime {
    apps: Arc<app_catalog::CatalogService>,
    icons: Arc<app_icons::IconService>,
    settings: SettingsService,
    history: Arc<Mutex<HistoryService>>,
    uploads: large_history::Uploads,
    capture: capture::CaptureService,
    opens: Arc<Mutex<external_open::OpenService>>,
    target: tiez_native_platform::target::TargetTracker,
    shell: tiez_native_platform::shell::ShellService,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
// ABI fixtures share the production singleton, but keep their own concurrent worker checks.
#[cfg(test)]
static ABI_TEST_LOCK: Mutex<()> = Mutex::new(());

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
    CleanupPreview {
        handle: u64,
        rules: String,
        text: String,
    },
    TagPage {
        handle: u64,
        query: history::TagQuery,
    },
    TagCreate {
        handle: u64,
        name: String,
        expected_revision: u64,
    },
    TagDeletePreview {
        handle: u64,
        name: String,
    },
    TagDelete {
        handle: u64,
        name: String,
        expected_revision: u64,
    },
    TagRename {
        handle: u64,
        old_name: String,
        new_name: String,
        expected_revision: u64,
    },
    TagColor {
        handle: u64,
        name: String,
        color: Option<String>,
        expected_revision: u64,
    },
    HistoryUploadLimits { handle: u64 },
    HistoryUploadBegin {
        handle: u64,
        version: u32,
        target: large_history::Target,
        expected_revision: u64,
        total_bytes: usize,
        sha256: String,
    },
    HistoryUploadAppend { handle: u64, token: String, offset: usize, data: String },
    HistoryUploadStatus { handle: u64, token: String },
    HistoryUploadCommit { handle: u64, token: String },
    HistoryUploadAbort { handle: u64, token: String },
    HistoryPage {
        handle: u64,
        query: HistoryQuery,
    },
    HistoryEntry {
        handle: u64,
        id: i64,
    },
    HistoryPreviewEntry {
        handle: u64,
        id: i64,
        expected_revision: u64,
        #[serde(default)]
        allow_sensitive: bool,
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
    HistoryTagSelection {
        handle: u64,
        id: i64,
    },
    HistoryTagsExact {
        handle: u64,
        id: i64,
        tags: Vec<String>,
        expected_revision: u64,
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
    HistoryDeleteMany {
        handle: u64,
        ids: Vec<i64>,
        expected_revision: u64,
    },
    HistoryClearPreview {
        handle: u64,
    },
    HistoryClear {
        handle: u64,
        expected_revision: u64,
    },
    HistoryDocumentPreview {
        handle: u64,
        id: i64,
        expected_revision: u64,
        #[serde(default)]
        allow_sensitive: bool,
    },
    HistoryImagePreview {
        handle: u64,
        id: i64,
        expected_revision: u64,
        #[serde(default)]
        allow_sensitive: bool,
        max_edge: u32,
    },
    HistoryChanges {
        handle: u64,
        after_revision: u64,
    },
    ContentOpen {
        handle: u64,
        id: i64,
        expected_revision: u64,
        #[serde(default)]
        allow_sensitive: bool,
    },
    ContentOpenPoll { handle: u64 },
    AppsStart { handle: u64, #[serde(default)] refresh: bool },
    AppsPoll { handle: u64, job_id: u64 },
    AppsCancel { handle: u64, job_id: u64 },
    AppsIconStart {
        handle: u64,
        catalog_job_id: u64,
        identity: app_catalog::CatalogIdentity,
        edge: u32,
    },
    AppsIconPoll {
        handle: u64,
        job_id: u64,
    },
    AppsIconCancel {
        handle: u64,
        job_id: u64,
    },
    ClipboardCopy {
        handle: u64,
        id: i64,
        #[serde(default)]
        expected_revision: Option<u64>,
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

impl Request {
    fn handle(&self) -> Option<u64> {
        match self {
            Self::Open { .. } => None,
            Self::Snapshot { handle, .. }
            | Self::Update { handle, .. }
            | Self::CleanupPreview { handle, .. }
            | Self::TagPage { handle, .. }
            | Self::TagCreate { handle, .. }
            | Self::TagDeletePreview { handle, .. }
            | Self::TagDelete { handle, .. }
            | Self::TagRename { handle, .. }
            | Self::TagColor { handle, .. }
            | Self::HistoryUploadLimits { handle }
            | Self::HistoryUploadBegin { handle, .. }
            | Self::HistoryUploadAppend { handle, .. }
            | Self::HistoryUploadStatus { handle, .. }
            | Self::HistoryUploadCommit { handle, .. }
            | Self::HistoryUploadAbort { handle, .. }
            | Self::HistoryPage { handle, .. }
            | Self::HistoryEntry { handle, .. }
            | Self::HistoryPreviewEntry { handle, .. }
            | Self::HistoryAdd { handle, .. }
            | Self::HistoryPin { handle, .. }
            | Self::HistoryEdit { handle, .. }
            | Self::HistoryTagSelection { handle, .. }
            | Self::HistoryTagsExact { handle, .. }
            | Self::HistoryTags { handle, .. }
            | Self::HistoryDelete { handle, .. }
            | Self::HistoryDeleteMany { handle, .. }
            | Self::HistoryClearPreview { handle, .. }
            | Self::HistoryClear { handle, .. }
            | Self::HistoryDocumentPreview { handle, .. }
            | Self::HistoryImagePreview { handle, .. }
            | Self::HistoryChanges { handle, .. }
            | Self::ContentOpen { handle, .. }
            | Self::ContentOpenPoll { handle, .. }
            | Self::AppsStart { handle, .. }
            | Self::AppsPoll { handle, .. }
            | Self::AppsCancel { handle, .. }
            | Self::AppsIconStart { handle, .. }
            | Self::AppsIconPoll { handle, .. }
            | Self::AppsIconCancel { handle, .. }
            | Self::ClipboardCopy { handle, .. }
            | Self::ClipboardPaste { handle, .. }
            | Self::ShellStart { handle, .. }
            | Self::ShellPoll { handle, .. }
            | Self::ShellStop { handle, .. }
            | Self::TargetStart { handle, .. }
            | Self::TargetStatus { handle, .. }
            | Self::TargetStop { handle, .. }
            | Self::TargetRestore { handle, .. }
            | Self::CaptureStart { handle, .. }
            | Self::CaptureStop { handle, .. }
            | Self::CaptureStatus { handle, .. }
            | Self::Close { handle, .. } => Some(*handle),
        }
    }
}

fn dispatch(bytes: &[u8]) -> CoreResult<Value> {
    let request: Request = serde_json::from_slice(bytes)
        .map_err(|_| CoreError::new("invalid_request", "Expected a valid UTF-8 JSON command"))?;
    let mut registry = REGISTRY
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| CoreError::new("internal", "Core registry unavailable"))?;
    if request.handle().is_some_and(|handle| registry.closing.contains(&handle))
        && !matches!(&request, Request::Close { .. } | Request::ContentOpenPoll { .. })
    {
        return Err(CoreError::new("core_closing", "Core shutdown is in progress; retry close if cleanup fails"));
    }
    if let Some(handle) = request.handle() {
        if let Some(runtime) = registry.services.get_mut(&handle) { runtime.uploads.reap(); }
    }
    match request {
        Request::Open { database_path } => {
            if !registry.services.is_empty() || !registry.closing.is_empty() {
                return Err(CoreError::new(
                    "already_open",
                    "Share the existing core between windows",
                ));
            }
            let history = Arc::new(Mutex::new(HistoryService::open(Path::new(&database_path))?));
            let service = SettingsService::open(Path::new(&database_path))?;
            let snapshot = service.snapshot()?;
            let opens = Arc::new(Mutex::new(external_open::OpenService::new(Arc::clone(&history), Path::new(&database_path).parent().unwrap().to_path_buf())?));
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
            // Icons resolve caller identities through this runtime's accepted catalog job.
            let apps = Arc::new(app_catalog::CatalogService::default());
            let icons = Arc::new(app_icons::IconService::new(Arc::clone(&apps)));
            registry.services.insert(
                handle,
                CoreRuntime {
                    apps,
                    icons,
                    settings: service,
                    history,
                    uploads: Default::default(),
                    capture,
                    opens,
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
        Request::CleanupPreview {
            handle,
            rules,
            text,
        } => {
            if !registry.services.contains_key(&handle) {
                return Err(invalid_handle());
            }
            // No borrowed service state: release the registry before compiling draft regexes.
            drop(registry);
            Ok(json!(cleanup_preview::preview(&rules, &text)?))
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
        Request::TagPage { handle, query } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .tag_page(query)?)),
        Request::TagCreate {
            handle,
            name,
            expected_revision,
        } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .create_tag(name, expected_revision)?)),
        Request::TagDeletePreview { handle, name } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .tag_delete_preview(name)?)),
        Request::TagDelete {
            handle,
            name,
            expected_revision,
        } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .delete_tag(name, expected_revision)?)),
        Request::TagRename {
            handle,
            old_name,
            new_name,
            expected_revision,
        } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .rename_tag(old_name, new_name, expected_revision)?)),
        Request::TagColor {
            handle,
            name,
            color,
            expected_revision,
        } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .set_tag_color(name, color, expected_revision)?)),
        Request::HistoryUploadLimits { handle } => {
            let runtime = registry.services.get(&handle).ok_or_else(invalid_handle)?;
            let history = runtime.history.lock().map_err(|_| CoreError::new("history", "History unavailable"))?;
            Ok(json!(large_history::limits(history.revision())))
        }
        Request::HistoryUploadBegin { handle, version, target, expected_revision, total_bytes, sha256 } => {
            let runtime = registry.services.get_mut(&handle).ok_or_else(invalid_handle)?;
            let history = runtime.history.lock().map_err(|_| CoreError::new("history", "History unavailable"))?;
            Ok(json!(runtime.uploads.begin(version, target, expected_revision, total_bytes, &sha256, &history)?))
        }
        Request::HistoryUploadAppend { handle, token, offset, data } => {
            let runtime = registry.services.get_mut(&handle).ok_or_else(invalid_handle)?;
            Ok(json!(runtime.uploads.append(&token, offset, &data)?))
        }
        Request::HistoryUploadStatus { handle, token } => {
            let runtime = registry.services.get_mut(&handle).ok_or_else(invalid_handle)?;
            Ok(json!(runtime.uploads.status(&token)?))
        }
        Request::HistoryUploadAbort { handle, token } => {
            let runtime = registry.services.get_mut(&handle).ok_or_else(invalid_handle)?;
            Ok(json!({ "aborted": runtime.uploads.abort(&token)? }))
        }
        Request::HistoryUploadCommit { handle, token } => {
            let runtime = registry.services.get_mut(&handle).ok_or_else(invalid_handle)?;
            let mut history = runtime.history.lock().map_err(|_| CoreError::new("history", "History unavailable"))?;
            Ok(json!(runtime.uploads.commit(&token, &mut history)?))
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
        Request::HistoryPreviewEntry {
            handle,
            id,
            expected_revision,
            allow_sensitive,
        } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .preview_entry(id, expected_revision, allow_sensitive)?)),
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
        Request::HistoryTagSelection { handle, id } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .tag_selection(id)?)),
        Request::HistoryTagsExact {
            handle,
            id,
            tags,
            expected_revision,
        } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .set_tags_exact(id, tags, expected_revision)?)),
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
        Request::HistoryDeleteMany {
            handle,
            ids,
            expected_revision,
        } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .delete_many(ids, expected_revision)?)),
        Request::HistoryClearPreview { handle } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .clear_preview()?)),
        Request::HistoryClear {
            handle,
            expected_revision,
        } => Ok(json!(registry
            .services
            .get_mut(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .clear(expected_revision)?)),
        Request::HistoryDocumentPreview {
            handle,
            id,
            expected_revision,
            allow_sensitive,
        } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .document_preview(id, expected_revision, allow_sensitive)?)),
        Request::HistoryImagePreview {
            handle,
            id,
            expected_revision,
            allow_sensitive,
            max_edge,
        } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .history
            .lock()
            .map_err(|_| CoreError::new("history", "History unavailable"))?
            .image_preview(
                id,
                expected_revision,
                allow_sensitive,
                max_edge
            )?)),
        Request::AppsStart { handle, refresh } => {
            let apps = Arc::clone(&registry.services.get(&handle).ok_or_else(invalid_handle)?.apps);
            drop(registry); apps.start(refresh)
        }
        Request::AppsPoll { handle, job_id } => {
            let apps = Arc::clone(&registry.services.get(&handle).ok_or_else(invalid_handle)?.apps);
            drop(registry); apps.poll(job_id)
        }
        Request::AppsCancel { handle, job_id } => {
            let apps = Arc::clone(&registry.services.get(&handle).ok_or_else(invalid_handle)?.apps);
            drop(registry); apps.cancel(job_id)
        }
        Request::AppsIconStart {
            handle,
            catalog_job_id,
            identity,
            edge,
        } => {
            let icons =
                Arc::clone(&registry.services.get(&handle).ok_or_else(invalid_handle)?.icons);
            drop(registry);
            icons.start(catalog_job_id, &identity, edge)
        }
        Request::AppsIconPoll { handle, job_id } => {
            let icons =
                Arc::clone(&registry.services.get(&handle).ok_or_else(invalid_handle)?.icons);
            drop(registry);
            icons.poll(job_id)
        }
        Request::AppsIconCancel { handle, job_id } => {
            let icons =
                Arc::clone(&registry.services.get(&handle).ok_or_else(invalid_handle)?.icons);
            drop(registry);
            icons.cancel(job_id)
        }
        Request::ContentOpen { handle, id, expected_revision, allow_sensitive } => {
            let runtime = registry.services.get(&handle).ok_or_else(invalid_handle)?;
            let values = runtime.settings.snapshot()?.values;
            let opens = Arc::clone(&runtime.opens);
            // Windows handlers can be slow. Keep unrelated settings/history/core calls available.
            drop(registry);
            let mut opens = opens.lock().map_err(|_| CoreError::new("open_unavailable", "Open service unavailable"))?;
            let result = opens.open_at_revision(id, expected_revision, allow_sensitive, &values)?;
            let route = match result.route { tiez_native_platform::open::Route::System => "system",
                tiez_native_platform::open::Route::Desktop => "desktop", tiez_native_platform::open::Route::Packaged => "packaged" };
            Ok(json!({ "id": result.id, "history_revision": result.history_revision, "route": route,
                "exported": result.exported, "edit_watch_required": result.edit_watch_required,
                "edit_writeback_available": false, "usage_updated": result.usage_updated,
                "usage_revision": result.usage_revision, "usage_error": result.usage_error }))
        }
        Request::ContentOpenPoll { handle } => {
            let opens = Arc::clone(&registry.services.get(&handle).ok_or_else(invalid_handle)?.opens);
            drop(registry);
            let mut opens = opens.lock().map_err(|_| CoreError::new("open_unavailable", "Open service unavailable"))?;
            let cleanup_error = if opens.status().closed { opens.close().err() } else { opens.reap_expired().err() };
            let status = opens.status();
            Ok(json!({ "active_exports": status.active_exports, "export_bytes": status.export_bytes,
                "closed": status.closed, "cleanup_error": cleanup_error }))
        }
        Request::ClipboardCopy {
            handle,
            id,
            expected_revision,
            plain_text,
            allow_sensitive,
        } => Ok(json!(registry
            .services
            .get(&handle)
            .ok_or_else(invalid_handle)?
            .capture
            .copy_entry_at_revision(id, plain_text, allow_sensitive, expected_revision)?)),
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
            // Reserve this runtime's slot until close succeeds or the stopped owner is restored.
            registry.closing.insert(handle);
            drop(registry);
            service.uploads.clear();
            // No registry/history/policy guards held while joining or waiting for a native open.
            let _ = service.shell.stop();
            let shell = shell_status(service.shell.status());
            let capture = service.capture.stop();
            let _ = service.target.stop();
            let target = target_status(service.target.status());
            // Signal both cancellations first, then join: a blocked scan must never delay the icon
            // workers' cancellation, and neither join runs under the registry lock.
            service.apps.cancel_shutdown();
            service.icons.cancel_shutdown();
            service.apps.join_shutdown();
            // Icon workers can be inside a blocking Shell extraction: cancel and join them here,
            // still outside the registry lock.
            service.icons.join_shutdown();
            let cleanup = service.opens.lock()
                .map_err(|_| CoreError::new("open_unavailable", "Open service unavailable"))
                .and_then(|mut opens| opens.close());
            let mut registry = REGISTRY.get().unwrap().lock()
                .map_err(|_| CoreError::new("internal", "Core registry unavailable"))?;
            match cleanup {
                Ok(removed) => {
                    registry.closing.remove(&handle);
                    Ok(json!({ "closed": true, "capture": capture, "target": target, "shell": shell,
                        "removed_exports": removed }))
                },
                Err(failure) => {
                    // Keep the closed opener and stopped services owned: caller can retry close.
                    registry.services.insert(handle, service);
                    Err(failure)
                }
            }
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
        let _registry_guard = ABI_TEST_LOCK.lock().unwrap();
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
        let preview = call(
            json!({"op":"cleanup_preview","handle":handle,"rules":"foo => bar","text":"  foo\r\n "}),
        );
        assert_eq!(preview["result"]["text"], "  bar\n ");
        assert_eq!(preview["result"]["matches"], 1);
        assert_eq!(
            call(json!({"op":"snapshot","handle":handle}))["result"]["snapshot"]["revision"],
            0
        );
        assert_eq!(
            call(json!({"op":"cleanup_preview","handle":0,"rules":"","text":""}))["error"]["code"],
            "invalid_handle"
        );
        assert_eq!(
            call(
                json!({"op":"cleanup_preview","handle":handle,"rules":"[ => bad","text":"secret sample"})
            )["error"]["code"],
            "invalid_rule"
        );

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

    #[test]
    fn open_abi_rejects_ui_bodies_and_releases_registry_before_waiting() {
        let _registry_guard = ABI_TEST_LOCK.lock().unwrap();
        let root=std::env::temp_dir().join(format!("TieZ-open-abi-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let opened=call(json!({"op":"open","database_path":root.join("fixture.db")}));
        let handle=opened["result"]["handle"].as_u64().unwrap();
        assert!(call(json!({"op":"update","handle":handle,"expected_revision":0,"patch":{"app.text":root.join("missing.exe").to_str().unwrap()}}))["ok"].as_bool().unwrap());
        let (id,revision,opens)={
            let registry=REGISTRY.get().unwrap().lock().unwrap();let runtime=&registry.services[&handle];
            let mut h=runtime.history.lock().unwrap();let saved=h.add_manual("text".into(),"  authoritative\r\n ".into(),None,vec![]).unwrap().id;
            let mut session=h.entry(saved).unwrap();session.id=0;let id=h.record_processed(session,false).unwrap().id;
            (id,h.revision(),runtime.opens.clone())
        };
        assert!(id<0);
        let rejected=call(json!({"op":"content_open","handle":handle,"id":id,"expected_revision":revision,"content":"injected UI body"}));
        assert_eq!(rejected["error"]["code"],"invalid_request");
        assert_eq!(call(json!({"op":"content_open","handle":handle,"id":id,"expected_revision":revision-1}))["error"]["code"],"history_conflict");
        assert_eq!(call(json!({"op":"content_open","handle":handle,"id":id,"expected_revision":revision}))["error"]["code"],"open_handler_missing");
        assert_eq!(call(json!({"op":"content_open_poll","handle":handle}))["result"]["active_exports"],0);
        // Hold only the opener: the poll must wait without retaining the global registry lock.
        let held=opens.lock().unwrap();let before=Arc::strong_count(&opens);
        let poll=std::thread::spawn(move||call(json!({"op":"content_open_poll","handle":handle})));
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(2);
        while Arc::strong_count(&opens)==before && std::time::Instant::now()<deadline {std::thread::sleep(std::time::Duration::from_millis(1));}
        let progressed=Arc::strong_count(&opens)>before;
        let (tx,rx)=std::sync::mpsc::channel();
        let snapshot=std::thread::spawn(move||{let result=call(json!({"op":"snapshot","handle":handle}));let _=tx.send(result);});
        let independent=rx.recv_timeout(std::time::Duration::from_secs(2));
        drop(held);assert_eq!(poll.join().unwrap()["ok"],true);snapshot.join().unwrap();
        let closed=call(json!({"op":"close","handle":handle}));drop(opens);
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(closed["result"]["closed"],true);assert!(progressed);assert_eq!(independent.unwrap()["result"]["snapshot"]["revision"],1);
    }
}
