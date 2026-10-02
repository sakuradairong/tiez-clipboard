use crate::app_state::SettingsState;
use crate::database::DbState;
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};

fn normalize_quick_paste_modifier(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "disabled" => "disabled",
        "ctrl" => "ctrl",
        "alt" => "alt",
        "shift" => "shift",
        "win" => "win",
        _ => "disabled",
    }
}

fn update_hotkey_setting(
    app_handle: &AppHandle,
    slot: &std::sync::Mutex<String>,
    key: &str,
    hotkey: String,
) -> AppResult<()> {
    let previous = slot
        .lock()
        .map_err(|e| AppError::Internal(e.to_string()))?
        .clone();
    *slot.lock().map_err(|e| AppError::Internal(e.to_string()))? = hotkey.clone();

    let db = app_handle.state::<DbState>();
    if let Err(error) = db.settings_repo.set(key, &hotkey) {
        *slot.lock().map_err(|e| AppError::Internal(e.to_string()))? = previous;
        return Err(AppError::from(error));
    }

    if let Err(registration_error) =
        crate::app::commands::hotkey_cmd::sync_registered_hotkeys(app_handle)
    {
        *slot.lock().map_err(|e| AppError::Internal(e.to_string()))? = previous.clone();
        let persistence_rollback = db.settings_repo.set(key, &previous);
        let registration_rollback =
            crate::app::commands::hotkey_cmd::sync_registered_hotkeys(app_handle);
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
pub fn set_sequential_mode(
    app_handle: AppHandle,
    state: State<'_, SettingsState>,
    enabled: bool,
) -> AppResult<()> {
    let previous = state.sequential_mode.load(Ordering::Relaxed);
    state.sequential_mode.store(enabled, Ordering::Relaxed);
    let db = app_handle.state::<DbState>();
    if let Err(error) = db
        .settings_repo
        .set("app.sequential_mode", &enabled.to_string())
    {
        state.sequential_mode.store(previous, Ordering::Relaxed);
        return Err(AppError::from(error));
    }
    if let Err(registration_error) =
        crate::app::commands::hotkey_cmd::sync_registered_hotkeys(&app_handle)
    {
        state.sequential_mode.store(previous, Ordering::Relaxed);
        let persistence_rollback = db
            .settings_repo
            .set("app.sequential_mode", &previous.to_string());
        let registration_rollback =
            crate::app::commands::hotkey_cmd::sync_registered_hotkeys(&app_handle);
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
pub fn set_sequential_hotkey(
    app_handle: AppHandle,
    state: State<'_, SettingsState>,
    hotkey: String,
) -> AppResult<()> {
    update_hotkey_setting(
        &app_handle,
        &state.sequential_paste_hotkey,
        "app.sequential_hotkey",
        hotkey,
    )
}

#[tauri::command]
pub fn set_rich_paste_hotkey(
    app_handle: AppHandle,
    state: State<'_, SettingsState>,
    hotkey: String,
) -> AppResult<()> {
    update_hotkey_setting(
        &app_handle,
        &state.rich_paste_hotkey,
        "app.rich_paste_hotkey",
        hotkey,
    )
}

#[tauri::command]
pub fn set_plain_paste_hotkey(
    app_handle: AppHandle,
    state: State<'_, SettingsState>,
    hotkey: String,
) -> AppResult<()> {
    update_hotkey_setting(
        &app_handle,
        &state.plain_paste_hotkey,
        "app.plain_paste_hotkey",
        hotkey,
    )
}

#[tauri::command]
pub fn set_search_hotkey(
    app_handle: AppHandle,
    state: State<'_, SettingsState>,
    hotkey: String,
) -> AppResult<()> {
    update_hotkey_setting(
        &app_handle,
        &state.search_hotkey,
        "app.search_hotkey",
        hotkey,
    )
}

#[tauri::command]
pub fn set_relay_send_hotkey(
    app_handle: AppHandle,
    state: State<'_, SettingsState>,
    hotkey: String,
) -> AppResult<()> {
    update_hotkey_setting(
        &app_handle,
        &state.relay_send_hotkey,
        "app.relay_send_hotkey",
        hotkey,
    )
}

#[tauri::command]
pub fn set_relay_fetch_hotkey(
    app_handle: AppHandle,
    state: State<'_, SettingsState>,
    hotkey: String,
) -> AppResult<()> {
    update_hotkey_setting(
        &app_handle,
        &state.relay_fetch_hotkey,
        "app.relay_fetch_hotkey",
        hotkey,
    )
}

#[tauri::command]
pub fn set_deduplication(
    app_handle: AppHandle,
    state: State<'_, crate::app_state::SettingsState>,
    enabled: bool,
) {
    state.deduplicate.store(enabled, Ordering::Relaxed);
    let db_state = app_handle.state::<DbState>();
    let _ = db_state
        .settings_repo
        .set("app.deduplicate", &enabled.to_string());
}

#[tauri::command]
pub fn save_setting(
    app_handle: AppHandle,
    db_state: State<'_, DbState>,
    settings_state: State<'_, crate::app_state::SettingsState>,
    key: String,
    mut value: String,
) -> AppResult<()> {
    if key == "clipboard_relay_shared_key" {
        return Err(AppError::Validation(
            "接力共享密钥只能通过系统安全密钥库配置".to_string(),
        ));
    }
    if key == "app.custom_background" {
        // The picker only authorizes the file for this process; keep the
        // asset protocol grant alive for the current session too (#5).
        crate::app::asset_scope::authorize_custom_background_file(&app_handle, &value)?;
    }
    match key.as_str() {
        "app.arrow_key_selection" => {
            settings_state
                .arrow_key_selection
                .store(value == "true", Ordering::Relaxed);
        }
        "app.sequential_mode" => {
            settings_state
                .sequential_mode
                .store(value == "true", Ordering::Relaxed);
        }
        "app.sound_enabled" => {
            settings_state
                .sound_enabled
                .store(value == "true", Ordering::Relaxed);
        }
        "app.sound_paste_enabled" => {
            settings_state
                .delete_after_paste
                .store(value != "false", Ordering::Relaxed);
        }
        "app.persistent" => {
            settings_state
                .persistent
                .store(value != "false", Ordering::Relaxed);
        }
        "app.capture_files" => {
            settings_state
                .capture_files
                .store(value != "false", Ordering::Relaxed);
        }
        "app.capture_rich_text" => {
            settings_state
                .capture_rich_text
                .store(value == "true", Ordering::Relaxed);
        }
        "app.silent_start" => {
            settings_state
                .silent_start
                .store(value != "false", Ordering::Relaxed);
        }
        "app.delete_after_paste" => {
            settings_state
                .delete_after_paste
                .store(value == "true", Ordering::Relaxed);
        }
        "app.privacy_protection" => {
            settings_state
                .privacy_protection
                .store(value == "true", Ordering::Relaxed);
        }
        "app.edge_docking" => {
            settings_state
                .edge_docking
                .store(value == "true", Ordering::Relaxed);
        }
        "app.follow_mouse" => {
            settings_state
                .follow_mouse
                .store(value != "false", Ordering::Relaxed);
        }
        "app.hide_tray_icon" => {
            settings_state
                .hide_tray_icon
                .store(value == "true", Ordering::Relaxed);
        }
        "app.quick_paste_modifier" => {
            value = normalize_quick_paste_modifier(&value).to_string();
            if let Ok(mut guard) = settings_state.quick_paste_modifier.lock() {
                *guard = value.clone();
            }
        }
        "app.quick_paste_when_edge_hidden" => {
            settings_state
                .quick_paste_when_edge_hidden
                .store(value == "true", Ordering::Relaxed);
        }
        _ => {}
    }

    db_state
        .settings_repo
        .set(&key, &value)
        .map_err(AppError::from)?;

    if is_appearance_setting(&key) {
        // Notify all webviews only after persistence (and background authorization).
        let _ = app_handle.emit("settings-changed", ());
    }

    Ok(())
}

fn is_appearance_setting(key: &str) -> bool {
    matches!(
        key,
        "app.theme"
            | "app.color_mode"
            | "app.compact_mode"
            | "app.clipboard_item_font_size"
            | "app.clipboard_tag_font_size"
            | "app.surface_opacity"
            | "app.custom_background"
            | "app.custom_background_opacity"
            | "app.theme_customization"
            | "app.appearance_presets"
    )
}

fn appearance_validation_error(key: &str) -> AppError {
    AppError::Validation(format!("外观设置无效: {key}"))
}

fn validate_appearance_theme(theme: &str) -> bool {
    matches!(
        theme,
        "retro" | "sticky-note" | "mica" | "acrylic" | "paper" | "sakura" | "minimal"
    ) || (theme.starts_with("store-")
        && theme.len() > 6
        && theme.len() <= 128
        && theme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
}

fn validate_appearance_number(key: &str, value: &str, min: u16, max: u16) -> AppResult<()> {
    match value.parse::<u16>() {
        Ok(number) if (min..=max).contains(&number) => Ok(()),
        _ => Err(appearance_validation_error(key)),
    }
}

fn validate_customization(value: &serde_json::Value) -> AppResult<()> {
    let object = value
        .as_object()
        .ok_or_else(|| appearance_validation_error("customization"))?;
    if let Some(accent) = object.get("accentColor").filter(|value| !value.is_null()) {
        let valid = accent
            .as_str()
            .map(|color| {
                color.len() == 7
                    && color.starts_with('#')
                    && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
            })
            .unwrap_or(false);
        if !valid {
            return Err(appearance_validation_error("accentColor"));
        }
    }
    if let Some(radius) = object.get("cornerRadius").filter(|value| !value.is_null()) {
        if !radius
            .as_f64()
            .map(|number| (0.0..=24.0).contains(&number))
            .unwrap_or(false)
        {
            return Err(appearance_validation_error("cornerRadius"));
        }
    }
    Ok(())
}

fn validate_preset_profile(value: &serde_json::Value) -> AppResult<()> {
    let profile = value
        .as_object()
        .ok_or_else(|| appearance_validation_error("profile"))?;
    let fields = [
        ("theme", "app.theme"),
        ("colorMode", "app.color_mode"),
        ("compactMode", "app.compact_mode"),
        ("clipboardItemFontSize", "app.clipboard_item_font_size"),
        ("clipboardTagFontSize", "app.clipboard_tag_font_size"),
        ("surfaceOpacity", "app.surface_opacity"),
        ("customBackground", "app.custom_background"),
        ("customBackgroundOpacity", "app.custom_background_opacity"),
    ];
    for (field, key) in fields {
        let value = profile
            .get(field)
            .ok_or_else(|| appearance_validation_error(field))?;
        let serialized = match value {
            serde_json::Value::String(text) => text.clone(),
            serde_json::Value::Bool(boolean) => boolean.to_string(),
            serde_json::Value::Number(number) => number.to_string(),
            _ => return Err(appearance_validation_error(field)),
        };
        validate_appearance_value(key, &serialized)?;
    }
    validate_customization(
        profile
            .get("customization")
            .ok_or_else(|| appearance_validation_error("customization"))?,
    )
}

fn validate_appearance_value(key: &str, value: &str) -> AppResult<()> {
    match key {
        "app.theme" if validate_appearance_theme(value) => Ok(()),
        "app.color_mode" if matches!(value, "system" | "light" | "dark") => Ok(()),
        "app.compact_mode" if matches!(value, "true" | "false") => Ok(()),
        "app.clipboard_item_font_size" => validate_appearance_number(key, value, 11, 18),
        "app.clipboard_tag_font_size" => validate_appearance_number(key, value, 8, 14),
        "app.surface_opacity" | "app.custom_background_opacity" => {
            validate_appearance_number(key, value, 0, 100)
        }
        "app.custom_background"
            if value.len() <= 4096 && !value.chars().any(|character| character <= '\u{1f}') =>
        {
            Ok(())
        }
        "app.theme_customization" if value.len() <= 4096 => {
            let parsed: serde_json::Value =
                serde_json::from_str(value).map_err(|_| appearance_validation_error(key))?;
            validate_customization(&parsed)
        }
        "app.appearance_presets" if value.len() <= 128 * 1024 => {
            let parsed: serde_json::Value =
                serde_json::from_str(value).map_err(|_| appearance_validation_error(key))?;
            let presets = parsed
                .as_array()
                .filter(|items| items.len() <= 20)
                .ok_or_else(|| appearance_validation_error(key))?;
            let mut ids = HashSet::new();
            for preset in presets {
                let id = preset
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| appearance_validation_error("id"))?;
                let name = preset
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| appearance_validation_error("name"))?;
                if id.is_empty()
                    || id.len() > 80
                    || !id.as_bytes()[0].is_ascii_alphanumeric()
                    || !id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                    || !ids.insert(id)
                {
                    return Err(appearance_validation_error("id"));
                }
                if name.trim().is_empty() || name.chars().count() > 60 {
                    return Err(appearance_validation_error("name"));
                }
                validate_preset_profile(
                    preset
                        .get("profile")
                        .ok_or_else(|| appearance_validation_error("profile"))?,
                )?;
            }
            Ok(())
        }
        _ => Err(appearance_validation_error(key)),
    }
}

fn validate_appearance_settings(settings: &HashMap<String, String>) -> AppResult<()> {
    if settings.is_empty() || settings.len() > 10 {
        return Err(appearance_validation_error("settings"));
    }
    for (key, value) in settings {
        if !is_appearance_setting(key) {
            return Err(appearance_validation_error(key));
        }
        validate_appearance_value(key, value)?;
    }
    Ok(())
}

#[tauri::command]
pub fn save_appearance_settings(
    app_handle: AppHandle,
    db_state: State<'_, DbState>,
    settings: HashMap<String, String>,
) -> AppResult<()> {
    validate_appearance_settings(&settings)?;
    if let Some(background) = settings.get("app.custom_background") {
        crate::app::asset_scope::authorize_custom_background_file(&app_handle, background)?;
    }
    db_state
        .settings_repo
        .set_many(&settings)
        .map_err(AppError::from)?;
    let _ = app_handle.emit("settings-changed", ());
    Ok(())
}

#[tauri::command]
pub fn set_ignore_blur(ignore: bool) {
    crate::IGNORE_BLUR.store(ignore, Ordering::Relaxed);
}

#[tauri::command]
pub fn set_window_pinned(app_handle: AppHandle, state: State<'_, DbState>, pinned: bool) {
    crate::WINDOW_PINNED.store(pinned, Ordering::Relaxed);
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.set_always_on_top(pinned);
        let _ = window.set_focusable(false);
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::{
                GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
            };
            if let Ok(hwnd) = window.hwnd() {
                unsafe {
                    let ex_style = GetWindowLongPtrW(HWND(hwnd.0), GWL_EXSTYLE);
                    let _ = SetWindowLongPtrW(
                        HWND(hwnd.0),
                        GWL_EXSTYLE,
                        ex_style | WS_EX_NOACTIVATE.0 as isize,
                    );
                }
            }
        }
    }
    let _ = state
        .settings_repo
        .set("app.window_pinned", &pinned.to_string());
}

#[tauri::command]
pub fn get_settings(
    app_handle: AppHandle,
    state: State<'_, DbState>,
) -> AppResult<std::collections::HashMap<String, String>> {
    let mut settings = state.settings_repo.get_all().map_err(AppError::from)?;
    settings.remove("clipboard_relay_shared_key");
    settings.remove("runtime.custom_background_error");

    if let Some(background) = settings.get("app.custom_background").cloned() {
        if let Err(error) =
            crate::app::asset_scope::authorize_custom_background_file(&app_handle, &background)
        {
            // Preserve the saved path in SQLite so a temporarily unavailable
            // drive can recover later, but keep this webview on the safe
            // default background and surface the reason to the user.
            settings.insert("app.custom_background".to_string(), String::new());
            settings.insert(
                "runtime.custom_background_error".to_string(),
                error.to_string(),
            );
        }
    }
    Ok(settings)
}

#[tauri::command]
pub fn set_file_server_auto_close(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state
        .file_server_auto_close
        .store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("file_transfer_auto_close", &enabled.to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_file_transfer_auto_open(db_state: State<'_, DbState>, enabled: bool) -> AppResult<()> {
    db_state
        .settings_repo
        .set("file_transfer_auto_open", &enabled.to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_arrow_key_selection(
    state: State<'_, crate::app_state::SettingsState>,
    enabled: bool,
) -> AppResult<()> {
    state.arrow_key_selection.store(enabled, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub fn set_persistence(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.persistent.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.persistent", &enabled.to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_capture_files(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.capture_files.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.capture_files", &enabled.to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_capture_rich_text(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.capture_rich_text.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.capture_rich_text", &enabled.to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_auto_copy_file(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.auto_copy_file.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set(
            "file_transfer_auto_copy",
            if enabled { "true" } else { "false" },
        )
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_silent_start(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.silent_start.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.silent_start", &enabled.to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_delete_after_paste(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.delete_after_paste.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.delete_after_paste", &enabled.to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_privacy_protection(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.privacy_protection.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.privacy_protection", &enabled.to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_privacy_protection_kinds(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    kinds: Vec<String>,
) -> AppResult<()> {
    let mut guard = state.privacy_protection_kinds.lock().unwrap();
    *guard = kinds.clone();
    let serialized = kinds.join(",");
    db_state
        .settings_repo
        .set("app.privacy_protection_kinds", &serialized)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_privacy_protection_custom_rules(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    rules: String,
) -> AppResult<()> {
    let list = rules
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    let mut guard = state.privacy_protection_custom_rules.lock().unwrap();
    *guard = list;
    db_state
        .settings_repo
        .set("app.privacy_protection_custom_rules", &rules)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_cleanup_rules(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    rules: String,
) -> AppResult<()> {
    let mut guard = state.cleanup_rules.lock().unwrap();
    *guard = rules.clone();
    db_state
        .settings_repo
        .set("app.cleanup_rules", &rules)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_app_cleanup_policies(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    policies: String,
) -> AppResult<()> {
    let mut guard = state.app_cleanup_policies.lock().unwrap();
    *guard = policies.clone();
    db_state
        .settings_repo
        .set("app.app_cleanup_policies", &policies)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_sound_enabled(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.sound_enabled.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.sound_enabled", &enabled.to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn get_mqtt_status() -> bool {
    crate::services::mqtt_sub::get_mqtt_status()
}

#[tauri::command]
pub fn get_mqtt_running() -> bool {
    crate::services::mqtt_sub::get_mqtt_running()
}

#[tauri::command]
pub fn restart_mqtt_client(app_handle: AppHandle) {
    crate::services::mqtt_sub::restart_mqtt_client(app_handle)
}

#[tauri::command]
pub fn get_cloud_sync_status() -> crate::services::cloud_sync::CloudSyncStatus {
    crate::services::cloud_sync::get_cloud_sync_status()
}

#[tauri::command]
pub fn restart_cloud_sync_client(app_handle: AppHandle) {
    crate::services::cloud_sync::restart_cloud_sync_client(app_handle);
}

#[tauri::command]
pub fn request_cloud_sync(app_handle: AppHandle) {
    crate::services::cloud_sync::request_cloud_sync(app_handle);
}

#[tauri::command]
pub async fn cloud_sync_now(
    app_handle: AppHandle,
) -> AppResult<crate::services::cloud_sync::CloudSyncStatus> {
    crate::services::cloud_sync::cloud_sync_now(app_handle).await
}

#[tauri::command]
pub fn reset_settings(
    app: AppHandle,
    state: State<'_, DbState>,
    settings_state: State<'_, crate::app_state::SettingsState>,
) -> AppResult<()> {
    use crate::database::seed_defaults;

    #[cfg(not(feature = "portable"))]
    crate::services::relay_key::clear()?;
    state.settings_repo.clear().map_err(AppError::from)?;
    {
        let conn = state.conn.lock().unwrap();
        seed_defaults(&conn).map_err(AppError::from)?;
    }

    let machine_id = crate::app::system::get_machine_id();
    let new_id = format!("{}-0000-0000-0000-000000000000", machine_id);
    state
        .settings_repo
        .set("app.anon_id", &new_id)
        .map_err(AppError::from)?;

    let main_hotkey = state
        .settings_repo
        .get("app.hotkey")
        .unwrap_or(Some("Alt+C".to_string()))
        .unwrap_or("Alt+C".to_string());
    let sequential_mode = state
        .settings_repo
        .get("app.sequential_mode")
        .unwrap_or(Some("false".to_string()))
        .map(|v| v == "true")
        .unwrap_or(false);
    let seq_hotkey = state
        .settings_repo
        .get("app.sequential_hotkey")
        .unwrap_or(Some("Alt+V".to_string()))
        .unwrap_or("Alt+V".to_string());
    let rich_hotkey = state
        .settings_repo
        .get("app.rich_paste_hotkey")
        .unwrap_or(Some("Ctrl+Shift+Z".to_string()))
        .unwrap_or("Ctrl+Shift+Z".to_string());
    let plain_hotkey = state
        .settings_repo
        .get("app.plain_paste_hotkey")
        .unwrap_or(Some(String::new()))
        .unwrap_or_default();
    let search_hotkey = state
        .settings_repo
        .get("app.search_hotkey")
        .unwrap_or(Some("Alt+F".to_string()))
        .unwrap_or("Alt+F".to_string());
    let relay_send_hotkey = state
        .settings_repo
        .get("app.relay_send_hotkey")
        .unwrap_or(Some(String::new()))
        .unwrap_or_default();
    let relay_fetch_hotkey = state
        .settings_repo
        .get("app.relay_fetch_hotkey")
        .unwrap_or(Some(String::new()))
        .unwrap_or_default();
    let quick_paste_modifier = state
        .settings_repo
        .get("app.quick_paste_modifier")
        .unwrap_or(Some("disabled".to_string()))
        .unwrap_or("disabled".to_string());

    settings_state
        .sequential_mode
        .store(sequential_mode, Ordering::Relaxed);
    {
        let mut guard = settings_state.main_hotkey.lock().unwrap();
        *guard = main_hotkey.clone();
    }
    {
        let mut guard = settings_state.sequential_paste_hotkey.lock().unwrap();
        *guard = seq_hotkey.clone();
    }
    {
        let mut guard = settings_state.rich_paste_hotkey.lock().unwrap();
        *guard = rich_hotkey.clone();
    }
    if let Ok(mut guard) = settings_state.plain_paste_hotkey.lock() {
        *guard = plain_hotkey.clone();
    }
    {
        let mut guard = settings_state.search_hotkey.lock().unwrap();
        *guard = search_hotkey.clone();
    }
    {
        let mut guard = settings_state.relay_send_hotkey.lock().unwrap();
        *guard = relay_send_hotkey;
    }
    {
        let mut guard = settings_state.relay_fetch_hotkey.lock().unwrap();
        *guard = relay_fetch_hotkey;
    }
    {
        let mut guard = settings_state.quick_paste_modifier.lock().unwrap();
        *guard = normalize_quick_paste_modifier(&quick_paste_modifier).to_string();
    }
    {
        let mut guard = crate::global_state::HOTKEY_STRING.lock().unwrap();
        *guard = main_hotkey.clone();
    }

    crate::app::commands::hotkey_cmd::sync_registered_hotkeys(&app)
}

#[tauri::command]
pub fn set_tray_visible(
    app_handle: AppHandle,
    state: State<'_, crate::app_state::SettingsState>,
    visible: bool,
) -> AppResult<()> {
    state.hide_tray_icon.store(!visible, Ordering::Relaxed);
    if let Some(tray) = app_handle.tray_by_id("main_tray") {
        let _ = tray.set_visible(visible);
    }
    let db_state = app_handle.state::<DbState>();
    db_state
        .settings_repo
        .set("app.hide_tray_icon", &(!visible).to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_edge_docking(
    app_handle: AppHandle,
    state: State<'_, crate::app_state::SettingsState>,
    enabled: bool,
) -> AppResult<()> {
    state.edge_docking.store(enabled, Ordering::Relaxed);
    let db_state = app_handle.state::<DbState>();
    db_state
        .settings_repo
        .set("app.edge_docking", &enabled.to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_follow_mouse(
    app_handle: AppHandle,
    state: State<'_, crate::app_state::SettingsState>,
    enabled: bool,
) -> AppResult<()> {
    state.follow_mouse.store(enabled, Ordering::Relaxed);
    let db_state = app_handle.state::<DbState>();
    db_state
        .settings_repo
        .set("app.follow_mouse", &enabled.to_string())
        .map_err(AppError::from)
}

#[cfg(test)]
mod appearance_tests {
    use super::*;

    #[test]
    fn appearance_batch_rejects_non_visual_keys_and_invalid_values() {
        for (key, value) in [
            ("mqtt_password", "secret"),
            ("app.theme", "unknown"),
            ("app.color_mode", "auto"),
            ("app.compact_mode", "1"),
            ("app.clipboard_item_font_size", "19"),
            ("app.clipboard_tag_font_size", "7"),
            ("app.surface_opacity", "101"),
            ("app.custom_background_opacity", "-1"),
            ("app.theme_customization", "not-json"),
            ("app.theme_customization", "[]"),
            (
                "app.theme_customization",
                r##"{"accentColor":"url(example)"}"##,
            ),
            ("app.theme_customization", r##"{"cornerRadius":25}"##),
            ("app.appearance_presets", "{}"),
        ] {
            let settings = HashMap::from([(key.to_string(), value.to_string())]);
            assert!(
                validate_appearance_settings(&settings).is_err(),
                "accepted {key}={value}"
            );
        }
        let oversized = HashMap::from([("app.theme_customization".to_string(), " ".repeat(4097))]);
        assert!(validate_appearance_settings(&oversized).is_err());
    }

    #[test]
    fn appearance_batch_accepts_a_complete_profile_and_named_preset() {
        let profile = serde_json::json!({
            "theme": "minimal",
            "colorMode": "system",
            "compactMode": false,
            "clipboardItemFontSize": 13,
            "clipboardTagFontSize": 10,
            "surfaceOpacity": 50,
            "customBackground": "C:\\old-background.png",
            "customBackgroundOpacity": 45,
            "customization": {"accentColor": "#2F6FED", "cornerRadius": 12}
        });
        let settings = HashMap::from([
            ("app.theme".to_string(), "minimal".to_string()),
            ("app.color_mode".to_string(), "system".to_string()),
            ("app.compact_mode".to_string(), "false".to_string()),
            ("app.clipboard_item_font_size".to_string(), "13".to_string()),
            ("app.clipboard_tag_font_size".to_string(), "10".to_string()),
            ("app.surface_opacity".to_string(), "50".to_string()),
            ("app.custom_background".to_string(), String::new()),
            (
                "app.custom_background_opacity".to_string(),
                "45".to_string(),
            ),
            (
                "app.theme_customization".to_string(),
                r##"{"accentColor":null,"cornerRadius":null}"##.to_string(),
            ),
            (
                "app.appearance_presets".to_string(),
                serde_json::json!([
                    {"id": "local-1", "name": "专注工作", "profile": profile}
                ])
                .to_string(),
            ),
        ]);

        assert!(validate_appearance_settings(&settings).is_ok());
        // Saving a preset may retain an old background path; applying it validates the file separately.
        assert!(validate_appearance_value(
            "app.appearance_presets",
            settings.get("app.appearance_presets").unwrap()
        )
        .is_ok());
    }

    #[test]
    fn appearance_presets_reject_duplicate_ids_and_invalid_profiles() {
        let profile = serde_json::json!({
            "theme": "mica", "colorMode": "dark", "compactMode": true,
            "clipboardItemFontSize": 13, "clipboardTagFontSize": 10,
            "surfaceOpacity": 50, "customBackground": "", "customBackgroundOpacity": 45,
            "customization": {"accentColor": null, "cornerRadius": null}
        });
        let duplicate = serde_json::json!([
            {"id": "same", "name": "A", "profile": profile},
            {"id": "same", "name": "B", "profile": profile}
        ]);
        assert!(
            validate_appearance_value("app.appearance_presets", &duplicate.to_string()).is_err()
        );
        let mut invalid = profile;
        invalid["clipboardItemFontSize"] = serde_json::json!(100);
        let invalid = serde_json::json!([{"id": "valid", "name": "工作", "profile": invalid}]);
        assert!(validate_appearance_value("app.appearance_presets", &invalid.to_string()).is_err());
    }
}
