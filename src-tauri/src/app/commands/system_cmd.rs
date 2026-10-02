use crate::app_state::{AppDataDir, AppReady};
use crate::database::ENCRYPT_PREFIX;
use crate::error::{AppError, AppResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json;
use tauri::{AppHandle, Manager, State};

#[cfg(target_os = "windows")]
const CLIPBOARD_BACKUP_KEY: &str = "Software\\tiez\\ClipboardSettingsBackup";
#[cfg(target_os = "windows")]
static CLIPBOARD_REGISTRY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(target_os = "windows")]
fn owned_clipboard_value_matches(
    current: Option<&winreg::RegValue>,
    applied_present: Option<u32>,
    applied: Option<&winreg::RegValue>,
) -> bool {
    match applied_present {
        Some(0) => current.is_none(),
        Some(1) => applied.is_some() && current == applied,
        _ => false,
    }
}

#[cfg(target_os = "windows")]
fn has_original_clipboard_snapshot(
    present: Option<u32>,
    value: Option<&winreg::RegValue>,
) -> bool {
    match present {
        Some(0) => true,
        Some(1) => value.is_some_and(|value| {
            matches!(value.vtype, winreg::enums::REG_DWORD | winreg::enums::REG_SZ)
        }),
        _ => false,
    }
}

#[cfg(target_os = "windows")]
fn save_clipboard_registry_snapshot(
    backup: &winreg::RegKey,
    prefix: &str,
    value: Option<&winreg::RegValue>,
) -> AppResult<()> {
    if let Some(value) = value {
        backup.set_raw_value(format!("{}Value", prefix), value)?;
    } else {
        match backup.delete_value(format!("{}Value", prefix)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    backup.set_value(format!("{}Present", prefix), &u32::from(value.is_some()))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn set_owned_clipboard_registry_value(
    hkcu: &winreg::RegKey,
    path: &str,
    name: &str,
    value: Option<winreg::RegValue>,
) -> AppResult<bool> {
    use winreg::enums::{REG_DWORD, REG_SZ};

    let _guard = CLIPBOARD_REGISTRY_LOCK.lock().unwrap();
    let (key, _) = hkcu.create_subkey(path)?;
    let current = match key.get_raw_value(name) {
        Ok(value) => Some(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if current == value {
        return Ok(false);
    }
    if current.as_ref().is_some_and(|value| !matches!(value.vtype, REG_DWORD | REG_SZ)) {
        return Err(AppError::Validation(format!("不支持修改注册表设置 {} 的当前类型", name)));
    }

    let (backup, _) = hkcu.create_subkey(format!("{}\\{}", CLIPBOARD_BACKUP_KEY, name))?;
    let applied = backup.get_raw_value("AppliedValue").ok();
    let original = backup.get_raw_value("OriginalValue").ok();
    let has_original = has_original_clipboard_snapshot(
        backup.get_value("OriginalPresent").ok(), original.as_ref(),
    );
    let still_owned = owned_clipboard_value_matches(
        current.as_ref(),
        backup.get_value("AppliedPresent").ok(),
        applied.as_ref(),
    );
    // A subsequent user change starts a new ownership interval.
    if !has_original || !still_owned {
        save_clipboard_registry_snapshot(&backup, "Original", current.as_ref())?;
    }
    // Persist the intended value before changing the system setting.
    save_clipboard_registry_snapshot(&backup, "Applied", value.as_ref())?;
    let result = match value.as_ref() {
        Some(value) => key.set_raw_value(name, value),
        None => key.delete_value(name),
    };
    if let Err(error) = result {
        let _ = save_clipboard_registry_snapshot(&backup, "Applied", current.as_ref());
        return Err(error.into());
    }
    Ok(true)
}

#[tauri::command]
pub fn is_app_ready(state: State<'_, AppReady>) -> AppResult<bool> {
    Ok(state.0.load(std::sync::atomic::Ordering::SeqCst))
}

#[tauri::command]
pub fn get_data_path(state: State<'_, AppDataDir>) -> AppResult<String> {
    let path = state.0.lock().unwrap();
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn open_folder(path: String) -> AppResult<()> {
    use std::process::Command;
    Command::new("explorer")
        .arg(path)
        .spawn()
        .map_err(|e| AppError::Internal(format!("Failed to open folder: {}", e)))?;
    Ok(())
}

#[tauri::command]
pub fn open_data_folder(state: State<'_, AppDataDir>) -> AppResult<()> {
    let path = state.0.lock().unwrap();
    let path_str = path.to_string_lossy().to_string();

    use std::process::Command;
    Command::new("explorer")
        .arg(path_str)
        .spawn()
        .map_err(|e| AppError::Internal(format!("Failed to open data folder: {}", e)))?;
    Ok(())
}

#[tauri::command]
pub fn open_file_with_default_app(file_path: String) -> AppResult<()> {
    use std::process::Command;
    Command::new("explorer")
        .arg(&file_path)
        .spawn()
        .map_err(|e| AppError::Internal(format!("Failed to open file: {}", e)))?;
    Ok(())
}

#[tauri::command]
pub fn open_file_location(file_path: String) -> AppResult<()> {
    use std::process::Command;
    Command::new("explorer")
        .arg("/select,")
        .arg(&file_path)
        .spawn()
        .map_err(|e| AppError::Internal(format!("Failed to open file location: {}", e)))?;
    Ok(())
}

#[tauri::command]
pub fn toggle_autostart(enabled: bool) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu
            .open_subkey_with_flags(
                "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                KEY_WRITE | KEY_READ,
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let app_path = std::env::current_exe()
            .map_err(|e| AppError::Internal(e.to_string()))?
            .to_string_lossy()
            .to_string();
        let cmd = format!("\"{}\" --minimized", app_path);

        if enabled {
            key.set_value("TieZ", &cmd)
                .map_err(|e| AppError::Internal(e.to_string()))?;
        } else {
            let _ = key.delete_value("TieZ");
            let _ = key.delete_value("tie-z");
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = enabled;
        Ok(())
    }
}

#[tauri::command]
pub fn is_autostart_enabled() -> AppResult<bool> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(key.get_value::<String, _>("TieZ").is_ok()
            || key.get_value::<String, _>("tie-z").is_ok())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(false)
    }
}

#[tauri::command]
pub fn set_windows_clipboard_history(enabled: bool) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        use winreg::types::ToRegValue;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let mut needs_restart = false;

        if let Ok((_key, _)) = hkcu.create_subkey("Software\\Microsoft\\Clipboard") {
            let value: u32 = if enabled { 1 } else { 0 };
            for name in ["EnableClipboardHistory", "EnableCloudClipboard"] {
                set_owned_clipboard_registry_value(
                    &hkcu, "Software\\Microsoft\\Clipboard", name, Some(value.to_reg_value()),
                )?;
            }
        }

        if let Ok((adv_key, _)) =
            hkcu.create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced")
        {
            let current_disabled: String = adv_key.get_value("DisabledHotkeys").unwrap_or_default();
            if current_disabled.to_uppercase().contains('V') {
                let new_val = current_disabled.replace(['V', 'v'], "");
                needs_restart |= set_owned_clipboard_registry_value(
                    &hkcu,
                    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced",
                    "DisabledHotkeys",
                    (!new_val.is_empty()).then(|| new_val.to_reg_value()),
                )?;
            }
        }

        if let Ok((policy_key, _)) =
            hkcu.create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\Explorer")
        {
            if policy_key
                .get_value::<u32, _>("DisallowClipboardHistory")
                .unwrap_or(0)
                != 0
            {
                needs_restart |= set_owned_clipboard_registry_value(
                    &hkcu, "Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\Explorer",
                    "DisallowClipboardHistory", None,
                )?;
            }
        }

        // Policy-based clipboard lock can also exist under Software\Policies\Microsoft\Windows\System.
        // Clear blocking values when restoring system Win+V behavior.
        if enabled {
            if let Ok((sys_policy, _)) =
                hkcu.create_subkey("Software\\Policies\\Microsoft\\Windows\\System")
            {
                if sys_policy
                    .get_value::<u32, _>("AllowClipboardHistory")
                    .unwrap_or(1)
                    == 0
                {
                    needs_restart |= set_owned_clipboard_registry_value(
                        &hkcu, "Software\\Policies\\Microsoft\\Windows\\System",
                        "AllowClipboardHistory", None,
                    )?;
                }
                if sys_policy
                    .get_value::<u32, _>("AllowCrossDeviceClipboard")
                    .unwrap_or(1)
                    == 0
                {
                    needs_restart |= set_owned_clipboard_registry_value(
                        &hkcu, "Software\\Policies\\Microsoft\\Windows\\System",
                        "AllowCrossDeviceClipboard", None,
                    )?;
                }
            }
        }

        if needs_restart {
            restart_explorer().ok();
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = enabled;
        Ok(())
    }
}

#[tauri::command]
pub fn get_windows_clipboard_history() -> AppResult<bool> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);

        let v_disabled = match hkcu
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced")
        {
            Ok(key) => key
                .get_value::<String, _>("DisabledHotkeys")
                .unwrap_or_default()
                .to_uppercase()
                .contains('V'),
            Err(_) => false,
        };
        let history_enabled = match hkcu.open_subkey("Software\\Microsoft\\Clipboard") {
            Ok(key) => {
                key.get_value::<u32, _>("EnableClipboardHistory")
                    .unwrap_or(1)
                    != 0
            }
            Err(_) => true,
        };
        Ok(history_enabled && !v_disabled)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(false)
    }
}

#[tauri::command]
pub fn set_win_clipboard_disabled(_disabled: bool) -> AppResult<()> {
    set_windows_clipboard_history(!_disabled)
}

#[tauri::command]
pub fn trigger_registry_win_v_optimization(enable: bool) -> AppResult<bool> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        use winreg::types::ToRegValue;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let mut changed = false;

        if let Ok((adv_key, _)) =
            hkcu.create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced")
        {
            let current: String = adv_key.get_value("DisabledHotkeys").unwrap_or_default();
            if enable && !current.to_uppercase().contains('V') {
                changed |= set_owned_clipboard_registry_value(
                    &hkcu,
                    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced",
                    "DisabledHotkeys", Some(format!("{}V", current).to_reg_value()),
                )?;
            } else if !enable && current.to_uppercase().contains('V') {
                let clean = current.replace(['V', 'v'], "");
                changed |= set_owned_clipboard_registry_value(
                    &hkcu,
                    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced",
                    "DisabledHotkeys", (!clean.is_empty()).then(|| clean.to_reg_value()),
                )?;
            }
        }

        if let Ok((_cb_key, _)) = hkcu.create_subkey("Software\\Microsoft\\Clipboard") {
            let val: u32 = if enable { 0 } else { 1 };
            for name in ["EnableClipboardHistory", "EnableCloudClipboard"] {
                changed |= set_owned_clipboard_registry_value(
                    &hkcu, "Software\\Microsoft\\Clipboard", name, Some(val.to_reg_value()),
                )?;
            }
        }

        // When disabling Win+V takeover, also clear policy-level lock that can keep Win+V unavailable
        // until a full reboot on some systems.
        if !enable {
            if let Ok((policy_key, _)) = hkcu
                .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\Explorer")
            {
                if policy_key
                    .get_value::<u32, _>("DisallowClipboardHistory")
                    .unwrap_or(0)
                    != 0
                {
                    changed |= set_owned_clipboard_registry_value(
                        &hkcu, "Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\Explorer",
                        "DisallowClipboardHistory", None,
                    )?;
                }
            }

            if let Ok((sys_policy, _)) =
                hkcu.create_subkey("Software\\Policies\\Microsoft\\Windows\\System")
            {
                if sys_policy
                    .get_value::<u32, _>("AllowClipboardHistory")
                    .unwrap_or(1)
                    == 0
                {
                    changed |= set_owned_clipboard_registry_value(
                        &hkcu, "Software\\Policies\\Microsoft\\Windows\\System",
                        "AllowClipboardHistory", None,
                    )?;
                }
                if sys_policy
                    .get_value::<u32, _>("AllowCrossDeviceClipboard")
                    .unwrap_or(1)
                    == 0
                {
                    changed |= set_owned_clipboard_registry_value(
                        &hkcu, "Software\\Policies\\Microsoft\\Windows\\System",
                        "AllowCrossDeviceClipboard", None,
                    )?;
                }
            }
        }
        Ok(changed)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = enable;
        Ok(false)
    }
}

#[tauri::command]
pub fn is_registry_win_v_optimized() -> AppResult<bool> {
    Ok(get_registry_win_v_optimized_status())
}

pub fn get_registry_win_v_optimized_status() -> bool {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) =
            hkcu.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced")
        {
            return key
                .get_value::<String, _>("DisabledHotkeys")
                .unwrap_or_default()
                .to_uppercase()
                .contains('V');
        }
        false
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

#[tauri::command]
pub fn restart_explorer() -> AppResult<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        Command::new("cmd")
            .args(["/C", "taskkill /F /IM explorer.exe & start explorer.exe"])
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|error| AppError::Internal(format!("failed to restart Explorer: {error}")))?;
    }
    Ok(())
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn relaunch(app: AppHandle) {
    use std::process::Command;
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).spawn();
    }
    app.exit(0);
}

#[tauri::command]
pub fn restart_as_admin(app_handle: AppHandle) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    {
        use std::env;
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        // Get current executable path
        let exe_path = env::current_exe().map_err(AppError::from)?;

        // Convert to wide string
        let exe_wide: Vec<u16> = OsStr::new(&exe_path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // "runas" verb for elevation
        let runas: Vec<u16> = OsStr::new("runas")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let result = ShellExecuteW(
                None,
                PCWSTR::from_raw(runas.as_ptr()),
                PCWSTR::from_raw(exe_wide.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );

            // ShellExecuteW returns > 32 on success
            if result.0 as usize <= 32 {
                return Err(AppError::Internal(
                    "Failed to restart as administrator. User may have cancelled UAC prompt."
                        .to_string(),
                ));
            }
        }

        // Close current instance
        app_handle.exit(0);

        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app_handle;
        Ok(())
    }
}

#[tauri::command]
pub fn check_is_admin() -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::ffi::c_void;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Security::{
            GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        unsafe {
            let mut token_handle = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle).is_ok() {
                let mut elevation = TOKEN_ELEVATION::default();
                let mut return_length = 0;
                let success = GetTokenInformation(
                    token_handle,
                    TokenElevation,
                    Some(&mut elevation as *mut _ as *mut c_void),
                    std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                    &mut return_length,
                );

                let _ = windows::Win32::Foundation::CloseHandle(token_handle);

                if success.is_ok() {
                    return elevation.TokenIsElevated != 0;
                }
            }
        }
        false
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

#[tauri::command]
pub fn set_data_path(app_handle: AppHandle, new_path: String) -> AppResult<()> {
    let clean_path = new_path.trim().to_string();
    let new_data_path = std::path::Path::new(&clean_path);
    if !new_data_path.exists() {
        return Err(AppError::Validation("Directory does not exist".to_string()));
    }

    let old_path_buf = app_handle.state::<AppDataDir>().0.lock().unwrap().clone();

    // 1. Migrate data folders if they exist in the OLD path
    {
        for folder in ["attachments", "emoji_favorites"] {
            let old_folder = old_path_buf.join(folder);
            let new_folder = new_data_path.join(folder);

            if old_folder.exists() && old_folder.is_dir() {
                if let Err(_) = std::fs::rename(&old_folder, &new_folder) {
                    if let Err(copy_err) = copy_dir_recursive(&old_folder, &new_folder) {
                        return Err(AppError::Internal(format!(
                            "Failed to copy {}: {}",
                            folder, copy_err
                        )));
                    } else {
                        let _ = std::fs::remove_dir_all(&old_folder);
                    }
                }
            }
        }

        // 1.2 Migrate database files (main + WAL/SHM)
        let db_files = ["clipboard.db", "clipboard.db-wal", "clipboard.db-shm"];
        for name in db_files {
            let old_db = old_path_buf.join(name);
            if !old_db.exists() {
                continue;
            }
            let new_db = new_data_path.join(name);
            if new_db.exists() {
                // Avoid overwriting any existing DB in new path
                let backup = new_data_path.join(format!("{}.backup", name));
                if backup.exists() {
                    let _ = std::fs::remove_file(&backup);
                }
                let _ = std::fs::rename(&new_db, &backup);
            }
            if let Err(_) = std::fs::rename(&old_db, &new_db) {
                if let Err(copy_err) = std::fs::copy(&old_db, &new_db) {
                    return Err(AppError::Internal(format!(
                        "Failed to copy {}: {}",
                        name, copy_err
                    )));
                } else {
                    let _ = std::fs::remove_file(&old_db);
                }
            }
        }
    }

    // 1.3 Rewrite internal attachment paths inside DB (if DB exists in new path)
    let new_db_path = new_data_path.join("clipboard.db");
    if new_db_path.exists() {
        rewrite_attachment_paths_in_db(&new_db_path, &old_path_buf, new_data_path)?;
        rewrite_emoji_favorites_in_db(&new_db_path, &old_path_buf, new_data_path)?;
        rewrite_custom_background_in_db(&new_db_path, &old_path_buf, new_data_path)?;
    }

    // 2. Save new path to a persistent config file
    let config_dir = app_handle.path().app_data_dir().map_err(AppError::from)?;
    if !config_dir.exists() {
        std::fs::create_dir_all(&config_dir).map_err(AppError::from)?;
    }

    let redirect_file = config_dir.join("datapath.txt");
    std::fs::write(&redirect_file, &clean_path).map_err(AppError::from)?;

    // 3. Keep the runtime asset protocol scope in sync: grant the new data
    // directory's image folders and retire the old grant until the pending
    // relaunch rebuilds the scope from scratch.
    crate::app::asset_scope::authorize_data_asset_scope(&app_handle, new_data_path);
    crate::app::asset_scope::retire_data_asset_scope(&app_handle, &old_path_buf);

    Ok(())
}

pub(crate) fn rewrite_attachment_paths_in_db(
    db_path: &std::path::Path,
    old_base: &std::path::Path,
    new_base: &std::path::Path,
) -> AppResult<()> {
    let old_attach = old_base.join("attachments");
    let new_attach = new_base.join("attachments");
    let old_prefix = old_attach.to_string_lossy().to_string();
    let new_prefix = new_attach.to_string_lossy().to_string();
    if old_prefix == new_prefix {
        return Ok(());
    }

    let old_prefix_slash = old_prefix.replace('\\', "/");
    let new_prefix_slash = new_prefix.replace('\\', "/");

    let conn = Connection::open(db_path).map_err(AppError::from)?;

    let mut stmt = conn
        .prepare(
            "SELECT id, content, html_content FROM clipboard_history
                  WHERE content_type IN ('image', 'file', 'video')
                     OR is_external = 1
                     OR html_content IS NOT NULL",
        )
        .map_err(AppError::from)?;

    let rows = stmt
        .query_map([], |row| {
            let id: i64 = row.get(0)?;
            let content: String = row.get(1)?;
            let html_content: Option<String> = row.get(2)?;
            Ok((id, content, html_content))
        })
        .map_err(AppError::from)?;

    for row in rows {
        let (id, content_raw, html_raw) = row.map_err(AppError::from)?;
        let mut content_new: Option<String> = None;
        let mut html_new: Option<String> = None;

        let content_was_encrypted = content_raw.starts_with(ENCRYPT_PREFIX);
        let content_plain = if content_was_encrypted {
            crate::infrastructure::encryption::decrypt_value(&content_raw)
                .unwrap_or_else(|| content_raw.clone())
        } else {
            content_raw.clone()
        };
        if let Some(updated_plain) = rewrite_content_path(
            &content_plain,
            &old_prefix,
            &new_prefix,
            &old_prefix_slash,
            &new_prefix_slash,
        ) {
            content_new = Some(if content_was_encrypted {
                crate::infrastructure::encryption::encrypt_value(&updated_plain)
                    .unwrap_or(updated_plain)
            } else {
                updated_plain
            });
        }

        if let Some(html_raw_value) = html_raw.as_ref() {
            let html_was_encrypted = html_raw_value.starts_with(ENCRYPT_PREFIX);
            let html_plain = if html_was_encrypted {
                crate::infrastructure::encryption::decrypt_value(html_raw_value)
                    .unwrap_or_else(|| html_raw_value.clone())
            } else {
                html_raw_value.clone()
            };
            if let Some(updated_plain) = rewrite_html_paths(
                &html_plain,
                &old_prefix,
                &new_prefix,
                &old_prefix_slash,
                &new_prefix_slash,
            ) {
                html_new = Some(if html_was_encrypted {
                    crate::infrastructure::encryption::encrypt_value(&updated_plain)
                        .unwrap_or(updated_plain)
                } else {
                    updated_plain
                });
            }
        }

        if content_new.is_some() || html_new.is_some() {
            let content_final = content_new.as_ref().unwrap_or(&content_raw);
            let html_final = match html_new.as_ref() {
                Some(v) => Some(v.as_str()),
                None => html_raw.as_deref(),
            };
            conn.execute(
                "UPDATE clipboard_history SET content = ?1, html_content = ?2 WHERE id = ?3",
                params![content_final, html_final, id],
            )
            .map_err(AppError::from)?;
        }
    }

    Ok(())
}

pub(crate) fn rewrite_emoji_favorites_in_db(
    db_path: &std::path::Path,
    old_base: &std::path::Path,
    new_base: &std::path::Path,
) -> AppResult<()> {
    let old_dir = old_base.join("emoji_favorites");
    let new_dir = new_base.join("emoji_favorites");
    let old_prefix = old_dir.to_string_lossy().to_string();
    let new_prefix = new_dir.to_string_lossy().to_string();
    if old_prefix == new_prefix {
        return Ok(());
    }

    let old_prefix_slash = old_prefix.replace('\\', "/");
    let new_prefix_slash = new_prefix.replace('\\', "/");

    let conn = Connection::open(db_path).map_err(AppError::from)?;
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'app.emoji_favorites'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(AppError::from)?;

    let Some(raw) = value else {
        return Ok(());
    };
    let parsed: Vec<String> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };

    let mut changed = false;
    let mut updated: Vec<String> = Vec::with_capacity(parsed.len());
    for path in parsed {
        let mut next = path.clone();
        if next.starts_with(&old_prefix) {
            next = format!("{}{}", new_prefix, &next[old_prefix.len()..]);
        } else if next.starts_with(&old_prefix_slash) {
            next = format!("{}{}", new_prefix_slash, &next[old_prefix_slash.len()..]);
        }
        if next != path {
            changed = true;
        }
        updated.push(next);
    }

    if changed {
        let serialized = serde_json::to_string(&updated).unwrap_or(raw);
        conn.execute(
            "UPDATE settings SET value = ?1 WHERE key = 'app.emoji_favorites'",
            params![serialized],
        )
        .map_err(AppError::from)?;
    }

    Ok(())
}

pub(crate) fn rewrite_custom_background_in_db(
    db_path: &std::path::Path,
    old_base: &std::path::Path,
    new_base: &std::path::Path,
) -> AppResult<()> {
    let conn = Connection::open(db_path).map_err(AppError::from)?;
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'app.custom_background'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(AppError::from)?;

    let Some(raw_path) = value else {
        return Ok(());
    };
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    let old_path = std::path::PathBuf::from(trimmed);
    if !old_path.starts_with(old_base) {
        return Ok(());
    }

    let Ok(relative) = old_path.strip_prefix(old_base) else {
        return Ok(());
    };
    let new_path = new_base.join(relative);

    if old_path != new_path && old_path.exists() {
        if let Some(parent) = new_path.parent() {
            std::fs::create_dir_all(parent).map_err(AppError::from)?;
        }
        if !new_path.exists() {
            if let Err(_) = std::fs::rename(&old_path, &new_path) {
                std::fs::copy(&old_path, &new_path).map_err(AppError::from)?;
                let _ = std::fs::remove_file(&old_path);
            }
        }
    }

    let new_value = new_path.to_string_lossy().to_string();
    if new_value != raw_path {
        conn.execute(
            "UPDATE settings SET value = ?1 WHERE key = 'app.custom_background'",
            params![new_value],
        )
        .map_err(AppError::from)?;
    }

    Ok(())
}

fn rewrite_content_path(
    value: &str,
    old_prefix: &str,
    new_prefix: &str,
    old_prefix_slash: &str,
    new_prefix_slash: &str,
) -> Option<String> {
    let replace_prefix = |v: &str| -> Option<String> {
        if v.starts_with(old_prefix) {
            return Some(format!("{}{}", new_prefix, &v[old_prefix.len()..]));
        }
        if v.starts_with(old_prefix_slash) {
            return Some(format!(
                "{}{}",
                new_prefix_slash,
                &v[old_prefix_slash.len()..]
            ));
        }
        None
    };

    if value.starts_with(ENCRYPT_PREFIX) {
        #[cfg(not(feature = "portable"))]
        {
            let plain = crate::database::encryption::decrypt_value(value)
                .unwrap_or_else(|| value.to_string());
            if let Some(updated_plain) = replace_prefix(&plain) {
                let encrypted = crate::database::encryption::encrypt_value(&updated_plain)
                    .unwrap_or(updated_plain);
                return Some(encrypted);
            }
        }
        return None;
    }

    replace_prefix(value)
}

fn rewrite_html_paths(
    value: &str,
    old_prefix: &str,
    new_prefix: &str,
    old_prefix_slash: &str,
    new_prefix_slash: &str,
) -> Option<String> {
    let replace_any = |v: &str| -> Option<String> {
        let mut updated = v.replace(old_prefix, new_prefix);
        updated = updated.replace(old_prefix_slash, new_prefix_slash);
        if updated == v {
            None
        } else {
            Some(updated)
        }
    };

    if value.starts_with(ENCRYPT_PREFIX) {
        #[cfg(not(feature = "portable"))]
        {
            let plain = crate::database::encryption::decrypt_value(value)
                .unwrap_or_else(|| value.to_string());
            if let Some(updated_plain) = replace_any(&plain) {
                let encrypted = crate::database::encryption::encrypt_value(&updated_plain)
                    .unwrap_or(updated_plain);
                return Some(encrypted);
            }
        }
        return None;
    }

    replace_any(value)
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    if !dst.exists() {
        std::fs::create_dir_all(dst)?;
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(all(test, target_os = "windows"))]
mod clipboard_registry_tests {
    use super::{has_original_clipboard_snapshot, owned_clipboard_value_matches};
    use winreg::types::ToRegValue;

    #[test]
    fn registry_ownership_requires_exact_value_and_presence() {
        let applied = 0u32.to_reg_value();
        let changed = 1u32.to_reg_value();
        assert!(owned_clipboard_value_matches(Some(&applied), Some(1), Some(&applied)));
        assert!(!owned_clipboard_value_matches(Some(&changed), Some(1), Some(&applied)));
        assert!(!owned_clipboard_value_matches(None, Some(1), Some(&applied)));
        assert!(owned_clipboard_value_matches(None, Some(0), None));
        assert!(!owned_clipboard_value_matches(Some(&applied), Some(0), None));
        assert!(!owned_clipboard_value_matches(None, None, None));
    }

    #[test]
    fn registry_ownership_distinguishes_string_case_and_value_type() {
        let applied = "V".to_reg_value();
        let changed = "v".to_reg_value();
        assert!(!owned_clipboard_value_matches(Some(&changed), Some(1), Some(&applied)));
        let dword = 0u32.to_reg_value();
        let text = "0".to_reg_value();
        assert!(!owned_clipboard_value_matches(Some(&text), Some(1), Some(&dword)));
    }

    #[test]
    fn original_snapshot_requires_a_value_when_marked_present() {
        let original = 0u32.to_reg_value();
        assert!(has_original_clipboard_snapshot(Some(0), None));
        assert!(has_original_clipboard_snapshot(Some(1), Some(&original)));
        assert!(!has_original_clipboard_snapshot(Some(1), None));
        assert!(!has_original_clipboard_snapshot(None, None));
        assert!(!has_original_clipboard_snapshot(Some(2), Some(&original)));
    }
}
