use crate::app_state::SettingsState;
use crate::database::DbState;
use crate::error::{AppError, AppResult};
use crate::global_state::HOTKEY_STRING;
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

fn register_shortcut(app_handle: &AppHandle, hotkey: &str) -> AppResult<()> {
    if hotkey.is_empty()
        || hotkey.eq_ignore_ascii_case("MouseMiddle")
        || hotkey.eq_ignore_ascii_case("MButton")
    {
        return Ok(());
    }

    let normalized = hotkey.replace("Win", "Super");
    let shortcut = normalized
        .parse::<Shortcut>()
        .map_err(|_| AppError::Validation(format!("invalid hotkey: {hotkey}")))?;
    app_handle
        .global_shortcut()
        .register(shortcut)
        .map_err(|e| AppError::Internal(format!("failed to register {hotkey}: {e}")))
}

pub(crate) fn sync_registered_hotkeys(app_handle: &AppHandle) -> AppResult<()> {
    let mut errors = Vec::new();
    if let Err(error) = app_handle.global_shortcut().unregister_all() {
        errors.push(format!("failed to unregister hotkeys: {error}"));
    }

    let Some(settings) = app_handle.try_state::<SettingsState>() else {
        return if errors.is_empty() {
            Ok(())
        } else {
            Err(AppError::Internal(errors.join("; ")))
        };
    };

    let main_hotkey = settings
        .main_hotkey
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?
        .clone();
    let sequential_hotkey = settings
        .sequential_paste_hotkey
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?
        .clone();
    let rich_hotkey = settings
        .rich_paste_hotkey
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?
        .clone();
    let plain_hotkey = settings
        .plain_paste_hotkey
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?
        .clone();
    let search_hotkey = settings
        .search_hotkey
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?
        .clone();

    let mut configured = vec![main_hotkey];
    if settings.sequential_mode.load(Ordering::Relaxed) {
        configured.push(sequential_hotkey);
    }
    configured.extend([rich_hotkey, plain_hotkey, search_hotkey]);
    for hotkey in configured {
        if let Err(error) = register_shortcut(app_handle, &hotkey) {
            errors.push(error.to_string());
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::Internal(errors.join("; ")))
    }
}

#[tauri::command]
pub fn register_hotkey(app_handle: AppHandle, hotkey: String) -> AppResult<()> {
    let settings = app_handle
        .try_state::<SettingsState>()
        .ok_or_else(|| AppError::Internal("settings state unavailable".to_string()))?;
    let previous = settings
        .main_hotkey
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?
        .clone();
    {
        *settings
            .main_hotkey
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))? = hotkey.clone();
        *HOTKEY_STRING
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))? = hotkey.clone();
    }

    let db = app_handle.state::<DbState>();
    if let Err(error) = db.settings_repo.set("app.hotkey", &hotkey) {
        *settings
            .main_hotkey
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))? = previous.clone();
        *HOTKEY_STRING
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))? = previous;
        return Err(AppError::from(error));
    }

    if let Err(registration_error) = sync_registered_hotkeys(&app_handle) {
        *settings
            .main_hotkey
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))? = previous.clone();
        *HOTKEY_STRING
            .lock()
            .map_err(|e| AppError::Internal(e.to_string()))? = previous.clone();
        let persistence_rollback = db.settings_repo.set("app.hotkey", &previous);
        let registration_rollback = sync_registered_hotkeys(&app_handle);
        if let Err(error) = persistence_rollback {
            return Err(AppError::Internal(format!(
                "{registration_error}; failed to restore setting: {error}"
            )));
        }
        if let Err(error) = registration_rollback {
            return Err(AppError::Internal(format!(
                "{registration_error}; failed to restore hotkeys: {error}"
            )));
        }
        return Err(registration_error);
    }

    Ok(())
}

#[tauri::command]
pub fn test_hotkey_available(app_handle: AppHandle, hotkey: String) -> AppResult<bool> {
    if hotkey.is_empty()
        || hotkey.eq_ignore_ascii_case("MouseMiddle")
        || hotkey.eq_ignore_ascii_case("MButton")
    {
        return Ok(true);
    }

    let normalized = hotkey.replace("Win", "Super");
    let shortcut = normalized
        .parse::<Shortcut>()
        .map_err(|_| AppError::Validation("快捷键格式无效".to_string()))?;

    match app_handle.global_shortcut().register(shortcut.clone()) {
        Ok(_) => {
            let _ = app_handle.global_shortcut().unregister(shortcut);
            Ok(true)
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            let user_msg = if err_str.contains("AlreadyRegistered") {
                "该快捷键已被其他程序占用".to_string()
            } else {
                "快捷键不可用".to_string()
            };
            Err(AppError::Internal(user_msg))
        }
    }
}
