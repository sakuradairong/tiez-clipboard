//! Tauri-compatible sequential-paste settings and FIFO state for WinUI.
//!
//! The service owns only the two exact non-secret settings plus captured entry
//! IDs. Clipboard payloads and privacy-sensitive metadata remain in
//! `ClipboardHistory` and cross this boundary only when one queued ID is pasted.

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

const SEQUENTIAL_MODE_KEY: &str = "app.sequential_mode";
const SEQUENTIAL_HOTKEY_KEY: &str = "app.sequential_hotkey";
const SEQUENTIAL_HOTKEY_DEFAULT: &str = "Alt+V";
const SEQUENTIAL_HOTKEY_MAX_CHARS: usize = 64;
const SEQUENTIAL_QUEUE_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeSequentialPasteSnapshot {
    pub available: bool,
    pub read_only: bool,
    pub enabled: bool,
    pub hotkey: String,
    pub queued_items: usize,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeSequentialPasteMutation {
    #[serde(flatten)]
    pub snapshot: NativeSequentialPasteSnapshot,
    pub message: String,
}

#[derive(Debug)]
pub struct NativeSequentialPaste {
    database_path: Option<PathBuf>,
    read_only: bool,
    queue: VecDeque<i64>,
    last_action_was_paste: bool,
}

impl NativeSequentialPaste {
    pub fn unavailable() -> Self {
        Self {
            database_path: None,
            read_only: false,
            queue: VecDeque::new(),
            last_action_was_paste: false,
        }
    }

    pub fn new(database_path: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            database_path: Some(database_path.into()),
            read_only,
            queue: VecDeque::new(),
            last_action_was_paste: false,
        }
    }

    pub fn snapshot(&self) -> Result<NativeSequentialPasteSnapshot, String> {
        let Some(database_path) = self.database_path.as_ref() else {
            return Ok(NativeSequentialPasteSnapshot {
                available: false,
                read_only: false,
                enabled: false,
                hotkey: String::new(),
                queued_items: self.queue.len(),
                unavailable_reason: Some("顺序粘贴仅在 WinUI 生产数据模式下可用".to_owned()),
            });
        };
        let connection = open_settings_connection(database_path, self.read_only)?;
        Ok(NativeSequentialPasteSnapshot {
            available: true,
            read_only: self.read_only,
            enabled: read_text_setting(&connection, SEQUENTIAL_MODE_KEY, "false")?
                .eq_ignore_ascii_case("true"),
            hotkey: read_text_setting(
                &connection,
                SEQUENTIAL_HOTKEY_KEY,
                SEQUENTIAL_HOTKEY_DEFAULT,
            )?,
            queued_items: self.queue.len(),
            unavailable_reason: self
                .read_only
                .then(|| "当前数据库为只读，顺序粘贴已停用".to_owned()),
        })
    }

    pub fn update(
        &mut self,
        field: &str,
        raw_value: &str,
    ) -> Result<NativeSequentialPasteMutation, String> {
        let database_path = self
            .database_path
            .as_ref()
            .ok_or_else(|| "顺序粘贴仅在 WinUI 生产数据模式下可用".to_owned())?;
        if self.read_only {
            return Err("当前数据库为只读，不能修改顺序粘贴设置".to_owned());
        }
        let (key, value, message) = match field.trim().to_ascii_lowercase().as_str() {
            "hotkey" => {
                let value = normalize_hotkey(raw_value)?;
                let message = if value.is_empty() {
                    "顺序粘贴快捷键已停用".to_owned()
                } else {
                    "顺序粘贴快捷键已保存".to_owned()
                };
                (SEQUENTIAL_HOTKEY_KEY, value, message)
            }
            "enabled" => {
                let value = normalize_bool(raw_value)?;
                let enabled = value == "true";
                if !enabled {
                    self.last_action_was_paste = false;
                }
                (
                    SEQUENTIAL_MODE_KEY,
                    value,
                    if enabled {
                        "顺序粘贴模式已开启".to_owned()
                    } else {
                        "顺序粘贴模式已关闭".to_owned()
                    },
                )
            }
            _ => return Err("顺序粘贴设置字段必须是 hotkey 或 enabled".to_owned()),
        };
        let connection = open_settings_connection(database_path, false)?;
        connection
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                (key, value.as_str()),
            )
            .map_err(|error| format!("无法保存顺序粘贴设置：{error}"))?;
        Ok(NativeSequentialPasteMutation {
            snapshot: self.snapshot()?,
            message,
        })
    }

    pub fn record_capture(&mut self, entry_id: i64) -> Result<(), String> {
        if !self.snapshot()?.enabled {
            return Ok(());
        }
        if self.last_action_was_paste {
            self.queue.clear();
            self.last_action_was_paste = false;
        }
        if self.queue.len() >= SEQUENTIAL_QUEUE_LIMIT {
            self.queue.pop_front();
        }
        self.queue.push_back(entry_id);
        Ok(())
    }

    pub fn pop_next(&mut self) -> Option<i64> {
        self.queue.pop_front()
    }

    pub fn requeue_front(&mut self, entry_id: i64) {
        self.queue.push_front(entry_id);
        while self.queue.len() > SEQUENTIAL_QUEUE_LIMIT {
            self.queue.pop_back();
        }
    }

    pub fn mark_pasted(&mut self) {
        self.last_action_was_paste = true;
    }

    #[cfg(test)]
    pub fn queued_ids(&self) -> Vec<i64> {
        self.queue.iter().copied().collect()
    }
}

fn open_settings_connection(database_path: &Path, read_only: bool) -> Result<Connection, String> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    Connection::open_with_flags(database_path, flags)
        .map_err(|error| format!("无法打开顺序粘贴设置：{error}"))
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
        .map_err(|error| format!("无法读取顺序粘贴设置 {key}：{error}"))
        .map(|value| value.unwrap_or_else(|| default_value.to_owned()))
}

fn normalize_hotkey(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.chars().count() > SEQUENTIAL_HOTKEY_MAX_CHARS {
        return Err(format!(
            "顺序粘贴快捷键不能超过 {SEQUENTIAL_HOTKEY_MAX_CHARS} 个字符"
        ));
    }
    if value.chars().any(char::is_control) {
        return Err("顺序粘贴快捷键不能包含控制字符".to_owned());
    }
    Ok(value.to_owned())
}

fn normalize_bool(raw: &str) -> Result<String, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" => Ok("true".to_owned()),
        "false" => Ok("false".to_owned()),
        _ => Err("顺序粘贴模式只能设置为 true 或 false".to_owned()),
    }
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
            "tiez-winui-sequential-paste-{label}-{}-{nonce}.db",
            std::process::id()
        ))
    }

    fn writable_service(label: &str) -> (PathBuf, NativeSequentialPaste) {
        let path = temporary_database(label);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO settings (key, value) VALUES
                    ('app.sequential_mode', 'false'),
                    ('app.sequential_hotkey', 'Alt+V'),
                    ('mqtt_password', 'must-not-leak');",
            )
            .unwrap();
        drop(connection);
        let service = NativeSequentialPaste::new(&path, false);
        (path, service)
    }

    #[test]
    fn sequential_settings_use_exact_keys_without_credentials() {
        let (path, mut service) = writable_service("settings");
        let initial = service.snapshot().unwrap();
        assert!(!initial.enabled);
        assert_eq!(initial.hotkey, "Alt+V");
        assert!(!serde_json::to_string(&initial)
            .unwrap()
            .contains("must-not-leak"));

        assert!(service.update("enabled", "true").unwrap().snapshot.enabled);
        assert_eq!(
            service
                .update("hotkey", " Ctrl+F22 ")
                .unwrap()
                .snapshot
                .hotkey,
            "Ctrl+F22"
        );
        assert!(service.update("enabled", "yes").is_err());
        assert!(service.update("queue", "true").is_err());
        assert!(service.update("hotkey", &"X".repeat(65)).is_err());

        let read_only = NativeSequentialPaste::new(&path, true);
        assert!(read_only.snapshot().unwrap().read_only);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sequential_queue_is_fifo_bounded_and_resets_after_a_paste() {
        let (path, mut service) = writable_service("queue");
        service.update("enabled", "true").unwrap();
        service.record_capture(11).unwrap();
        service.record_capture(12).unwrap();
        assert_eq!(service.pop_next(), Some(11));
        service.mark_pasted();
        service.record_capture(13).unwrap();
        assert_eq!(service.queued_ids(), vec![13]);
        service.requeue_front(10);
        assert_eq!(service.queued_ids(), vec![10, 13]);

        for id in 1000..(1000 + SEQUENTIAL_QUEUE_LIMIT as i64 + 5) {
            service.record_capture(id).unwrap();
        }
        assert_eq!(
            service.snapshot().unwrap().queued_items,
            SEQUENTIAL_QUEUE_LIMIT
        );
        assert_eq!(
            service.queued_ids().last(),
            Some(&(1000 + SEQUENTIAL_QUEUE_LIMIT as i64 + 4))
        );
        fs::remove_file(path).unwrap();
    }
}
