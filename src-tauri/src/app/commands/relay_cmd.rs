use crate::error::AppResult;
use crate::services::clipboard_relay::{RelayFetchResult, RelaySendResult};
use tauri::{AppHandle, Manager};

fn ensure_relay_runtime_allowed(app: &AppHandle) -> AppResult<()> {
    let data_dir = app
        .try_state::<crate::app_state::AppDataDir>()
        .and_then(|state| state.0.lock().ok().map(|value| value.clone()));
    crate::services::relay_key::ensure_runtime_allowed(data_dir.as_deref())
}

#[derive(serde::Serialize)]
pub struct RelaySharedKeyStatus {
    configured: bool,
}

#[tauri::command]
pub async fn relay_shared_key_status(app: AppHandle) -> AppResult<RelaySharedKeyStatus> {
    ensure_relay_runtime_allowed(&app)?;
    let configured =
        tauri::async_runtime::spawn_blocking(crate::services::relay_key::is_configured)
            .await
            .map_err(|error| crate::error::AppError::Internal(error.to_string()))??;
    Ok(RelaySharedKeyStatus { configured })
}

#[tauri::command]
pub async fn relay_set_shared_key(
    app: AppHandle,
    shared_key: String,
) -> AppResult<RelaySharedKeyStatus> {
    ensure_relay_runtime_allowed(&app)?;
    crate::services::relay_key::validate_format(&shared_key)?;
    tauri::async_runtime::spawn_blocking(move || crate::services::relay_key::store(&shared_key))
        .await
        .map_err(|error| crate::error::AppError::Internal(error.to_string()))??;
    Ok(RelaySharedKeyStatus { configured: true })
}

#[tauri::command]
pub async fn relay_generate_shared_key(app: AppHandle) -> AppResult<String> {
    ensure_relay_runtime_allowed(&app)?;
    tauri::async_runtime::spawn_blocking(crate::services::relay_key::generate)
        .await
        .map_err(|error| crate::error::AppError::Internal(error.to_string()))?
}

#[tauri::command]
pub async fn relay_clear_shared_key(app: AppHandle) -> AppResult<RelaySharedKeyStatus> {
    ensure_relay_runtime_allowed(&app)?;
    tauri::async_runtime::spawn_blocking(crate::services::relay_key::clear)
        .await
        .map_err(|error| crate::error::AppError::Internal(error.to_string()))??;
    Ok(RelaySharedKeyStatus { configured: false })
}

#[tauri::command]
pub async fn relay_send_clipboard(app_handle: AppHandle) -> AppResult<RelaySendResult> {
    crate::services::clipboard_relay::send_current_clipboard(&app_handle).await
}

#[tauri::command]
pub async fn relay_fetch_to_clipboard(app_handle: AppHandle) -> AppResult<RelayFetchResult> {
    crate::services::clipboard_relay::fetch_latest_to_clipboard(&app_handle).await
}
