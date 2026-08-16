//! Tauri-compatible latest-item paste hotkeys for the native WinUI shell.
//!
//! Only the two non-secret shortcut keys and the delete-after-paste flag cross
//! this adapter. Other settings, including credentials stored in the same
//! table, never enter the C ABI response.

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};

const RICH_PASTE_HOTKEY_KEY: &str = "app.rich_paste_hotkey";
const RICH_PASTE_HOTKEY_DEFAULT: &str = "Alt+Shift+V";
const PLAIN_PASTE_HOTKEY_KEY: &str = "app.plain_paste_hotkey";
const PLAIN_PASTE_HOTKEY_DEFAULT: &str = "";
const DELETE_AFTER_PASTE_KEY: &str = "app.delete_after_paste";
const PASTE_HOTKEY_MAX_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativePasteHotkeySnapshot {
    pub available: bool,
    pub read_only: bool,
    pub rich_hotkey: String,
    pub plain_hotkey: String,
    pub delete_after_paste: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativePasteHotkeyMutation {
    #[serde(flatten)]
    pub snapshot: NativePasteHotkeySnapshot,
    pub message: String,
}

pub struct NativePasteHotkeys {
    database_path: Option<PathBuf>,
    read_only: bool,
}

impl NativePasteHotkeys {
    pub fn unavailable() -> Self {
        Self {
            database_path: None,
            read_only: false,
        }
    }

    pub fn new(database_path: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            database_path: Some(database_path.into()),
            read_only,
        }
    }

    pub fn snapshot(&self) -> Result<NativePasteHotkeySnapshot, String> {
        let Some(database_path) = self.database_path.as_ref() else {
            return Ok(NativePasteHotkeySnapshot {
                available: false,
                read_only: false,
                rich_hotkey: String::new(),
                plain_hotkey: String::new(),
                delete_after_paste: false,
                unavailable_reason: Some("粘贴快捷键仅在 WinUI 生产数据模式下可用".to_owned()),
            });
        };
        let connection = open_settings_connection(database_path, self.read_only)?;
        Ok(NativePasteHotkeySnapshot {
            available: true,
            read_only: self.read_only,
            rich_hotkey: read_text_setting(
                &connection,
                RICH_PASTE_HOTKEY_KEY,
                RICH_PASTE_HOTKEY_DEFAULT,
            )?,
            plain_hotkey: read_text_setting(
                &connection,
                PLAIN_PASTE_HOTKEY_KEY,
                PLAIN_PASTE_HOTKEY_DEFAULT,
            )?,
            delete_after_paste: read_text_setting(&connection, DELETE_AFTER_PASTE_KEY, "false")?
                .eq_ignore_ascii_case("true"),
            unavailable_reason: self
                .read_only
                .then(|| "当前数据库为只读，不能修改粘贴快捷键".to_owned()),
        })
    }

    pub fn update(&self, kind: &str, value: &str) -> Result<NativePasteHotkeyMutation, String> {
        let database_path = self
            .database_path
            .as_ref()
            .ok_or_else(|| "粘贴快捷键仅在 WinUI 生产数据模式下可用".to_owned())?;
        if self.read_only {
            return Err("当前数据库为只读，不能修改粘贴快捷键".to_owned());
        }
        let (key, label) = paste_hotkey_definition(kind)?;
        let value = normalize_hotkey(value)?;
        let connection = open_settings_connection(database_path, false)?;
        connection
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                (key, value.as_str()),
            )
            .map_err(|error| format!("无法保存{label}粘贴快捷键：{error}"))?;
        Ok(NativePasteHotkeyMutation {
            snapshot: self.snapshot()?,
            message: if value.is_empty() {
                format!("{label}粘贴快捷键已停用")
            } else {
                format!("{label}粘贴快捷键已保存")
            },
        })
    }

    pub fn delete_after_paste(&self) -> Result<bool, String> {
        let Some(database_path) = self.database_path.as_ref() else {
            return Ok(false);
        };
        let connection = open_settings_connection(database_path, self.read_only)?;
        Ok(
            read_text_setting(&connection, DELETE_AFTER_PASTE_KEY, "false")?
                .eq_ignore_ascii_case("true"),
        )
    }
}

fn paste_hotkey_definition(kind: &str) -> Result<(&'static str, &'static str), String> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "rich" => Ok((RICH_PASTE_HOTKEY_KEY, "富文本")),
        "plain" => Ok((PLAIN_PASTE_HOTKEY_KEY, "纯文本")),
        _ => Err("粘贴快捷键类型必须是 rich 或 plain".to_owned()),
    }
}

fn open_settings_connection(database_path: &Path, read_only: bool) -> Result<Connection, String> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    Connection::open_with_flags(database_path, flags)
        .map_err(|error| format!("无法打开粘贴快捷键设置：{error}"))
}

fn read_text_setting(
    connection: &Connection,
    key: &str,
    default_value: &str,
) -> Result<String, String> {
    connection
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .map_err(|error| format!("无法读取粘贴快捷键设置 {key}：{error}"))
        .map(|value| value.unwrap_or_else(|| default_value.to_owned()))
}

fn normalize_hotkey(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.chars().count() > PASTE_HOTKEY_MAX_CHARS {
        return Err(format!(
            "粘贴快捷键不能超过 {PASTE_HOTKEY_MAX_CHARS} 个字符"
        ));
    }
    if value.chars().any(char::is_control) {
        return Err("粘贴快捷键不能包含控制字符".to_owned());
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_database(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "tiez-winui-paste-hotkey-{label}-{}-{nonce}.db",
            std::process::id()
        ))
    }

    #[test]
    fn paste_hotkeys_round_trip_exact_keys_without_credentials() {
        let path = temporary_database("round-trip");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO settings (key, value) VALUES
                    ('app.rich_paste_hotkey', 'Ctrl+Alt+R'),
                    ('app.plain_paste_hotkey', 'Ctrl+Alt+P'),
                    ('app.delete_after_paste', 'true'),
                    ('mqtt_password', 'must-not-leak');",
            )
            .unwrap();
        drop(connection);

        let settings = NativePasteHotkeys::new(&path, false);
        let snapshot = settings.snapshot().unwrap();
        assert_eq!(snapshot.rich_hotkey, "Ctrl+Alt+R");
        assert_eq!(snapshot.plain_hotkey, "Ctrl+Alt+P");
        assert!(snapshot.delete_after_paste);
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains("must-not-leak"));

        let mutation = settings.update("plain", "  Shift+F20  ").unwrap();
        assert_eq!(mutation.snapshot.plain_hotkey, "Shift+F20");
        assert_eq!(
            Connection::open(&path)
                .unwrap()
                .query_row(
                    "SELECT value FROM settings WHERE key = 'app.plain_paste_hotkey'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "Shift+F20"
        );

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn paste_hotkeys_default_validate_kinds_and_reject_read_only_updates() {
        let path = temporary_database("validation");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .unwrap();
        drop(connection);

        let settings = NativePasteHotkeys::new(&path, false);
        let snapshot = settings.snapshot().unwrap();
        assert_eq!(snapshot.rich_hotkey, RICH_PASTE_HOTKEY_DEFAULT);
        assert_eq!(snapshot.plain_hotkey, PLAIN_PASTE_HOTKEY_DEFAULT);
        assert!(settings.update("sequential", "Alt+V").is_err());
        assert!(settings.update("plain", "Ctrl+\u{0007}V").is_err());
        assert!(settings.update("plain", &"X".repeat(65)).is_err());

        let read_only = NativePasteHotkeys::new(&path, true);
        assert!(read_only.snapshot().unwrap().read_only);
        assert!(read_only.update("rich", "Ctrl+F20").is_err());

        fs::remove_file(path).unwrap();
    }
}
