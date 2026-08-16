//! Tauri-compatible search-hotkey settings for the native WinUI shell.
//!
//! This adapter intentionally reads and writes one non-secret setting key. It
//! does not reuse the broad settings table as a serialization boundary, so
//! credentials stored beside the shortcut cannot cross the C ABI.

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};

const SEARCH_HOTKEY_KEY: &str = "app.search_hotkey";
const SEARCH_HOTKEY_DEFAULT: &str = "Alt+F";
const SEARCH_HOTKEY_MAX_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeSearchHotkeySnapshot {
    pub available: bool,
    pub read_only: bool,
    pub hotkey: String,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeSearchHotkeyMutation {
    #[serde(flatten)]
    pub snapshot: NativeSearchHotkeySnapshot,
    pub message: String,
}

pub struct NativeSearchHotkey {
    database_path: Option<PathBuf>,
    read_only: bool,
}

impl NativeSearchHotkey {
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

    pub fn snapshot(&self) -> Result<NativeSearchHotkeySnapshot, String> {
        let Some(database_path) = self.database_path.as_ref() else {
            return Ok(NativeSearchHotkeySnapshot {
                available: false,
                read_only: false,
                hotkey: String::new(),
                unavailable_reason: Some("搜索快捷键仅在 WinUI 生产数据模式下可用".to_owned()),
            });
        };
        let connection = open_settings_connection(database_path, self.read_only)?;
        let hotkey = connection
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [SEARCH_HOTKEY_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("无法读取搜索快捷键：{error}"))?
            .unwrap_or_else(|| SEARCH_HOTKEY_DEFAULT.to_owned());
        Ok(NativeSearchHotkeySnapshot {
            available: true,
            read_only: self.read_only,
            hotkey,
            unavailable_reason: self
                .read_only
                .then(|| "当前数据库为只读，不能修改搜索快捷键".to_owned()),
        })
    }

    pub fn update(&self, value: &str) -> Result<NativeSearchHotkeyMutation, String> {
        let database_path = self
            .database_path
            .as_ref()
            .ok_or_else(|| "搜索快捷键仅在 WinUI 生产数据模式下可用".to_owned())?;
        if self.read_only {
            return Err("当前数据库为只读，不能修改搜索快捷键".to_owned());
        }
        let value = normalize_hotkey(value)?;
        let connection = open_settings_connection(database_path, false)?;
        connection
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                (SEARCH_HOTKEY_KEY, value.as_str()),
            )
            .map_err(|error| format!("无法保存搜索快捷键：{error}"))?;
        Ok(NativeSearchHotkeyMutation {
            snapshot: self.snapshot()?,
            message: if value.is_empty() {
                "搜索快捷键已停用".to_owned()
            } else {
                "搜索快捷键已保存".to_owned()
            },
        })
    }
}

fn open_settings_connection(database_path: &Path, read_only: bool) -> Result<Connection, String> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    Connection::open_with_flags(database_path, flags)
        .map_err(|error| format!("无法打开搜索快捷键设置：{error}"))
}

fn normalize_hotkey(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.chars().count() > SEARCH_HOTKEY_MAX_CHARS {
        return Err(format!(
            "搜索快捷键不能超过 {SEARCH_HOTKEY_MAX_CHARS} 个字符"
        ));
    }
    if value.chars().any(char::is_control) {
        return Err("搜索快捷键不能包含控制字符".to_owned());
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
            "tiez-winui-search-hotkey-{label}-{}-{nonce}.db",
            std::process::id()
        ))
    }

    #[test]
    fn search_hotkey_round_trips_the_existing_key_without_credentials() {
        let path = temporary_database("round-trip");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO settings (key, value) VALUES
                    ('app.search_hotkey', 'Ctrl+Alt+F'),
                    ('mqtt_password', 'must-not-leak');",
            )
            .unwrap();
        drop(connection);

        let settings = NativeSearchHotkey::new(&path, false);
        let snapshot = settings.snapshot().unwrap();
        assert_eq!(snapshot.hotkey, "Ctrl+Alt+F");
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains("must-not-leak"));

        let mutation = settings.update("  Shift+F21  ").unwrap();
        assert_eq!(mutation.snapshot.hotkey, "Shift+F21");
        assert_eq!(
            Connection::open(&path)
                .unwrap()
                .query_row(
                    "SELECT value FROM settings WHERE key = 'app.search_hotkey'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "Shift+F21"
        );

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn search_hotkey_defaults_and_rejects_invalid_or_read_only_updates() {
        let path = temporary_database("validation");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .unwrap();
        drop(connection);

        let settings = NativeSearchHotkey::new(&path, false);
        assert_eq!(settings.snapshot().unwrap().hotkey, SEARCH_HOTKEY_DEFAULT);
        assert!(settings.update("Ctrl+\u{0007}F").is_err());
        assert!(settings.update(&"X".repeat(65)).is_err());

        let read_only = NativeSearchHotkey::new(&path, true);
        assert!(read_only.snapshot().unwrap().read_only);
        assert!(read_only.update("Ctrl+F").is_err());

        fs::remove_file(path).unwrap();
    }
}
