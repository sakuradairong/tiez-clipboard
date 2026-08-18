use crate::app_state::SettingsState;
use crate::database::DbState;
use crate::error::{AppError, AppResult};
use crate::global_state::HOTKEY_STRING;
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

pub(crate) fn normalize_hotkey_aliases(hotkey: &str) -> String {
    hotkey
        .split('+')
        .map(|part| match part.trim().to_ascii_lowercase().as_str() {
            "win" | "windows" | "command" | "cmd" | "meta" => "Super".to_string(),
            "option" => "Alt".to_string(),
            "control" => "Ctrl".to_string(),
            _ => part.trim().to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn is_ignorable_hotkey(hotkey: &str) -> bool {
    hotkey.is_empty()
        || hotkey.eq_ignore_ascii_case("MouseMiddle")
        || hotkey.eq_ignore_ascii_case("MButton")
}

fn parse_shortcut(hotkey: &str) -> AppResult<Shortcut> {
    let normalized = normalize_hotkey_aliases(hotkey);
    normalized
        .parse::<Shortcut>()
        .map_err(|_| AppError::Validation(format!("invalid hotkey: {hotkey}")))
}

pub(crate) fn unique_configured_hotkeys(hotkeys: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();

    for hotkey in hotkeys {
        if is_ignorable_hotkey(&hotkey) {
            continue;
        }
        let normalized = normalize_hotkey_aliases(&hotkey);
        if seen.insert(normalized) {
            unique.push(hotkey);
        }
    }

    unique
}

fn register_shortcut(app_handle: &AppHandle, hotkey: &str) -> AppResult<()> {
    if is_ignorable_hotkey(hotkey) {
        return Ok(());
    }

    let shortcut = parse_shortcut(hotkey)?;
    let global_shortcut = app_handle.global_shortcut();
    // Re-sync can run while a shortcut is still registered; clear it first.
    let _ = global_shortcut.unregister(shortcut.clone());
    global_shortcut
        .register(shortcut)
        .map_err(|e| AppError::Internal(format!("failed to register {hotkey}: {e}")))
}

pub(crate) fn sync_registered_hotkeys(app_handle: &AppHandle) -> AppResult<()> {
    let _ = app_handle.global_shortcut().unregister_all();

    let Some(settings) = app_handle.try_state::<SettingsState>() else {
        return Ok(());
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
    let relay_send_hotkey = settings
        .relay_send_hotkey
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?
        .clone();
    let relay_fetch_hotkey = settings
        .relay_fetch_hotkey
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?
        .clone();

    let mut configured = vec![main_hotkey];
    if settings.sequential_mode.load(Ordering::Relaxed) {
        configured.push(sequential_hotkey);
    }
    configured.extend([
        rich_hotkey,
        plain_hotkey,
        search_hotkey,
        relay_send_hotkey,
        relay_fetch_hotkey,
    ]);

    let mut errors = Vec::new();
    for hotkey in unique_configured_hotkeys(configured) {
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

fn internal_error_message(error: &AppError) -> String {
    match error {
        AppError::Internal(message) => message.clone(),
        other => other.to_string(),
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
                "{}; failed to restore setting: {error}",
                internal_error_message(&registration_error)
            )));
        }
        if let Err(error) = registration_rollback {
            return Err(AppError::Internal(format!(
                "{}; failed to restore hotkeys: {}",
                internal_error_message(&registration_error),
                internal_error_message(&error)
            )));
        }
        return Err(registration_error);
    }

    Ok(())
}

#[tauri::command]
pub fn test_hotkey_available(app_handle: AppHandle, hotkey: String) -> AppResult<bool> {
    if is_ignorable_hotkey(&hotkey) {
        return Ok(true);
    }

    let shortcut = parse_shortcut(&hotkey)?;
    let global_shortcut = app_handle.global_shortcut();
    let _ = global_shortcut.unregister(shortcut.clone());

    match global_shortcut.register(shortcut.clone()) {
        Ok(_) => {
            let _ = global_shortcut.unregister(shortcut);
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

#[cfg(test)]
mod tests {
    use super::unique_configured_hotkeys;

    #[test]
    fn unique_configured_hotkeys_dedupes_normalized_aliases() {
        assert_eq!(
            unique_configured_hotkeys(vec![
                "Win+V".to_string(),
                "Super+V".to_string(),
                "Alt+C".to_string(),
            ]),
            vec!["Win+V".to_string(), "Alt+C".to_string()]
        );
    }

    #[test]
    fn unique_configured_hotkeys_skips_empty_and_mouse_bindings() {
        assert_eq!(
            unique_configured_hotkeys(vec![
                "".to_string(),
                "MouseMiddle".to_string(),
                "Alt+F".to_string(),
            ]),
            vec!["Alt+F".to_string()]
        );
    }
}
