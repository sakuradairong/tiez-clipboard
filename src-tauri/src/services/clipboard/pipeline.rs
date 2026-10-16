use crate::app_state::{AppDataDir, PasteQueue, SessionHistory, SettingsState};
use crate::database::DbState;
use crate::database::{calc_image_hash, calc_text_hash, is_text_type};
use crate::domain::models::ClipboardEntry;
use crate::infrastructure::windows_api::window_tracker::{
    get_clipboard_source_app_info, ActiveAppInfo,
};
use crate::services::clipboard::utils::*;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

use super::processing::normalize_text_preserving_edge_whitespace;

pub use super::data::ClipboardData;

pub struct PipelineContext {
    pub data: ClipboardData,
    pub app_handle: AppHandle,
    pub source_app: String,
    pub source_app_path: Option<String>,
    pub timestamp: i64,
    pub entry: Option<ClipboardEntry>,
    pub should_stop: bool,
    pub pending_removals: Vec<i64>,
    pub reuse_session_id: Option<i64>,
}

impl PipelineContext {
    pub fn new(
        app_handle: AppHandle,
        data: ClipboardData,
        source_snapshot: Option<ActiveAppInfo>,
    ) -> Self {
        let active_app = source_snapshot.unwrap_or_else(get_clipboard_source_app_info);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        Self {
            data,
            app_handle,
            source_app: active_app.app_name,
            source_app_path: active_app.process_path,
            timestamp,
            entry: None,
            should_stop: false,
            pending_removals: Vec::new(),
            reuse_session_id: None,
        }
    }
}

pub trait PipelineStage {
    fn process(&self, context: &mut PipelineContext);
}

pub struct ClipboardPipeline {
    stages: Vec<Box<dyn PipelineStage + Send + Sync>>,
}

impl ClipboardPipeline {
    pub fn new() -> Self {
        Self {
            stages: vec![
                Box::new(DiscoveryStage),
                Box::new(TransformationStage),
                Box::new(ValidationStage),
                Box::new(PersistenceStage),
                Box::new(DistributionStage),
            ],
        }
    }

    pub fn execute(&self, context: &mut PipelineContext) {
        for stage in &self.stages {
            stage.process(context);
            if context.should_stop {
                break;
            }
        }
    }
}

// Stage 1: Discovery
pub struct DiscoveryStage;
impl PipelineStage for DiscoveryStage {
    fn process(&self, ctx: &mut PipelineContext) {
        ctx.entry = Some(super::processing::discover(&ctx.data, &ctx.source_app, ctx.source_app_path.as_deref(), ctx.timestamp));
    }
}

// Stage 2: Transformation
pub struct TransformationStage;
impl PipelineStage for TransformationStage {
    fn process(&self, ctx: &mut PipelineContext) {
        let settings = ctx.app_handle.state::<SettingsState>();
        let options = super::processing::TransformOptions {
            persistent: settings.persistent.load(Ordering::Relaxed),
            cleanup_rules: settings.cleanup_rules.lock().unwrap().clone(),
            app_cleanup_policies: settings.app_cleanup_policies.lock().unwrap().clone(),
            privacy_protection: settings.privacy_protection.load(Ordering::Relaxed),
            privacy_kinds: settings.privacy_protection_kinds.lock().unwrap().clone(),
            privacy_custom_rules: settings.privacy_protection_custom_rules.lock().unwrap().clone(),
        };
        let app_data_dir = ctx.app_handle.state::<AppDataDir>();
        let data_dir = app_data_dir.0.lock().unwrap().clone();
        ctx.should_stop = !super::processing::transform(ctx.entry.as_mut().unwrap(), &options, &data_dir);
    }
}

// Stage 3: Validation (Deduplication & Sequential Echo)
pub struct ValidationStage;
impl PipelineStage for ValidationStage {
    fn process(&self, ctx: &mut PipelineContext) {
        let settings = ctx.app_handle.state::<SettingsState>();

        // Recent paste echo check
        {
            let entry = ctx.entry.as_ref().unwrap();
            let queue_state = ctx.app_handle.state::<PasteQueue>();
            let queue = queue_state.0.lock().unwrap();
            if queue.last_action_was_paste {
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                if now_ms.saturating_sub(queue.last_paste_timestamp_ms) >= 10_000 {
                    drop(queue);
                    crate::services::clipboard_ops::clear_recent_paste_marker(&ctx.app_handle);
                } else {
                    let entry_fingerprint = build_clipboard_text_fingerprint(
                        &entry.content_type,
                        &entry.content,
                        entry.html_content.as_deref(),
                    );
                    let exact_match = queue.last_pasted_content.as_deref() == Some(&entry.content);
                    let fingerprint_match = !entry_fingerprint.is_empty()
                        && queue.last_pasted_fingerprint.as_deref()
                            == Some(entry_fingerprint.as_str());
                    if exact_match || fingerprint_match {
                        println!("Ignoring echo paste from recent paste marker");
                        drop(queue);
                        crate::services::clipboard_ops::clear_recent_paste_marker(&ctx.app_handle);
                        ctx.should_stop = true;
                        return;
                    }
                }
            }
        }

        // Deduplication
        if settings.deduplicate.load(Ordering::Relaxed) {
            let persistent_enabled = settings.persistent.load(Ordering::Relaxed);
            let db_state = ctx.app_handle.state::<DbState>();
            let conn = db_state.conn.lock().unwrap();

            let mut existing_id = None;
            let (content, content_type, html_content) = {
                let e = ctx.entry.as_ref().unwrap();
                (
                    e.content.clone(),
                    e.content_type.clone(),
                    e.html_content.clone(),
                )
            };

            // Try precise match and line-ending-normalized/hash match.
            let normalized_content = normalize_text_preserving_edge_whitespace(&content);
            let normalized_html = |html: &str| html.trim().replace("\r\n", "\n");
            let normalized_content_hash = calc_text_hash(&normalized_content) as i64;
            let recent_self_copy_window = {
                let last_app_time = crate::LAST_APP_SET_TIMESTAMP.load(Ordering::Relaxed);
                let now_secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                last_app_time != 0 && now_secs.saturating_sub(last_app_time) < 10
            };
            let htmls_equivalent = |a: Option<&str>, b: Option<&str>| -> bool {
                match (a, b) {
                    (None, None) => true,
                    (Some(left), Some(right)) => normalized_html(left) == normalized_html(right),
                    _ => false,
                }
            };
            let rich_text_html_matches = |id: i64| -> bool {
                if let Ok(Some((stored_content, c_type, h_content))) = db_state
                    .repo
                    .get_entry_content_with_html_with_conn(&conn, id)
                {
                    if c_type != "rich_text" {
                        return false;
                    }
                    if htmls_equivalent(html_content.as_deref(), h_content.as_deref()) {
                        return true;
                    }
                    return recent_self_copy_window
                        && calc_text_hash(&stored_content) as i64 == normalized_content_hash;
                }
                false
            };

            // For rich text, we want to deduplicate against all text types
            let types_to_check = if content_type == "rich_text" {
                vec!["rich_text", "text", "code", "url"]
            } else {
                vec![content_type.as_str()]
            };

            for t in types_to_check {
                if let Ok(Some(id)) =
                    db_state
                        .repo
                        .find_by_content_with_conn(&conn, &content, Some(t))
                {
                    if content_type == "rich_text"
                        && t == "rich_text"
                        && !rich_text_html_matches(id)
                    {
                        continue;
                    }
                    existing_id = Some(id);
                    break;
                }
            }

            if persistent_enabled {
                if let Some(id) = existing_id {
                    // Instead of deleting, we set the entry ID so PersistenceStage performs an UPDATE
                    // This ensures the item is "moved to top" without risking data loss
                    let entry_mut = ctx.entry.as_mut().unwrap();
                    entry_mut.id = id;
                }
            }

            // Session history cleanup
            let session_history = ctx.app_handle.state::<SessionHistory>();
            let mut removed_ids = Vec::new();
            let mut reuse_session_id: Option<i64> = None;
            {
                let session = session_history.0.lock().unwrap();
                let entry = ctx.entry.as_ref().expect("entry exists");
                let normalized_content_hash = calc_text_hash(&entry.content) as i64;
                let entry_image_hash = if entry.content_type == "image" {
                    calc_image_hash(&entry.content)
                } else {
                    None
                };
                for item in session.iter() {
                    let item_normalized_hash = calc_text_hash(&item.content) as i64;
                    let rich_text_match =
                        if entry.content_type == "rich_text" && item.content_type == "rich_text" {
                            htmls_equivalent(
                                item.html_content.as_deref(),
                                entry.html_content.as_deref(),
                            ) || (recent_self_copy_window
                                && item_normalized_hash == normalized_content_hash)
                        } else {
                            true
                        };
                    let image_match = entry.content_type == "image"
                        && item.content_type == "image"
                        && entry_image_hash.is_some()
                        && calc_image_hash(&item.content) == entry_image_hash;
                    let text_match = (item.content == entry.content
                        || item_normalized_hash == normalized_content_hash)
                        && rich_text_match;
                    let match_found = image_match || text_match;
                    if match_found {
                        removed_ids.push(item.id);
                        if !persistent_enabled {
                            reuse_session_id = Some(item.id);
                        }
                    }
                }
            }
            if !persistent_enabled {
                if let Some(reuse_id) = reuse_session_id {
                    ctx.reuse_session_id = Some(reuse_id);
                    if let Some(entry_mut) = ctx.entry.as_mut() {
                        entry_mut.id = reuse_id;
                    }
                    removed_ids.retain(|id| *id != reuse_id);
                }
            }
            ctx.pending_removals.extend(removed_ids);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_text_preserving_edge_whitespace;

    #[test]
    fn normalization_preserves_trailing_spaces() {
        assert_eq!(
            normalize_text_preserving_edge_whitespace("hello "),
            "hello "
        );
        assert_eq!(
            normalize_text_preserving_edge_whitespace(" hello"),
            " hello"
        );
        assert_eq!(
            normalize_text_preserving_edge_whitespace("hello\r\n"),
            "hello\n"
        );
    }
}

// Stage 4: Persistence
pub struct PersistenceStage;
impl PipelineStage for PersistenceStage {
    fn process(&self, ctx: &mut PipelineContext) {
        let entry = ctx.entry.as_mut().unwrap();
        let settings = ctx.app_handle.state::<SettingsState>();
        let db_state = ctx.app_handle.state::<DbState>();

        if settings.persistent.load(Ordering::Relaxed) {
            let app_data_dir = ctx.app_handle.state::<AppDataDir>();
            let data_dir = app_data_dir.0.lock().unwrap().clone();
            let conn = db_state.conn.lock().unwrap();

            if let Ok(id) = db_state.repo.save_with_conn(&conn, entry, Some(&data_dir)) {
                entry.id = id;
                if let Ok(deleted_ids) = db_state
                    .repo
                    .enforce_limit_with_conn(&conn, Some(&data_dir))
                {
                    for rid in deleted_ids {
                        let _ = ctx.app_handle.emit("clipboard-removed", rid);
                    }
                }
            }
        } else {
            // Session-only items
            if let Some(reuse_id) = ctx.reuse_session_id {
                let session_history = ctx.app_handle.state::<SessionHistory>();
                let mut updated_entry: Option<ClipboardEntry> = None;
                {
                    let mut session = session_history.0.lock().unwrap();
                    if let Some(existing) = session.iter_mut().find(|i| i.id == reuse_id) {
                        let preserved_tags = existing.tags.clone();
                        let preserved_pinned = existing.is_pinned;
                        let preserved_pinned_order = existing.pinned_order;
                        let preserved_use_count = existing.use_count;

                        existing.content_type = entry.content_type.clone();
                        existing.content = entry.content.clone();
                        existing.html_content = entry.html_content.clone();
                        existing.source_app = entry.source_app.clone();
                        existing.source_app_path = entry.source_app_path.clone();
                        existing.timestamp = entry.timestamp;
                        existing.preview = entry.preview.clone();
                        existing.is_external = entry.is_external;
                        existing.file_preview_exists = entry.file_preview_exists;
                        existing.is_pinned = preserved_pinned;
                        existing.pinned_order = preserved_pinned_order;
                        existing.tags = if entry.tags.is_empty() {
                            preserved_tags
                        } else {
                            entry.tags.clone()
                        };
                        existing.use_count = preserved_use_count + 1;

                        updated_entry = Some(existing.clone());
                    }
                }

                if let Some(updated) = updated_entry {
                    *entry = updated;
                    return;
                }
            }

            // Use a unique negative ID for new session-only items
            let id = -(SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_micros() as i64
                / 1000);
            entry.id = id;
            let session_history = ctx.app_handle.state::<SessionHistory>();
            let mut session = session_history.0.lock().unwrap();
            session.push_back(entry.clone());
            if session.len() > 500 {
                if let Some(removed) = session.pop_front() {
                    let _ = ctx.app_handle.emit("clipboard-removed", removed.id);
                }
            }
        }
    }
}

// Stage 5: Distribution
pub struct DistributionStage;
impl PipelineStage for DistributionStage {
    fn process(&self, ctx: &mut PipelineContext) {
        let entry = ctx.entry.as_ref().unwrap();
        let settings = ctx.app_handle.state::<SettingsState>();

        if entry.id == 0 && settings.persistent.load(Ordering::Relaxed) {
            return; // Failed to save
        }

        if !ctx.pending_removals.is_empty() {
            let mut pending = std::mem::take(&mut ctx.pending_removals);
            pending.retain(|id| *id != entry.id);
            if !pending.is_empty() {
                let unique: std::collections::HashSet<i64> = pending.into_iter().collect();
                {
                    let session_history = ctx.app_handle.state::<SessionHistory>();
                    let mut session = session_history.0.lock().unwrap();
                    session.retain(|item| !unique.contains(&item.id));
                }
                for rid in unique {
                    let _ = ctx.app_handle.emit("clipboard-removed", rid);
                }
            }
        }

        // Sequential Queue updates
        if settings.sequential_mode.load(Ordering::Relaxed) {
            let queue_state = ctx.app_handle.state::<PasteQueue>();
            let mut queue = queue_state.0.lock().unwrap();
            if queue.last_action_was_paste {
                queue.items.clear();
                queue.last_action_was_paste = false;
                queue.last_pasted_content = None;
                queue.last_pasted_fingerprint = None;
                queue.last_paste_timestamp_ms = 0;
            }
            queue.items.push_back(entry.id);
        }

        // Sound
        if settings.sound_enabled.load(Ordering::Relaxed) {
            let _ = ctx.app_handle.emit("play-sound", "copy");
        }

        // Notify
        let _ = ctx
            .app_handle
            .emit("clipboard-updated", truncate_entry_for_ui(entry.clone()));

        if settings.persistent.load(Ordering::Relaxed) && entry.id > 0 {
            crate::services::cloud_sync::request_cloud_sync(ctx.app_handle.clone());
        }
    }
}
