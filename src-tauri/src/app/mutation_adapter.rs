use crate::app_state::{AppDataDir, EncryptionQueueState, SessionHistory};
use crate::database::{has_sensitive_tag, DbState};
use crate::domain::models::ClipboardEntry;
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use crate::infrastructure::repository::tag_repo::TagRepository;
use crate::services::encryption_queue::{EncryptionAction, EncryptionJob};
use tauri::{AppHandle, Emitter, Manager};
use tiez_core::production_mutation::{
    clear_unprotected, complete_session_tags, plan_delete, plan_pin, plan_sensitivity_transition,
    plan_session_tags, replace_session_id, MutationRecord, PinMutationRecord, PinStoragePlan,
    SensitivityTransition, TagMutationRecord,
};

/// Production adapter for clipboard-history mutations.
///
/// Shared core policy updates session state and identifies persistent work;
/// this adapter keeps Tauri-specific storage, events, and cloud sync together.
pub(crate) struct TauriMutationAdapter<'a> {
    state: &'a DbState,
    app_handle: &'a AppHandle,
}

impl<'a> TauriMutationAdapter<'a> {
    pub(crate) fn new(state: &'a DbState, app_handle: &'a AppHandle) -> Self {
        Self { state, app_handle }
    }

    pub(crate) fn delete(
        &self,
        session: &SessionHistory,
        app_data: &AppDataDir,
        entry_id: i64,
    ) -> AppResult<()> {
        let plan = {
            let mut session_items = session.0.lock().unwrap();
            plan_delete(&mut session_items, entry_id)
        };

        if let Some(persisted_id) = plan.persisted_id {
            let data_dir = app_data.0.lock().unwrap();
            self.state.repo.delete(persisted_id, Some(&data_dir))?;
        }

        self.notify_changed();
        Ok(())
    }

    pub(crate) fn clear(&self, session: &SessionHistory, app_data: &AppDataDir) -> AppResult<()> {
        {
            let mut session_items = session.0.lock().unwrap();
            clear_unprotected(&mut session_items);
        }

        let data_dir = app_data.0.lock().unwrap();
        self.state
            .repo
            .clear(Some(&data_dir))
            .map_err(AppError::from)?;
        self.notify_changed();
        Ok(())
    }

    pub(crate) fn update_pinned_order(&self, orders: Vec<(i64, i64)>) -> AppResult<()> {
        self.state
            .repo
            .update_pinned_order(orders)
            .map_err(AppError::from)?;
        self.notify_changed();
        Ok(())
    }

    pub(crate) fn toggle_pin(
        &self,
        session: &SessionHistory,
        app_data: &AppDataDir,
        entry_id: i64,
        is_pinned: bool,
    ) -> AppResult<i64> {
        let storage_plan = {
            let mut session_items = session.0.lock().unwrap();
            plan_pin(&mut session_items, entry_id, is_pinned)
        };
        let mut real_id = entry_id;
        let connection = self.state.conn.lock().unwrap();

        match storage_plan {
            PinStoragePlan::SessionOnly => {}
            PinStoragePlan::ToggleExisting { entry_id } => real_id = entry_id,
            PinStoragePlan::PersistThenToggle { session_id, entry } => {
                let data_dir = app_data.0.lock().unwrap().clone();
                if let Ok(new_id) =
                    self.state
                        .repo
                        .save_with_conn(&connection, &entry, Some(&data_dir))
                {
                    real_id = new_id;
                    if let Ok(deleted_ids) = self
                        .state
                        .repo
                        .enforce_limit_with_conn(&connection, Some(&data_dir))
                    {
                        for deleted_id in deleted_ids {
                            let _ = self.app_handle.emit("clipboard-removed", deleted_id);
                        }
                    }

                    let mut session_items = session.0.lock().unwrap();
                    replace_session_id(&mut session_items, session_id, new_id);
                }
            }
        }

        if real_id > 0 {
            self.state
                .repo
                .toggle_pin_with_conn(&connection, real_id, is_pinned)
                .map_err(AppError::from)?;
        }
        drop(connection);
        self.notify_changed();
        Ok(real_id)
    }

    pub(crate) fn update_tags(
        &self,
        session: &SessionHistory,
        app_data: &AppDataDir,
        entry_id: i64,
        tags: Vec<String>,
    ) -> AppResult<i64> {
        if entry_id < 0 {
            let mut session_items = session.0.lock().unwrap();
            let plan = plan_session_tags(&session_items, entry_id, tags)
                .map_err(|_| AppError::Validation("Item not found".to_owned()))?;
            let data_dir = app_data.0.lock().unwrap().clone();
            let new_id = self.state.repo.save(&plan.entry, Some(&data_dir))?;
            complete_session_tags(
                &mut session_items,
                plan.session_id,
                new_id,
                plan.requested_tags,
            );
            self.request_sync();
            return Ok(new_id);
        }

        let old_sensitive = {
            let connection = self.state.conn.lock().unwrap();
            let tags_json: Option<String> = connection
                .query_row(
                    "SELECT tags FROM clipboard_history WHERE id = ?",
                    [entry_id],
                    |row| row.get(0),
                )
                .ok();
            let previous_tags: Vec<String> = tags_json
                .as_deref()
                .and_then(|value| serde_json::from_str(value).ok())
                .unwrap_or_default();
            has_sensitive_tag(&previous_tags)
        };

        let new_sensitive = has_sensitive_tag(&tags);
        self.state
            .tag_repo
            .update_entry_tags(entry_id, tags)
            .map_err(AppError::from)?;

        let transition = plan_sensitivity_transition(old_sensitive, new_sensitive);
        if transition == SensitivityTransition::Encrypt {
            // OCR text is intentionally stored as plaintext for fast local search.
            // Remove any existing index before the entry becomes sensitive.
            let connection = self
                .state
                .conn
                .lock()
                .map_err(|error| AppError::Database(error.to_string()))?;
            connection.execute(
                "DELETE FROM clipboard_image_analysis WHERE entry_id = ?1",
                [entry_id],
            )?;
        }

        let encryption_action = match transition {
            SensitivityTransition::Unchanged => None,
            SensitivityTransition::Encrypt => Some(EncryptionAction::Encrypt),
            SensitivityTransition::Decrypt => Some(EncryptionAction::Decrypt),
        };
        if let Some(action) = encryption_action {
            let queue = self.app_handle.state::<EncryptionQueueState>();
            queue.0.enqueue(EncryptionJob {
                id: entry_id,
                action,
            });
        }

        self.request_sync();
        Ok(entry_id)
    }

    fn notify_changed(&self) {
        let _ = self.app_handle.emit("clipboard-changed", ());
        self.request_sync();
    }

    fn request_sync(&self) {
        crate::services::cloud_sync::request_cloud_sync(self.app_handle.clone());
    }
}

impl MutationRecord for ClipboardEntry {
    fn id(&self) -> i64 {
        self.id
    }

    fn is_pinned(&self) -> bool {
        self.is_pinned
    }

    fn has_tags(&self) -> bool {
        !self.tags.is_empty()
    }
}

impl PinMutationRecord for ClipboardEntry {
    fn id(&self) -> i64 {
        self.id
    }

    fn set_id(&mut self, id: i64) {
        self.id = id;
    }

    fn set_pinned(&mut self, is_pinned: bool) {
        self.is_pinned = is_pinned;
    }
}

impl TagMutationRecord for ClipboardEntry {
    fn id(&self) -> i64 {
        self.id
    }

    fn set_id(&mut self, id: i64) {
        self.id = id;
    }

    fn set_tags(&mut self, tags: Vec<String>) {
        self.tags = tags;
    }
}
