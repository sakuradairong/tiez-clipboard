use crate::app::history_adapter::TauriHistoryAdapter;
use crate::app::mutation_adapter::TauriMutationAdapter;
use crate::app_state::{AppDataDir, SessionHistory};
use crate::database::DbState;
use crate::domain::models::ClipboardEntry;
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::clipboard_repo::ClipboardRepository;
use crate::infrastructure::repository::tag_repo::TagRepository;
use crate::services::clipboard::{
    build_entry_preview, derive_rich_text_content, truncate_html_for_preview,
};
use tauri::{AppHandle, State};

fn normalize_rich_text_item_content(item: &mut ClipboardEntry) {
    if item.content_type != "rich_text" {
        return;
    }

    let normalized = derive_rich_text_content(&item.content, item.html_content.as_deref());
    if !normalized.trim().is_empty() {
        item.content = normalized;
    }
}

#[tauri::command]
pub fn get_clipboard_history(
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    limit: i32,
    offset: i32,
    content_type: Option<String>,
) -> AppResult<Vec<ClipboardEntry>> {
    let adapter = TauriHistoryAdapter::new(&state.repo, session.inner());
    let mut history = adapter.list(limit, offset, content_type.as_deref())?;

    // Truncate content for UI performance after the shared history policy has
    // merged session-only entries and established stable ordering.
    for item in &mut history {
        normalize_rich_text_item_content(item);

        if (item.content_type == "text"
            || item.content_type == "code"
            || item.content_type == "url"
            || item.content_type == "rich_text")
            && item.content.chars().count() > 2000
        {
            item.content = format!(
                "{}... [Truncated for speed]",
                item.content.chars().take(2000).collect::<String>()
            );
        }

        if let Some(ref html) = item.html_content {
            if html.chars().count() > 5000 {
                item.html_content = truncate_html_for_preview(html);
            }
        }

        if item.content_type == "text"
            || item.content_type == "code"
            || item.content_type == "url"
            || item.content_type == "rich_text"
        {
            item.preview = build_entry_preview(
                &item.content_type,
                &item.content,
                item.html_content.as_deref(),
            );
        }
    }

    Ok(history)
}

#[tauri::command]
pub fn search_clipboard_history(
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    search_term: String,
    limit: i32,
    tag_only: Option<bool>,
) -> AppResult<Vec<ClipboardEntry>> {
    let is_tag_only = tag_only.unwrap_or(false);
    let adapter = TauriHistoryAdapter::new(&state.repo, session.inner());
    let mut history = adapter.search(&search_term, limit, is_tag_only)?;

    for item in &mut history {
        normalize_rich_text_item_content(item);

        if (item.content_type == "text"
            || item.content_type == "code"
            || item.content_type == "url"
            || item.content_type == "rich_text")
            && item.content.chars().count() > 2000
        {
            item.content = format!(
                "{}... [Truncated for speed]",
                item.content.chars().take(2000).collect::<String>()
            );
        }

        if let Some(ref html) = item.html_content {
            if html.chars().count() > 5000 {
                item.html_content = truncate_html_for_preview(html);
            }
        }

        if item.content_type == "text"
            || item.content_type == "code"
            || item.content_type == "url"
            || item.content_type == "rich_text"
        {
            item.preview = build_entry_preview(
                &item.content_type,
                &item.content,
                item.html_content.as_deref(),
            );
        }
    }

    Ok(history)
}

#[tauri::command]
pub fn delete_clipboard_entry(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    app_data: State<'_, AppDataDir>,
    id: i64,
) -> AppResult<()> {
    TauriMutationAdapter::new(state.inner(), &app_handle).delete(
        session.inner(),
        app_data.inner(),
        id,
    )
}

#[tauri::command]
pub fn clear_clipboard_history(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    app_data: State<'_, AppDataDir>,
) -> AppResult<()> {
    TauriMutationAdapter::new(state.inner(), &app_handle).clear(session.inner(), app_data.inner())
}

#[tauri::command]
pub fn get_tag_items(state: State<'_, DbState>, tag: String) -> AppResult<Vec<ClipboardEntry>> {
    let mut history = state
        .tag_repo
        .get_entries_by_tag(&tag)
        .map_err(AppError::from)?;

    for item in &mut history {
        normalize_rich_text_item_content(item);

        if (item.content_type == "text"
            || item.content_type == "code"
            || item.content_type == "url"
            || item.content_type == "rich_text")
            && item.content.chars().count() > 50000
        {
            item.content = format!(
                "{}... [Content Truncated]",
                item.content.chars().take(50000).collect::<String>()
            );
        }

        if item.content_type == "text"
            || item.content_type == "code"
            || item.content_type == "url"
            || item.content_type == "rich_text"
        {
            item.preview = build_entry_preview(
                &item.content_type,
                &item.content,
                item.html_content.as_deref(),
            );
        }
    }

    Ok(history)
}

#[tauri::command]
pub fn get_all_tags_info(
    state: State<'_, DbState>,
) -> AppResult<std::collections::HashMap<String, i32>> {
    state.tag_repo.get_all_with_counts().map_err(AppError::from)
}

#[tauri::command]
pub fn rename_tag_globally(
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    old_name: String,
    new_name: String,
) -> AppResult<()> {
    {
        let mut session_items = session.inner().0.lock().unwrap();
        for item in session_items.iter_mut() {
            for tag in item.tags.iter_mut() {
                if *tag == old_name {
                    *tag = new_name.clone();
                }
            }
            item.tags.sort();
            item.tags.dedup();
        }
    }

    state
        .tag_repo
        .rename(&old_name, &new_name)
        .map_err(AppError::from)
}

fn delete_tag_then_update_session<F>(
    session: &SessionHistory,
    tag_name: &str,
    delete_from_repository: F,
) -> AppResult<()>
where
    F: FnOnce() -> AppResult<()>,
{
    delete_from_repository()?;
    let mut session_items = session.0.lock().unwrap();
    session_items.retain(|item| !item.tags.iter().any(|tag| tag == tag_name));
    Ok(())
}

#[tauri::command]
pub fn delete_tag_from_all(
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    app_data: State<'_, AppDataDir>,
    tag_name: String,
) -> AppResult<()> {
    let data_dir = app_data.0.lock().unwrap();
    delete_tag_then_update_session(session.inner(), &tag_name, || {
        state
            .tag_repo
            .delete_globally(&tag_name, Some(&data_dir))
            .map_err(AppError::from)
    })
}

#[tauri::command]
pub fn create_new_tag(state: State<'_, DbState>, tag_name: String) -> AppResult<()> {
    state.tag_repo.create(&tag_name).map_err(AppError::from)
}

#[tauri::command]
pub fn get_clipboard_content(
    state: State<'_, DbState>,
    session: State<'_, SessionHistory>,
    id: i64,
) -> AppResult<String> {
    let adapter = TauriHistoryAdapter::new(&state.repo, session.inner());
    if let Some(resolved) = adapter.content(id).map_err(AppError::from)? {
        if resolved.content_type == "rich_text" {
            let normalized =
                derive_rich_text_content(&resolved.content, resolved.html_content.as_deref());
            if !normalized.trim().is_empty() {
                return Ok(normalized);
            }
        }
        return Ok(resolved.content);
    }

    Err(AppError::Validation("Entry not found".to_string()))
}

#[tauri::command]
pub fn update_pinned_order(
    app_handle: AppHandle,
    state: State<'_, DbState>,
    orders: Vec<(i64, i64)>,
) -> AppResult<()> {
    TauriMutationAdapter::new(state.inner(), &app_handle).update_pinned_order(orders)
}

#[tauri::command]
pub fn get_db_count(state: State<'_, DbState>) -> AppResult<i64> {
    state.repo.get_count().map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::delete_tag_then_update_session;
    use crate::app_state::SessionHistory;
    use crate::domain::models::ClipboardEntry;
    use crate::error::AppError;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[test]
    fn failed_global_tag_delete_keeps_session_history_consistent() {
        let entry = ClipboardEntry {
            id: -1,
            content_type: "text".to_string(),
            content: "session item".to_string(),
            html_content: None,
            source_app: "test".to_string(),
            timestamp: 1,
            preview: "session item".to_string(),
            is_pinned: false,
            tags: vec!["remove-me".to_string()],
            use_count: 0,
            is_external: false,
            pinned_order: 0,
            source_app_path: None,
            file_preview_exists: true,
        };
        let session = SessionHistory(Mutex::new(VecDeque::from([entry])));

        let result = delete_tag_then_update_session(&session, "remove-me", || {
            Err(AppError::Internal("repository failed".to_string()))
        });

        assert!(result.is_err());
        let items = session.0.lock().expect("lock retained session");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].tags, vec!["remove-me"]);
    }
}
