//! Tauri host adapter for the shared clipboard content helpers.
use tauri::Manager;

pub(super) fn current_clipboard_image_roots() -> Vec<std::path::PathBuf> {
    let mut roots = vec![std::env::temp_dir()];
    if let Some(app) = crate::global_state::GLOBAL_APP_HANDLE.get() {
        if let Some(state) = app.try_state::<crate::app_state::AppDataDir>() {
            if let Ok(data_dir) = state.0.lock() {
                roots.extend(crate::services::local_image_resource::managed_image_roots(
                    &data_dir,
                ));
            }
        }
    }
    roots
}
