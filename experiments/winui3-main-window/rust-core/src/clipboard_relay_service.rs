//! Native clipboard-relay adapter for the WinUI host.
//!
//! The wire protocol, authenticated encryption, replay ledger, and credential
//! names live in `tiez-core`. This adapter only binds the production SQLite
//! path and executes the async WebDAV work away from C++ ownership concerns.

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::future;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tiez_core::clipboard_relay::{
    fetch_latest_to_clipboard, send_text, RelayConfig, RelayError, RelayErrorKind,
    RelayFetchResult, RelaySendResult, SqliteRelayReceiptStore,
};
use tiez_core::cloud_sync_settings::CloudSyncSettings;
use tiez_core::cloud_sync_sqlite::ensure_cloud_sync_device_id;

const RELAY_SEND_HOTKEY_KEY: &str = "app.relay_send_hotkey";
const RELAY_FETCH_HOTKEY_KEY: &str = "app.relay_fetch_hotkey";
const RELAY_HOTKEY_MAX_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeRelaySnapshot {
    pub available: bool,
    pub read_only: bool,
    pub key_configured: bool,
    pub webdav_configured: bool,
    pub secure_transport: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeRelayKeyMutation {
    #[serde(flatten)]
    pub snapshot: NativeRelaySnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_key: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeRelayHotkeySnapshot {
    pub available: bool,
    pub read_only: bool,
    pub send_hotkey: String,
    pub fetch_hotkey: String,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeRelayHotkeyMutation {
    #[serde(flatten)]
    pub snapshot: NativeRelayHotkeySnapshot,
    pub key: String,
    pub value: String,
    pub message: String,
}

pub struct NativeClipboardRelay {
    database_path: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    read_only: bool,
}

impl NativeClipboardRelay {
    pub fn unavailable() -> Self {
        Self {
            database_path: None,
            data_dir: None,
            read_only: false,
        }
    }

    pub fn new(
        database_path: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        read_only: bool,
    ) -> Self {
        Self {
            database_path: Some(database_path.into()),
            data_dir: Some(data_dir.into()),
            read_only,
        }
    }

    pub fn snapshot(&self) -> Result<NativeRelaySnapshot, String> {
        let Some(database_path) = self.database_path.as_ref() else {
            return Ok(NativeRelaySnapshot {
                available: false,
                read_only: false,
                key_configured: false,
                webdav_configured: false,
                secure_transport: false,
                unavailable_reason: Some("剪贴板接力仅在 WinUI 生产数据模式下可用".to_owned()),
            });
        };
        let data_dir = self
            .data_dir
            .as_deref()
            .ok_or_else(|| "剪贴板接力数据目录不可用".to_owned())?;
        if let Err(error) = tiez_core::relay_key::ensure_runtime_allowed(Some(data_dir), false) {
            return Ok(NativeRelaySnapshot {
                available: false,
                read_only: self.read_only,
                key_configured: false,
                webdav_configured: false,
                secure_transport: false,
                unavailable_reason: Some(error.to_string()),
            });
        }
        let settings = CloudSyncSettings::open_sqlite(database_path, self.read_only)
            .map_err(|error| format!("无法读取 WebDAV 设置：{error}"))?;
        let cloud = settings
            .snapshot()
            .map_err(|error| format!("无法读取 WebDAV 设置：{error}"))?;
        let legacy_relay_ready = settings.relay_runner_config("relay-status").is_ok();
        Ok(NativeRelaySnapshot {
            available: true,
            read_only: self.read_only,
            key_configured: tiez_core::relay_key::is_configured()
                .map_err(|error| error.to_string())?,
            webdav_configured: !cloud.webdav_url.trim().is_empty() || legacy_relay_ready,
            secure_transport: cloud.secure_transport || legacy_relay_ready,
            unavailable_reason: self
                .read_only
                .then(|| "当前数据库为只读，不能发送、接收或修改接力密钥".to_owned()),
        })
    }

    pub fn set_key(&self, raw: &str) -> Result<NativeRelayKeyMutation, String> {
        self.ensure_writable()?;
        tiez_core::relay_key::validate_format(raw).map_err(|error| error.to_string())?;
        tiez_core::relay_key::store(raw).map_err(|error| error.to_string())?;
        Ok(NativeRelayKeyMutation {
            snapshot: self.snapshot()?,
            generated_key: None,
            message: "接力共享密钥已安全保存".to_owned(),
        })
    }

    pub fn generate_key(&self) -> Result<NativeRelayKeyMutation, String> {
        self.ensure_writable()?;
        let generated_key = tiez_core::relay_key::generate().map_err(|error| error.to_string())?;
        Ok(NativeRelayKeyMutation {
            snapshot: self.snapshot()?,
            generated_key: Some(generated_key),
            message: "已生成并安全保存新密钥；请立即复制到其他设备".to_owned(),
        })
    }

    pub fn clear_key(&self) -> Result<NativeRelayKeyMutation, String> {
        self.ensure_writable()?;
        tiez_core::relay_key::clear().map_err(|error| error.to_string())?;
        Ok(NativeRelayKeyMutation {
            snapshot: self.snapshot()?,
            generated_key: None,
            message: "接力共享密钥已清除".to_owned(),
        })
    }

    pub fn hotkey_snapshot(&self) -> Result<NativeRelayHotkeySnapshot, String> {
        let Some(database_path) = self.database_path.as_ref() else {
            return Ok(NativeRelayHotkeySnapshot {
                available: false,
                read_only: false,
                send_hotkey: String::new(),
                fetch_hotkey: String::new(),
                unavailable_reason: Some("接力快捷键仅在 WinUI 生产数据模式下可用".to_owned()),
            });
        };
        let connection = open_settings_connection(database_path, self.read_only)?;
        Ok(NativeRelayHotkeySnapshot {
            available: true,
            read_only: self.read_only,
            send_hotkey: read_hotkey(&connection, RELAY_SEND_HOTKEY_KEY)?,
            fetch_hotkey: read_hotkey(&connection, RELAY_FETCH_HOTKEY_KEY)?,
            unavailable_reason: self
                .read_only
                .then(|| "当前数据库为只读，不能修改接力快捷键".to_owned()),
        })
    }

    pub fn update_hotkey(
        &self,
        key: &str,
        value: &str,
    ) -> Result<NativeRelayHotkeyMutation, String> {
        let database_path = self
            .database_path
            .as_ref()
            .ok_or_else(|| "接力快捷键仅在 WinUI 生产数据模式下可用".to_owned())?;
        if self.read_only {
            return Err("当前数据库为只读，不能修改接力快捷键".to_owned());
        }
        if !matches!(key, RELAY_SEND_HOTKEY_KEY | RELAY_FETCH_HOTKEY_KEY) {
            return Err("不支持的接力快捷键设置".to_owned());
        }
        let value = normalize_hotkey(value)?;
        let connection = open_settings_connection(database_path, false)?;
        connection
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                (key, value.as_str()),
            )
            .map_err(|error| format!("无法保存接力快捷键：{error}"))?;
        Ok(NativeRelayHotkeyMutation {
            snapshot: self.hotkey_snapshot()?,
            key: key.to_owned(),
            value,
            message: "接力快捷键已保存".to_owned(),
        })
    }

    pub fn send(&self, text: &str) -> Result<RelaySendResult, String> {
        let config = self.config()?;
        relay_runtime()?
            .block_on(send_text(&config, text))
            .map_err(|error| error.to_string())
    }

    pub fn fetch(
        &self,
        set_clipboard: impl FnOnce(&str) -> Result<(), String>,
    ) -> Result<RelayFetchResult, String> {
        let config = self.config()?;
        let database_path = self
            .database_path
            .as_ref()
            .ok_or_else(|| "剪贴板接力仅在 WinUI 生产数据模式下可用".to_owned())?;
        let store = SqliteRelayReceiptStore::new(database_path);
        relay_runtime()?
            .block_on(fetch_latest_to_clipboard(&config, &store, move |content| {
                future::ready(
                    set_clipboard(&content)
                        .map_err(|error| RelayError::new(RelayErrorKind::Internal, error)),
                )
            }))
            .map_err(|error| error.to_string())
    }

    fn ensure_writable(&self) -> Result<(), String> {
        if self.database_path.is_none() {
            return Err("剪贴板接力仅在 WinUI 生产数据模式下可用".to_owned());
        }
        if self.read_only {
            return Err("当前数据库为只读，不能修改剪贴板接力".to_owned());
        }
        let data_dir = self
            .data_dir
            .as_deref()
            .ok_or_else(|| "剪贴板接力数据目录不可用".to_owned())?;
        tiez_core::relay_key::ensure_runtime_allowed(Some(data_dir), false)
            .map_err(|error| error.to_string())
    }

    fn config(&self) -> Result<RelayConfig, String> {
        self.ensure_writable()?;
        let database_path = self.database_path.as_ref().expect("checked above");
        let device_id = ensure_cloud_sync_device_id(database_path)
            .map_err(|error| format!("无法创建接力设备标识：{error}"))?;
        let runner = CloudSyncSettings::open_sqlite(database_path, false)
            .and_then(|settings| settings.relay_runner_config(device_id))
            .map_err(|error| format!("WebDAV 设置无效：{error}"))?;
        let shared_key = tiez_core::relay_key::load()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "请先配置剪贴板接力共享密钥".to_owned())?;
        RelayConfig::new(
            runner.webdav_url,
            runner.webdav_username,
            runner.webdav_password,
            runner.webdav_base_path,
            runner.device_id,
            shared_key,
        )
        .map_err(|error| error.to_string())
    }
}

fn open_settings_connection(database_path: &Path, read_only: bool) -> Result<Connection, String> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    Connection::open_with_flags(database_path, flags)
        .map_err(|error| format!("无法打开接力快捷键设置：{error}"))
}

fn read_hotkey(connection: &Connection, key: &str) -> Result<String, String> {
    connection
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(String::new()),
            other => Err(other),
        })
        .map_err(|error| format!("无法读取接力快捷键：{error}"))
}

fn normalize_hotkey(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.chars().count() > RELAY_HOTKEY_MAX_CHARS {
        return Err(format!(
            "接力快捷键不能超过 {RELAY_HOTKEY_MAX_CHARS} 个字符"
        ));
    }
    if value.chars().any(char::is_control) {
        return Err("接力快捷键不能包含控制字符".to_owned());
    }
    Ok(value.to_owned())
}

fn relay_runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .thread_name("tiez-winui-relay")
                .build()
                .map_err(|error| format!("无法启动剪贴板接力运行时：{error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
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
            "tiez-winui-relay-hotkey-{label}-{}-{nonce}.db",
            std::process::id()
        ))
    }

    #[test]
    fn relay_hotkeys_round_trip_existing_tauri_keys_without_credentials() {
        let path = temporary_database("round-trip");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO settings (key, value) VALUES
                    ('app.relay_send_hotkey', 'Ctrl+Alt+S'),
                    ('app.relay_fetch_hotkey', 'Ctrl+Alt+F'),
                    ('mqtt_password', 'must-not-leak');",
            )
            .unwrap();
        drop(connection);

        let relay = NativeClipboardRelay::new(&path, path.parent().unwrap(), false);
        let snapshot = relay.hotkey_snapshot().unwrap();
        assert_eq!(snapshot.send_hotkey, "Ctrl+Alt+S");
        assert_eq!(snapshot.fetch_hotkey, "Ctrl+Alt+F");
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("must-not-leak"));

        let mutation = relay
            .update_hotkey(RELAY_SEND_HOTKEY_KEY, "  Alt+Shift+S  ")
            .unwrap();
        assert_eq!(mutation.value, "Alt+Shift+S");
        assert_eq!(mutation.snapshot.send_hotkey, "Alt+Shift+S");
        assert_eq!(mutation.snapshot.fetch_hotkey, "Ctrl+Alt+F");

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn relay_hotkeys_reject_unknown_keys_controls_and_read_only_updates() {
        let path = temporary_database("validation");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .unwrap();
        drop(connection);

        let relay = NativeClipboardRelay::new(&path, path.parent().unwrap(), false);
        assert!(relay.update_hotkey("mqtt_password", "secret").is_err());
        assert!(relay
            .update_hotkey(RELAY_SEND_HOTKEY_KEY, "Ctrl+S\n")
            .is_ok());
        assert!(relay
            .update_hotkey(RELAY_FETCH_HOTKEY_KEY, "Ctrl+\u{0007}F")
            .is_err());

        let read_only = NativeClipboardRelay::new(&path, path.parent().unwrap(), true);
        assert!(read_only.hotkey_snapshot().unwrap().read_only);
        assert!(read_only
            .update_hotkey(RELAY_SEND_HOTKEY_KEY, "Ctrl+S")
            .is_err());

        fs::remove_file(path).unwrap();
    }
}
